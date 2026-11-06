#!/usr/bin/env python3
"""Real middle clicks in Files, on a private nested Wayland compositor only.

The virtual pointer refuses the user's wayland-0/1 sockets. This runner starts
and stops its own hidden nested Hyprland, identifies the Files window by PID,
and translates the app's painted target rectangles into nested-output points.
Fixture commands only select an initial file and inspect state; folder open,
tab close and ignored-file clicks use BTN_MIDDLE delivered by the compositor.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'docs/evidence/0.0.4'
STATE = 'FILES_TAB_GESTURE_STATE '
spec = importlib.util.spec_from_file_location('middle_nested', ROOT / 'scripts/preview-copy-nested-check.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def process_start(pid):
    try:
        text = Path(f'/proc/{pid}/stat').read_text()
        fields = text[text.rindex(')') + 2:].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--label', default='source')
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    identity = str(time.time_ns())
    name = f'tab-middle-native-{args.label}-{identity}'
    report_path = OUT / (name + '.json')
    saved_log = OUT / (name + '.txt')
    report = {
        'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED',
        'label': args.label, 'binary_path': str(binary), 'log_path': str(saved_log),
        'method': 'Real BTN_MIDDLE over a virtual pointer on a private hidden nested Wayland compositor; painted PID-owned window targets',
        'checks': {}, 'states': [], 'captures': [],
    }
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    process = None
    nested_identities = {}
    with tempfile.TemporaryDirectory(prefix='tab-middle-native-', dir=base_root) as tmp:
        base = Path(tmp)
        log = base / 'app.log'
        try:
            for artifact, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                                  ('files-html-helper', 'html_helper_sha256')]:
                path = binary if artifact == 'files' else binary.parent / artifact
                if path.is_file():
                    report[key] = digest(path)
            pointer = harness.build_pointer(base)
            socket, signature, pids = harness.start_nested(base, report)
            nested_identities = {int(pid): process_start(pid) for pid in pids}
            assert nested_identities and all(start is not None for start in nested_identities.values()), 'cannot identify private compositor processes'
            assert socket != report['parent_socket'] and socket not in ('wayland-0', 'wayland-1'), socket
            nested = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
            home = base / 'home'
            for part in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
                (home / part).mkdir(parents=True)
            directory = home / 'fixture'
            (directory / 'folder').mkdir(parents=True)
            file = directory / 'file.txt'
            file.write_bytes(b'Native middle-click fixture\n')
            (directory / 'folder' / 'child.txt').write_bytes(b'background folder\n')
            original = digest(file)
            raw = base / 'frame.ppm'
            devices = base / 'devices.json'
            devices.write_text('{"blockdevices": []}\n')
            shell = base / 'shell.toml'
            shell.write_text('[font]\nbase-size = 14\n')
            env = {
                **nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
                'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
                'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1',
                'FILES_TEST_DEVICES_JSON': str(devices), 'FILES_CLOSE_TRACE': '1',
                'OMARCHY_FLTK_ANIM': 'off', 'OMARCHY_FLTK_USER_SHELL': str(shell),
                'TMPDIR': str(base), 'MAGICK_TEMPORARY_PATH': str(base),
            }
            env.pop('FILES_TEST_ISOLATE_INPUT', None)
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(directory), '--size', '1000x650', '--scale', '1',
                     '--no-session', '--ui-smoke', '--exit-after', '60'],
                    env=env, stdout=subprocess.DEVNULL, stderr=stream, start_new_session=True)

            def text():
                return log.read_text(errors='replace') if log.exists() else ''

            def wait(check, why, timeout=15, allow_exit=False):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = check()
                    if value:
                        return value
                    if process.poll() is not None and not allow_exit:
                        raise AssertionError(f'{why}: Files exited {process.returncode}')
                    time.sleep(.03)
                raise AssertionError(why)

            def command(action):
                before = len(text())
                pending = raw.with_suffix('.pending')
                pending.write_text(action)
                pending.replace(raw.with_suffix('.popup-command'))
                marker = f'FILES_TOOLKIT_FIXTURE action={action}\n'
                wait(lambda: marker in text()[before:], action)
                return before

            def state():
                before = command('tab-gesture-state')
                def complete():
                    return next((json.loads(line[len(STATE):])
                                 for line in text()[before:].split('\n')[:-1]
                                 if line.startswith(STATE)), None)
                value = wait(complete, 'complete gesture state')
                report['states'].append(value)
                return value

            def ready(value, targets=()):
                return (value.get('loaded') is True and value.get('settled') is True
                        and value.get('refreshing') is False
                        and all(value.get('targets', {}).get(target) for target in targets))

            def settled_state(targets=()):
                def complete():
                    value = state()
                    return value if ready(value, targets) else None
                return wait(complete, 'settled listing and painted targets: ' + ','.join(targets))

            def tabs_state(count):
                def complete():
                    value = state()
                    return value if len(value['tabs']) == count and ready(value, ('folder', 'file', 'tab-last')) else None
                return wait(complete, f'settled native result with {count} tabs')

            def current_client():
                clients = [c for c in json.loads(harness.hypr(['clients', '-j'], nested))
                           if c.get('pid') == process.pid]
                assert len(clients) <= 1, clients
                return clients[0] if clients else None

            def middle(target):
                value = settled_state((target,))
                rect = value.get('targets', {}).get(target)
                assert rect and len(rect) == 4, (target, value.get('targets'))
                x, y, width, height = rect
                assert width > 0 and height > 0, rect
                client = wait(current_client, 'PID-owned Files window')
                monitors = json.loads(harness.hypr(['monitors', '-j'], nested))
                monitor = next((m for m in monitors if m.get('id') == client.get('monitor')), monitors[0])
                extent = [round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale'])]
                local = [x + width // 2, y + height // 2]
                point = [client['at'][0] + local[0], client['at'][1] + local[1]]
                origin = [monitor.get('x', 0), monitor.get('y', 0)]
                assert origin == [0, 0], 'private compositor must use one output at 0,0'
                assert 0 <= point[0] < extent[0] and 0 <= point[1] < extent[1], (target, point, extent)
                report.setdefault('input', []).append({'target': target, 'rect': rect, 'point': point,
                                                      'extent': extent, 'pid': process.pid})
                subprocess.run([str(pointer), socket, str(extent[0]), str(extent[1]),
                                str(point[0]), str(point[1]), str(point[0]), str(point[1]), 'm'],
                               check=True, timeout=10, env=nested)

            def capture(label):
                raw.unlink(missing_ok=True)
                raw.with_suffix('.request').touch()
                def complete():
                    if not raw.exists():
                        return False
                    parts = raw.read_bytes().split(b'\n', 3)
                    if len(parts) != 4:
                        return False
                    dimensions = [int(v) for v in parts[1].split()]
                    return len(dimensions) == 2 and len(parts[3]) == dimensions[0] * dimensions[1] * 3
                wait(complete, 'capture ' + label)
                path = OUT / f'{name}-{label}.png'
                subprocess.run(['magick', str(raw), str(path)], env=env, check=True, timeout=15)
                report['captures'].append(str(path))

            def unchanged(before, after):
                for key in ['path', 'active', 'selected_raw', 'selection_raw', 'anchor_raw', 'generation', 'pane_id']:
                    assert before[key] == after[key], (key, before[key], after[key])
                assert not after['closing'], after

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'Files smoke ready')
            wait(current_client, 'PID-owned window mapped')
            settled_state(('folder', 'file', 'tab-last'))
            command('tab-gesture:select-file')
            before = settled_state(('folder', 'file', 'tab-last'))
            assert before['selected_raw'] and before['selection_raw'] and before['anchor_raw'], before
            assert len(before['tabs']) == 1, before
            capture('before')
            middle('folder')
            after = tabs_state(2)
            unchanged(before, after)
            assert after['tabs'][-1]['path'] == str(directory / 'folder'), after
            assert after['tabs'][-1]['id'] != before['active'], after
            report['checks']['background_folder_preserves_active'] = True
            capture('background-tab')
            middle('tab-last')
            closed = tabs_state(1)
            unchanged(before, closed)
            assert closed['tabs'] == before['tabs'], closed
            report['checks']['middle_tab_closes_without_navigation'] = True
            middle('file')
            time.sleep(.2)
            ignored = settled_state(('folder', 'file', 'tab-last'))
            unchanged(closed, ignored)
            assert ignored['tabs'] == closed['tabs'], ignored
            report['checks']['middle_file_is_ignored'] = True
            assert digest(file) == original, 'fixture bytes changed'
            report['checks']['fixture_intact'] = True
            command('close')
            process.wait(timeout=15)
            assert process.returncode == 0, process.returncode
            assert 'FILES_CLOSE_DRAINED' in text(), 'normal worker drain was not recorded'
            report['normal_exit'] = True
            report['functional_gate'] = 'PASS'
        except Exception as error:
            report['error'] = repr(error)
        finally:
            if process is not None and process.poll() is None:
                stop_tree(process, None)
            saved_log.write_text(log.read_text(errors='replace') if log.exists() else '')
            report['exit_code'] = process.returncode if process is not None else None
            def nested_alive(pid, start):
                return start is not None and process_start(pid) == start
            for sig in [signal.SIGTERM, signal.SIGKILL]:
                for pid, start in nested_identities.items():
                    if nested_alive(pid, start):
                        try:
                            os.kill(pid, sig)
                        except ProcessLookupError:
                            pass
                deadline = time.monotonic() + 10
                while any(nested_alive(pid, start) for pid, start in nested_identities.items()) and time.monotonic() < deadline:
                    time.sleep(.1)
                if not any(nested_alive(pid, start) for pid, start in nested_identities.items()):
                    break
            identified = bool(nested_identities) and all(start is not None for start in nested_identities.values())
            report['nested_stopped'] = identified and not any(nested_alive(pid, start) for pid, start in nested_identities.items())
            if not report['nested_stopped']:
                report['functional_gate'] = 'FAIL'
                report.setdefault('error', 'private nested compositor did not stop')
            report_path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(report_path), 'functional_gate': report['functional_gate'], 'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
