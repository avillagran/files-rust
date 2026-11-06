#!/usr/bin/env python3
"""Check Sidebar and browser motion through the app's private fixture handler.

Run under Xvfb or scripts/nested-run.py. The app advances the toolkit's
frozen animation clock, renders offscreen frames, and checks their geometry.
This checks app handlers and rendered layout, not native pointer delivery.
HOME, config, data and the directory listing belong to a private fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'docs/evidence/0.0.4'
MARKER = 'FILES_SIDEBAR_MOTION_REPORT '


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    identity = str(time.time_ns())
    report_path = OUT / f'sidebar-motion-ui-{identity}.json'
    saved_log = OUT / f'sidebar-motion-ui-{identity}.txt'
    shots = OUT / f'sidebar-motion-{identity}-shots'
    shots.mkdir()
    report = {
        'functional_gate': 'FAIL',
        'resource_gate': 'NOT_MEASURED',
        'startup_gate': 'NOT_MEASURED',
        'method': 'Private Files fixture handler, frozen toolkit clock, offscreen rendered frame geometry; no native input',
        'binary_path': str(binary),
        'log_path': str(saved_log),
        'viewport': [1000, 650],
        'captures': [],
    }
    process = None
    with tempfile.TemporaryDirectory(prefix='sidebar-motion-', dir=base_root) as tmp:
        base = Path(tmp)
        log = base / 'app.log'
        raw = base / 'frame.ppm'
        try:
            home = base / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
                (home / name).mkdir(parents=True)
            listing = home / 'fixture'
            listing.mkdir()
            fixture = listing / 'example.txt'
            fixture.write_bytes(b'Sidebar motion fixture\n')
            original_hash = digest(fixture)
            data = base / 'data'
            data.mkdir()
            config = base / 'config'
            config.mkdir()
            devices = base / 'devices.json'
            devices.write_text('{"blockdevices": []}\n')
            shell = base / 'shell.toml'
            shell.write_text('[font]\nbase-size = 14\n')
            for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                              ('files-html-helper', 'html_helper_sha256')]:
                artifact = binary if name == 'files' else binary.parent / name
                if artifact.is_file():
                    report[key] = digest(artifact)
            env = {
                **os.environ,
                'HOME': str(home),
                'XDG_CONFIG_HOME': str(config),
                'XDG_DATA_HOME': str(data),
                'FILES_CONFIG_DIR': str(base / 'files-config'),
                'FILES_TEST_DEVICES_JSON': str(devices),
                'FILES_TEST_CAPTURE': str(raw),
                'FILES_TEST_FIXED_SIZE': '1',
                'FILES_TEST_ISOLATE_INPUT': '1',
                'FILES_TEST_TOOLKIT_POPUP': '1',
                'FILES_TEST_MOTION_SHOTS': str(shots),
                'FILES_CLOSE_TRACE': '1',
                'OMARCHY_FLTK_ANIM': 'on',
                'OMARCHY_FLTK_USER_SHELL': str(shell),
                'TMPDIR': str(base),
                'MAGICK_TEMPORARY_PATH': str(base),
            }
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(listing), '--size', '1000x650', '--scale', '1',
                     '--no-session', '--ui-smoke', '--exit-after', '30'],
                    env=env, stdout=subprocess.DEVNULL, stderr=stream, start_new_session=True)

            def wait(check, reason, timeout=20):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = check()
                    if value:
                        return value
                    if process.poll() is not None:
                        raise RuntimeError(f'{reason}: app exit {process.returncode}')
                    time.sleep(.02)
                raise RuntimeError(f'{reason}: timed out')

            def text():
                return log.read_text(errors='replace')

            def send(action):
                pending = raw.with_suffix('.pending')
                pending.write_text(action)
                pending.replace(raw.with_suffix('.popup-command'))

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
            send('sidebar-motion-check')

            def motion_report():
                return next((line[len(MARKER):] for line in text().split('\n')[:-1]
                             if line.startswith(MARKER)), None)

            report['motion'] = json.loads(wait(motion_report, 'Sidebar motion report'))
            captures = report['motion'].get('captures', [])
            assert isinstance(captures, list) and len(captures) == 3, captures
            for value in captures:
                path = Path(value)
                assert path.is_absolute() and path.parent == shots and path.is_file(), value
            report['captures'] = captures
            assert report['motion'].get('ok') is True, report['motion']
            assert digest(fixture) == original_hash, 'fixture contents changed'
            report['fixture_intact'] = True
            send('close')
            process.wait(timeout=15)
            assert process.returncode == 0, f'app exit {process.returncode}'
            assert 'FILES_CLOSE_DRAINED' in text(), 'normal close did not report drained workers'
            report.update(functional_gate='PASS', normal_exit=True)
        except Exception as error:
            report['error'] = repr(error)
        finally:
            if process is not None and process.poll() is None:
                stop_tree(process, None)
            report['captures'] = sorted(str(path) for path in shots.iterdir() if path.is_file())
            report['exit_code'] = process.returncode if process is not None else None
            saved_log.write_text(log.read_text(errors='replace') if log.exists() else '')
            with report_path.open('x') as stream:
                json.dump(report, stream, indent=2)
                stream.write('\n')
    print(json.dumps({'report': str(report_path), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
