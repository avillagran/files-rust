#!/usr/bin/env python3
"""Check tab context commands through private Files fixtures and real frames.

Run under Xvfb or scripts/nested-run.py. The fixture opens the production
popup from painted tab targets, then drives its toolkit keyboard handlers.
This verifies app behavior and framebuffers, not native OS pointer delivery.
HOME, config, data and all directory contents belong to the private fixture.
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
STATE = 'FILES_TAB_CONTEXT_STATE '
W, H = 1000, 650


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--animations', choices=['on', 'off'], default='on')
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    identity = str(time.time_ns())
    report_path = OUT / f'tab-context-ui-{identity}.json'
    saved_log = OUT / f'tab-context-ui-{identity}.txt'
    report = {
        'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED',
        'startup_gate': 'NOT_MEASURED', 'binary_path': str(binary),
        'method': 'Private painted tab context targets and toolkit popup/key handlers with framebuffer captures; no native input',
        'log_path': str(saved_log), 'states': [], 'checks': [], 'captures': [],
        'viewport': [W, H], 'animations': args.animations,
    }
    process = None
    with tempfile.TemporaryDirectory(prefix='tab-context-', dir=base_root) as tmp:
        base = Path(tmp)
        log = base / 'app.log'
        raw = base / 'frame.ppm'
        try:
            home = base / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
                (home / name).mkdir(parents=True)
            locations = [home / name for name in ['A', 'B', 'C']]
            originals = {}
            for folder in locations:
                folder.mkdir()
                fixture = folder / 'file.txt'
                fixture.write_bytes(f'Tab context fixture {folder.name}\n'.encode())
                originals[fixture] = digest(fixture)
            (base / 'data').mkdir()
            (base / 'config').mkdir()
            devices = base / 'devices.json'
            devices.write_text('{"blockdevices": []}\n')
            shell = base / 'shell.toml'
            shell.write_text('[font]\nbase-size = 14\n')
            for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                              ('files-html-helper', 'html_helper_sha256')]:
                artifact = binary if name == 'files' else binary.parent / name
                assert artifact.is_file(), artifact
                report[key] = digest(artifact)
            env = {
                **os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
                'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
                'FILES_TEST_DEVICES_JSON': str(devices), 'FILES_TEST_CAPTURE': str(raw),
                'FILES_TEST_FIXED_SIZE': '1', 'FILES_TEST_ISOLATE_INPUT': '1',
                'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
                'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_ANIM': args.animations,
                'OMARCHY_FLTK_USER_SHELL': str(shell), 'TMPDIR': str(base),
                'MAGICK_TEMPORARY_PATH': str(base),
            }
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(locations[0]), '--size', f'{W}x{H}', '--scale', '1',
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
                output = text()[before:]
                assert 'FILES_TOOLKIT_FIXTURE_INVALID' not in output, (action, output)
                assert 'result=Err(' not in output, (action, output)
                return before

            def state(label):
                before = send('tab-context-state')

                def complete():
                    for line in text()[before:].split('\n')[:-1]:
                        if line.startswith(STATE):
                            return json.loads(line[len(STATE):])
                    return None

                value = wait(complete, f'{label}: complete tab context state')
                assert not value['closing'], value
                report['states'].append({'label': label, **value})
                return value

            def ready(label, opened=False, path=None, count=None):
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    value = state(label)
                    if (value.get('loaded') is True and value.get('settled') is True
                            and value.get('refreshing') is False
                            and value.get('context_open') is opened
                            and (path is None or value['path'] == str(path))
                            and (count is None or len(value['tabs']) == count)):
                        return value
                    time.sleep(.03)
                raise RuntimeError(f'{label}: settled context state was not ready')

            def retained(before, after):
                keys = ['path', 'active', 'selected_raw', 'selection_raw', 'anchor_raw',
                        'generation', 'pane_id']
                assert {key: before[key] for key in keys} == {key: after[key] for key in keys}, (before, after)

            def open_context(target, baseline):
                send('tab-context-' + target)
                value = ready('opened-' + target, opened=True, count=len(baseline['tabs']))
                retained(baseline, value)
                assert value['tabs'] == baseline['tabs'], (baseline, value)
                return value

            def capture(label):
                raw.unlink(missing_ok=True)
                raw.with_suffix('.request').touch()

                def complete():
                    if not raw.exists():
                        return False
                    parts = raw.read_bytes().split(b'\n', 3)
                    return (len(parts) == 4 and parts[1] == f'{W} {H}'.encode()
                            and len(parts[3]) == W * H * 3)

                wait(complete, f'{label}: framebuffer')
                target = OUT / f'tab-context-{label}-{identity}.png'
                subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=10)
                report['captures'].append(str(target))

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
            ready('loaded-A', path=locations[0], count=1)
            send('tab-gesture:select-file')
            initial = ready('selected-A', path=locations[0], count=1)
            expected_name = b'file.txt'.hex()
            assert initial['selected_raw'] == initial['anchor_raw'] == expected_name, initial
            assert initial['selection_raw'] == [expected_name], initial
            send('tab-context-seed')
            seeded = ready('seeded', path=locations[0], count=3)
            retained(initial, seeded)
            assert [row['path'] for row in seeded['tabs']] == list(map(str, locations)), seeded
            a, b, c = [row['id'] for row in seeded['tabs']]
            assert len({a, b, c}) == 3 and seeded['active'] == a, seeded
            capture('three-tabs')

            send('tab-context-key-right')
            shifted = ready('keyboard-move-right', path=locations[0], count=3)
            retained(seeded, shifted)
            assert shifted['tabs'] == [seeded['tabs'][1], seeded['tabs'][0], seeded['tabs'][2]], shifted
            send('tab-context-key-left')
            shifted_back = ready('keyboard-move-left', path=locations[0], count=3)
            retained(seeded, shifted_back)
            assert shifted_back['tabs'] == seeded['tabs'], shifted_back
            report['checks'].append('CtrlShiftPageDown and CtrlShiftPageUp move the active stable tab while preserving browser state')

            open_context('last', seeded)
            capture('inactive-tab-menu')
            send('escape')
            escaped = ready('escaped', path=locations[0], count=3)
            retained(seeded, escaped)
            assert escaped['tabs'] == seeded['tabs'] and escaped['focus_browser'] is True, escaped
            report['checks'].append('context on an inactive tab and Escape preserve active path, selection, anchor and generation; focus returns to browser')

            send('tab-context-key-open')
            keyboard = ready('keyboard-open', opened=True, path=locations[0], count=3)
            retained(seeded, keyboard)
            assert keyboard['tabs'] == seeded['tabs'], keyboard
            send('escape')
            keyboard_escaped = ready('keyboard-escaped', path=locations[0], count=3)
            retained(seeded, keyboard_escaped)
            assert keyboard_escaped['focus_browser'] is True, keyboard_escaped
            report['checks'].append('Ctrl+Shift+F10 opens the active-tab context and Escape restores browser focus without navigation')

            open_context('last', seeded)
            send('tab-context-move-left')
            moved = ready('moved-left', path=locations[0], count=3)
            retained(seeded, moved)
            assert [row['id'] for row in moved['tabs']] == [a, c, b], moved
            assert [row['path'] for row in moved['tabs']] == [str(locations[i]) for i in [0, 2, 1]], moved
            assert moved['closed'] == seeded['closed'], moved
            assert moved['focus_browser'] is True, moved
            capture('reordered-with-A-active')
            report['checks'].append('Move Left reorders the clicked stable tab ID without activating it or changing active browser state')

            open_context('first', moved)
            send('tab-context-close-right')
            right_closed = ready('closed-right', path=locations[0], count=1)
            retained(seeded, right_closed)
            assert right_closed['tabs'] == seeded['tabs'][:1], right_closed
            assert right_closed['closed'] == seeded['closed'] + 2, right_closed
            report['checks'].append('Close Right uses current visual order and closes only tabs to the right, retaining the active browser')

            send('tab-context-seed')
            reseeded = ready('reseeded', path=locations[0], count=3)
            retained(right_closed, reseeded)
            assert [row['path'] for row in reseeded['tabs']] == list(map(str, locations)), reseeded
            assert all(row['id'] > max(a, b, c) for row in reseeded['tabs'][1:]), reseeded
            open_context('first', reseeded)
            send('tab-context-close-others')
            others_closed = ready('closed-others', path=locations[0], count=1)
            retained(reseeded, others_closed)
            assert others_closed['tabs'] == reseeded['tabs'][:1], others_closed
            assert others_closed['closed'] == reseeded['closed'] + 2, others_closed
            assert others_closed['focus_browser'] is True, others_closed
            report['checks'].append('Close Others keeps the clicked active survivor; fresh IDs are not recycled after batch closure')

            send('tab-context-seed')
            inactive_seed = ready('inactive-survivor-seeded', path=locations[0], count=3)
            retained(others_closed, inactive_seed)
            clicked = inactive_seed['tabs'][-1]
            assert clicked['path'] == str(locations[2]), inactive_seed
            open_context('last', inactive_seed)
            send('tab-context-close-others')
            survivor = ready('inactive-survivor-activated', path=locations[2], count=1)
            assert survivor['tabs'] == [clicked] and survivor['active'] == clicked['id'], survivor
            assert survivor['closed'] == inactive_seed['closed'] + 2, survivor
            assert survivor['generation'] != inactive_seed['generation'], (inactive_seed, survivor)
            assert survivor['focus_browser'] is True, survivor
            capture('clicked-survivor-active')
            report['checks'].append('Close Others on an inactive tab activates that survivor only after closing the prior active tab')

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
