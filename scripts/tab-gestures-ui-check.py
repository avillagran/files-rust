#!/usr/bin/env python3
"""Check middle-button tab gestures with private rendered Files fixtures.

Run under Xvfb or scripts/nested-run.py. Fixture commands call the shared
auxiliary-pointer handler on painted hits. This checks app behavior and
framebuffers, not native OS pointer delivery. No personal files or settings
are used. --binary also validates an installed artifact.
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
STATE = 'FILES_TAB_GESTURE_STATE '


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
    report_path = OUT / f'tab-gestures-ui-{identity}.json'
    saved_log = OUT / f'tab-gestures-ui-{identity}.txt'
    report = {
        'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED',
        'startup_gate': 'NOT_MEASURED', 'binary_path': str(binary),
        'method': 'Private fixture commands invoke auxiliary-pointer handling on painted hits; framebuffer captures; no native input',
        'log_path': str(saved_log), 'states': [], 'checks': [], 'captures': [],
    }
    process = None
    with tempfile.TemporaryDirectory(prefix='tab-gestures-', dir=base_root) as tmp:
        base = Path(tmp)
        log = base / 'app.log'
        raw = base / 'frame.ppm'
        try:
            home = base / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
                (home / name).mkdir(parents=True)
            listing = home / 'fixture'
            folder = listing / 'folder'
            folder.mkdir(parents=True)
            fixture = listing / 'file.txt'
            fixture.write_bytes(b'Tab gesture fixture\n')
            child_file = folder / 'child.txt'
            child_file.write_bytes(b'Background folder fixture\n')
            originals = {p: digest(p) for p in [fixture, child_file]}
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
                **os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(config),
                'XDG_DATA_HOME': str(data), 'FILES_CONFIG_DIR': str(base / 'files-config'),
                'FILES_TEST_DEVICES_JSON': str(devices), 'FILES_TEST_CAPTURE': str(raw),
                'FILES_TEST_FIXED_SIZE': '1', 'FILES_TEST_ISOLATE_INPUT': '1',
                'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
                'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_ANIM': 'off',
                'OMARCHY_FLTK_USER_SHELL': str(shell), 'TMPDIR': str(base),
                'MAGICK_TEMPORARY_PATH': str(base),
            }
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(listing), '--size', '1000x650', '--scale', '1',
                     '--no-session', '--ui-smoke', '--exit-after', '60'],
                    env=env, stdout=subprocess.DEVNULL, stderr=stream, start_new_session=True)

            def text():
                return log.read_text(errors='replace')

            def wait(check, reason, timeout=15):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = check()
                    if value:
                        return value
                    if process.poll() is not None:
                        raise RuntimeError(f'{reason}: app exit {process.returncode}')
                    time.sleep(.02)
                raise RuntimeError(f'{reason}: timed out')

            def send(action):
                before = len(text())
                pending = raw.with_suffix('.pending')
                pending.write_text(action)
                pending.replace(raw.with_suffix('.popup-command'))
                marker = f'FILES_TOOLKIT_FIXTURE action={action}'
                wait(lambda: marker in text()[before:].split('\n')[:-1], action)
                return before

            def state(label):
                before = send('tab-gesture-state')

                def ready():
                    for line in text()[before:].split('\n')[:-1]:
                        if line.startswith(STATE):
                            return json.loads(line[len(STATE):])
                    return None

                value = wait(ready, f'{label}: fresh state')
                assert not value['closing'], value
                report['states'].append({'label': label, **value})
                return value

            def act(target):
                send('tab-gesture:' + target)
                return state(target)

            def ready_target(target, other=False):
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    value = state('ready-' + target)
                    loaded = value.get('other_loaded' if other else 'loaded', False)
                    settled = value.get('settled') is True and value.get('refreshing') is False
                    if loaded and settled and value.get('targets', {}).get(target):
                        return value
                    time.sleep(.03)
                raise RuntimeError(f'{target}: painted target was not ready')

            def retained(before, after):
                keys = ['path', 'active', 'selected_raw', 'selection_raw', 'anchor_raw',
                        'generation', 'pane_id']
                assert {k: before[k] for k in keys} == {k: after[k] for k in keys}, (before, after)

            def capture(label):
                raw.unlink(missing_ok=True)
                raw.with_suffix('.request').touch()

                def ready():
                    if not raw.exists():
                        return False
                    parts = raw.read_bytes().split(b'\n', 3)
                    return (len(parts) == 4 and parts[1] == b'1000 650'
                            and len(parts[3]) == 1000 * 650 * 3)

                wait(ready, f'{label}: framebuffer')
                target = OUT / f'tab-gestures-{label}-{identity}.png'
                subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=10)
                report['captures'].append(str(target))

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
            ready_target('file')
            act('select-file')
            initial = ready_target('folder')
            report['states'].append({'label': 'initial', **initial})
            assert initial['path'] == str(listing), initial
            assert initial['selected_raw'] == b'file.txt'.hex(), initial
            assert initial['selection_raw'], initial
            assert len(initial['tabs']) == 1, initial
            capture('selected-file')

            opened = act('folder')
            retained(initial, opened)
            assert len(opened['tabs']) == 2 and opened['tabs'][-1]['path'] == str(folder), opened
            first_background_id = opened['tabs'][-1]['id']
            report['checks'].append('folder opens metadata-only tab while active path/selection/anchor/generation remain unchanged')
            capture('background-folder')

            ignored = act('file')
            retained(opened, ignored)
            assert ignored['tabs'] == opened['tabs'], ignored
            no_new = act('tab-new-button')
            retained(ignored, no_new)
            assert no_new['tabs'] == ignored['tabs'], no_new
            blocked = act('blocked-folder')
            retained(no_new, blocked)
            assert blocked['tabs'] == no_new['tabs'], blocked
            report['checks'].append('middle file/new-tab-button/covered folder do not run primary actions')

            closed = act('tab-close-button-last')
            retained(initial, closed)
            assert len(closed['tabs']) == 1 and closed['closed'] == initial['closed'] + 1, closed
            reopened = act('folder')
            retained(initial, reopened)
            assert reopened['tabs'][-1]['id'] > first_background_id, reopened
            body_closed = act('tab-last')
            retained(initial, body_closed)
            assert len(body_closed['tabs']) == 1 and body_closed['closed'] == initial['closed'] + 2, body_closed
            report['checks'].append('middle tab body and close-button close only that stable tab ID')

            for view in ['list', 'grid', 'columns']:
                act('view-' + view)
                before = ready_target('folder')
                after = act('folder')
                retained(before, after)
                assert len(after['tabs']) == len(before['tabs']) + 1, (before, after)
                assert after['tabs'][-1]['path'] == str(folder), after
                after_close = act('tab-last')
                retained(before, after_close)
                assert len(after_close['tabs']) == len(before['tabs']), after_close
            report['checks'].append('List/Grid/Columns folder hits open background tabs without changing active state')

            act('dual')
            before_dual = ready_target('inactive-folder', other=True)
            after_dual = act('inactive-folder')
            retained(before_dual, after_dual)
            assert after_dual['tabs'] == before_dual['tabs'], (before_dual, after_dual)
            assert after_dual['other_path'] == before_dual['other_path'], (before_dual, after_dual)
            assert len(after_dual['other_tabs']) == len(before_dual['other_tabs']) + 1, (before_dual, after_dual)
            report['checks'].append('inactive dual pane owns its new tab without switching pane or disturbing active listing')

            assert all(digest(path) == value for path, value in originals.items()), 'fixture contents changed'
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
