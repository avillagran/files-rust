#!/usr/bin/env python3
"""Install Files desktop identity without selecting default associations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--prefix', type=Path, default=Path.home() / '.local')
parser.add_argument('--receipt', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
prefix = args.prefix.absolute()

def directory(path):
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current = current / part
        try:
            current.mkdir(mode=0o700)
        except FileExistsError:
            pass
        info = current.lstat()
        if not stat.S_ISDIR(info.st_mode):
            raise RuntimeError('Desktop identity directory is not a real directory')

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def copy(source, target):
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, 'wb') as out:
        out.write(source.read_bytes())
        out.flush()
        os.fsync(out.fileno())

directory(prefix)
transaction = prefix / 'share/files/identity-backups' / str(time.time_ns())
directory(transaction)
records = []
staged = []
replaced = []
for source, relative in [
    (root / 'assets/files.svg', 'share/icons/hicolor/scalable/apps/files.svg'),
    (root / 'packaging/files.desktop', 'share/applications/files.desktop'),
]:
    target = prefix / relative
    directory(target.parent)
    old = None
    try:
        info = target.lstat()
    except FileNotFoundError:
        pass
    else:
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise RuntimeError('Desktop identity destination must be an owned regular file')
        old = sha(target)
        copy(target, transaction / target.name)
        if sha(transaction / target.name) != old:
            raise RuntimeError('Desktop identity backup hash mismatch')
    stage = target.with_name('.' + target.name + '.' + transaction.name)
    copy(source, stage)
    expected = sha(source)
    if sha(stage) != expected:
        raise RuntimeError('Desktop identity staging hash mismatch')
    records.append(dict(path=str(target), old_sha256=old, sha256=expected))
    staged.append((stage, target))
for record, (stage, target) in zip(records, staged):
    if (sha(target) if target.exists() else None) != record['old_sha256']:
        raise RuntimeError('Concurrent desktop identity installation')
    os.replace(stage, target)
    replaced.append(str(target))
    if sha(target) != record['sha256']:
        raise RuntimeError('Installed desktop identity hash mismatch')
with args.receipt.open('x') as out:
    json.dump(dict(files=records, backup=str(transaction),
                   associations='handler entry only; defaults untouched'), out, indent=2)
