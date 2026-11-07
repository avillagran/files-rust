#!/usr/bin/env python3
"""Compare Linux private Wayland Sheet captures, excluding variable text."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import stat
import struct
import subprocess
import time


def absolute_path(value):
    """Normalize spelling without resolving any filesystem symlink."""
    path = Path(value)
    if '..' in path.parts:
        raise ValueError('parent traversal is not allowed in capture paths')
    return path if path.is_absolute() else Path.cwd() / path


def parent_fd(path):
    """Open every parent component without following symlinks."""
    directory = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parent.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                            dir_fd=directory)
            os.close(directory)
            directory = child
        result, directory = directory, None
        return result
    finally:
        if directory is not None:
            os.close(directory)


class CaptureRoot:
    """Pin each capture directory for the complete comparison transaction."""
    def __init__(self, path):
        self.path = path
        self.descriptor = parent_fd(path / '.boundary')
        metadata = os.fstat(self.descriptor)
        if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) & 0o022:
            self.close()
            raise ValueError('capture root must be owned by this user and not writable by other users')

    def close(self):
        if self.descriptor is not None:
            os.close(self.descriptor)
            self.descriptor = None

    def relative(self, path):
        relative = path.relative_to(self.path)
        if relative == Path('.') or '..' in relative.parts:
            raise ValueError('capture path must be a strict child of its pinned root')
        return relative

    def read_regular(self, path, limit):
        relative = self.relative(path)
        directory = os.dup(self.descriptor)
        try:
            for part in relative.parent.parts:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=directory)
                metadata = os.fstat(child)
                if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) & 0o022:
                    os.close(child)
                    raise ValueError('capture subdirectory is writable by another user')
                os.close(directory)
                directory = child
            descriptor = os.open(relative.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                                 dir_fd=directory)
            with os.fdopen(descriptor, 'rb') as source:
                metadata = os.fstat(source.fileno())
                if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
                        or stat.S_IMODE(metadata.st_mode) & 0o022
                        or metadata.st_size > limit):
                    raise ValueError('capture input must be a bounded regular file owned by this user')
                data = source.read(limit + 1)
                if len(data) > limit:
                    raise ValueError('capture input grew beyond its byte limit')
                return data
        finally:
            os.close(directory)

    def write_new(self, name, data):
        """Never replace an existing evidence leaf, including a symlink."""
        if name != Path(name).name or name in ('', '.', '..'):
            raise ValueError('comparison output must be a direct child of its pinned root')
        descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                             0o600, dir_fd=self.descriptor)
        with os.fdopen(descriptor, 'wb') as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())


def decode_rgb(encoded, width, height, x, y):
    """Bound decoder output in anonymous files instead of an unbounded pipe."""
    expected = width * height * 3
    cap = max(expected, 64 * 1024)
    output = os.memfd_create('sheet-compare-rgb', os.MFD_CLOEXEC)
    errors = None
    try:
        errors = os.memfd_create('sheet-compare-errors', os.MFD_CLOEXEC)
        def child_limits():
            for kind, wanted in ((resource.RLIMIT_FSIZE, cap),
                                 (resource.RLIMIT_AS, 1024 * 1024 * 1024),
                                 (resource.RLIMIT_CPU, 10)):
                _, hard = resource.getrlimit(kind)
                maximum = wanted if hard == resource.RLIM_INFINITY else min(wanted, hard)
                resource.setrlimit(kind, (maximum, maximum))
        command = ['magick', '-limit', 'memory', '128MiB', '-limit', 'map', '0',
                   '-limit', 'disk', '0', '-limit', 'thread', '2', 'png:-',
                   '-crop', f'{width}x{height}+{x}+{y}', '+repage', '-alpha', 'off',
                   '-depth', '8', 'rgb:-']
        try:
            result = subprocess.run(command, input=encoded, stdout=output, stderr=errors,
                                    env={**os.environ, 'MAGICK_OCL_DEVICE': 'OFF', 'OMP_NUM_THREADS': '2'},
                                    preexec_fn=child_limits, timeout=15, check=False)
        except subprocess.TimeoutExpired as exc:
            os.lseek(errors, 0, os.SEEK_SET)
            detail = os.read(errors, 8192).decode('utf-8', errors='replace')
            raise ValueError('PNG decoding timed out: ' + detail) from exc
        if result.returncode != 0 or os.fstat(output).st_size != expected:
            os.lseek(errors, 0, os.SEEK_SET)
            detail = os.read(errors, 8192).decode('utf-8', errors='replace')
            raise ValueError(f'PNG decoding failed (exit {result.returncode}): {detail}')
        os.lseek(output, 0, os.SEEK_SET)
        with os.fdopen(os.dup(output), 'rb') as source:
            return source.read(expected)
    finally:
        if errors is not None:
            os.close(errors)
        os.close(output)


def main():
    if not __debug__:
        raise SystemExit('Evidence checks require Python assertions; do not use optimized mode')
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('on', type=Path)
    ap.add_argument('off', type=Path)
    args = ap.parse_args()
    out = None
    boundaries = []
    output_root = None
    report = {'functional_gate': 'FAIL', 'on_report': str(args.on), 'off_report': str(args.off),
              'method': 'RGB pixel comparison restricted to Sheet panel; directory and status label rectangles masked',
              'cases': {}}
    try:
        on_path, off_path = [absolute_path(p) for p in (args.on, args.off)]
        on_root = CaptureRoot(on_path.parent)
        boundaries.append(on_root)
        output_root = on_root
        out = on_path.parent / ('motion-comparison-' + str(time.time_ns()) + '.json')
        off_root = CaptureRoot(off_path.parent)
        boundaries.append(off_root)
        report.update(on_report=str(on_path), off_report=str(off_path))
        on, off = [json.loads(boundary.read_regular(p, 16 * 1024 * 1024))
                   for boundary, p in ((on_root, on_path), (off_root, off_path))]
        roots = {}
        for value, source, boundary in ((on, on_path, on_root), (off, off_path, off_root)):
            fixture = absolute_path(value['fixture_dir'])
            if fixture != source.parent:
                raise ValueError('report must belong to its declared fixture directory')
            roots[id(value)] = boundary
        assert on['functional_gate'] == off['functional_gate'] == 'PASS'
        assert on['animations'] == 'on' and off['animations'] == 'off'
        for key in ('binary_sha256', 'helper_sha256', 'html_helper_sha256', 'window_size'):
            assert on[key] == off[key], key
        window = on['window_size']
        if (not isinstance(window, list) or len(window) != 2
                or any(type(v) is not int or v <= 0 or v > 4096 for v in window)
                or window[0] * window[1] > 4 * 1024 * 1024):
            raise ValueError('mapped window exceeds the capture pixel budget')
        def snapshot(r):
            return next(s for s in r['states'] if s['label'] == 'create-file' and s.get('open') and s.get('built'))
        first, second = snapshot(on), snapshot(off)
        assert first['rect'] == second['rect']
        for key in ('fields', 'apply', 'cancel', 'scroll', 'directory', 'status_rect', 'values', 'control_count',
                    'status', 'apply_enabled', 'field_enabled'):
            assert first['geometry'][key] == second['geometry'][key], key
        for key in ('can_apply', 'can_edit', 'error', 'dismissible'):
            assert first[key] == second[key], key
        rect = first['rect']; x, y, width, height = rect
        assert min(x, y) >= 0 and 0 < width <= 4096 and 0 < height <= 4096
        assert x + width <= on['window_size'][0] and y + height <= on['window_size'][1]
        masks = [first['geometry']['directory'], first['geometry']['status_rect']]
        assert all(len(m) == 4 and m[2] > 0 and m[3] > 0 for m in masks)
        report['panel_rect'] = rect
        report['excluded_rectangles'] = [{'reason': reason, 'rect': mask} for reason, mask in
                                         zip(('fixture-specific directory text', 'variable status text'), masks)]
        def capture(r, label):
            matches = [absolute_path(p) for p in r['captures'] if Path(p).name == label + '.png']
            if len(matches) != 1:
                raise ValueError('each capture label must identify exactly one image')
            path = matches[0]
            roots[id(r)].relative(path)
            return roots[id(r)], path
        images = {}
        def pixels(boundary, path):
            key = (id(boundary), path)
            if key in images:
                return images[key]
            encoded = boundary.read_regular(path, 32 * 1024 * 1024)
            if not encoded.startswith(b'\x89PNG\r\n\x1a\n'):
                raise ValueError('capture must be a PNG image')
            if len(encoded) < 24 or struct.unpack('>II', encoded[16:24]) != tuple(on['window_size']):
                raise ValueError('PNG dimensions differ from the mapped window')
            # Decode the bytes already checked, rather than reopening a path.
            output = decode_rgb(encoded, width, height, x, y)
            assert len(output) == width * height * 3
            data = bytearray(output)
            for mx, my, mw, mh in masks:
                left, right = max(x, mx), min(x + width, mx + mw)
                top, bottom = max(y, my), min(y + height, my + mh)
                if left >= right or top >= bottom:
                    continue
                for row in range(top, bottom):
                    a = ((row - y) * width + left - x) * 3
                    b = ((row - y) * width + right - x) * 3
                    data[a:b] = b'\0' * (b - a)
            images[key] = (data, hashlib.sha256(encoded).hexdigest())
            return images[key]
        cases = [('on_open_start_middle', on, 'open-start', on, 'open-middle', True),
                 ('on_open_middle_settled', on, 'open-middle', on, 'open-settled', True),
                 ('off_open_start_settled', off, 'open-start', off, 'open-settled', False),
                 ('on_off_open_settled', on, 'open-settled', off, 'open-settled', False),
                 ('on_cover_to_return_middle', on, 'cover-middle', on, 'cover-return-middle', True),
                 ('on_return_middle_settled', on, 'cover-return-middle', on, 'cover-return-settled', True)]
        for label, ar, an, br, bn, changed in cases:
            (a_root, a), (b_root, b) = capture(ar, an), capture(br, bn)
            report['active_case'] = {'label': label, 'images': [str(a), str(b)]}
            (pa, ha), (pb, hb) = pixels(a_root, a), pixels(b_root, b)
            count = sum(pa[i:i+3] != pb[i:i+3] for i in range(0, len(pa), 3))
            report['cases'][label] = {'changed_pixels': count, 'images': [str(a), str(b)],
                                      'sha256': [ha, hb]}
            assert (count > 0) == changed, label
        report.pop('active_case', None)
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        report['error'] = repr(exc)
    attempted_report = str(out) if out is not None else None
    if out is not None:
        try:
            output_root.write_new(out.name, (json.dumps(report, indent=2) + '\n').encode())
        except Exception as exc:
            report.update(functional_gate='FAIL', error='Cannot preserve comparison report: ' + repr(exc))
            out = None
    for boundary in boundaries:
        boundary.close()
    print(json.dumps({'functional_gate': report['functional_gate'], 'report': str(out) if out else None,
                      'attempted_report': attempted_report,
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
