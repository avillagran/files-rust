#!/usr/bin/env python3
"""Exercise keyed tab input on an isolated hidden nested compositor.

The app fixture prepares and inspects state only. Pointer and keyboard events
are delivered through the private Wayland socket. Evidence and private fixture
files are retained under ~/.cache/files-test, including on failure.
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
spec = importlib.util.spec_from_file_location('nested_harness', ROOT / 'scripts/preview-copy-nested-check.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
STATE = 'FILES_TAB_WIDGET_STATE '


def start_time(pid):
    try:
        s = Path(f'/proc/{pid}/stat').read_text()
        fields = s[s.rindex(')') + 2:].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    ap.add_argument('--size', default='1000x650', choices=('1000x650', '600x650'))
    ap.add_argument('--animations', choices=('on', 'off'), default='on')
    args = ap.parse_args()
    binary = args.binary.resolve(strict=True)
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    base = Path(tempfile.mkdtemp(prefix='keyed-tabs-native-', dir=cache))
    log, raw = base / 'app.log', base / 'frame.ppm'
    report = {'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED',
              'startup_gate': 'NOT_MEASURED', 'binary_path': str(binary), 'requested_size': args.size,
              'animations': args.animations,
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'fixture_dir': str(base), 'checks': [], 'states': [], 'input': []}
    process, identities = None, {}
    try:
        for name, field in (('files-helper', 'helper_sha256'), ('files-html-helper', 'html_helper_sha256')):
            artifact = binary.parent / name
            assert artifact.is_file(), artifact
            report[field] = hashlib.sha256(artifact.read_bytes()).hexdigest()
        pointer = harness.build_pointer(base)
        parent_socket = os.environ.get('WAYLAND_DISPLAY')
        socket, signature, pids = harness.start_nested(base, report)
        identities = {int(pid): start_time(int(pid)) for pid in pids}
        assert identities and all(v is not None for v in identities.values()), 'unidentified compositor'
        assert socket not in (parent_socket, report['parent_socket'], 'wayland-0', 'wayland-1')
        assert signature != os.environ.get('HYPRLAND_INSTANCE_SIGNATURE'), 'parent compositor signature'
        nested = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
        nested.pop('DISPLAY', None)
        home = base / 'home'
        for name in ('Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'A', 'B', 'C',
                     *(f'extra-long-tab-label-{i:02}' for i in range(1, 6))):
            (home / name).mkdir(parents=True)
        originals = {}
        for name in ('A', 'B', 'C'):
            path = home / name / 'file.txt'
            path.write_text('Private keyed tab fixture ' + name + '\n')
            originals[path] = hashlib.sha256(path.read_bytes()).hexdigest()
        devices, shell = base / 'devices.json', base / 'shell.toml'
        for name in ('config', 'data', 'files-config'):
            (base / name).mkdir()
        devices.write_text('{"blockdevices": []}\n')
        shell.write_text('[font]\nbase-size = 14\n')
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1',
               'FILES_TEST_HIDDEN_CONTEXT': '1', 'FILES_TEST_DEVICES_JSON': str(devices),
               'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_ANIM': args.animations,
               'OMARCHY_FLTK_USER_SHELL': str(shell), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        with log.open('w') as output:
            process = subprocess.Popen([str(binary), str(home / 'A'), '--size', args.size,
                                        '--scale', '1', '--no-session', '--ui-smoke',
                                        '--exit-after', '180'], env=env, stdout=subprocess.DEVNULL,
                                       stderr=output, start_new_session=True)

        def text():
            return log.read_text(errors='replace')

        def wait(fn, reason, timeout=15):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                value = fn()
                if value:
                    return value
                if process.poll() is not None:
                    raise AssertionError(f'{reason}: Files exited {process.returncode}')
                time.sleep(.04)
            raise AssertionError(reason)

        def send(action):
            before = len(text())
            pending = raw.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(raw.with_suffix('.popup-command'))
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in text()[before:], action)
            assert 'FILES_TOOLKIT_FIXTURE_INVALID' not in text()[before:]
            return before

        def state(label):
            before = send('tab-widget-state')
            def read():
                return next((json.loads(line[len(STATE):]) for line in text()[before:].split('\n')[:-1]
                             if line.startswith(STATE)), None)
            value = wait(read, 'widget state: ' + label)
            report['states'].append({'label': label, **value})
            return value

        def settled(label, condition=lambda s: True):
            return wait(lambda: (s if s.get('loaded') and s.get('settled') and not s.get('refreshing')
                                  and condition(s) else None) if (s := state(label)) else None,
                        'settled: ' + label)

        def client():
            clients = [c for c in json.loads(harness.hypr(['clients', '-j'], nested)) if c.get('pid') == process.pid]
            assert len(clients) <= 1
            return clients[0] if clients else None

        def point(rect):
            assert rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0, rect
            c = wait(client, 'PID-owned window')
            monitors = json.loads(harness.hypr(['monitors', '-j'], nested))
            assert len(monitors) == 1 and c['monitor'] == monitors[0]['id']
            monitor = monitors[0]
            assert [monitor.get('x', 0), monitor.get('y', 0)] == [0, 0]
            extent = [round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale'])]
            p = [c['at'][0] + rect[0] + rect[2] // 2, c['at'][1] + rect[1] + rect[3] // 2]
            assert 0 <= p[0] < extent[0] and 0 <= p[1] < extent[1], (p, extent)
            return p, extent

        def row(s, key):
            return next(r for r in s['widget']['rows'] if r['key'] == key)

        def pointer_event(rect, mode, end=None):
            p, extent = point(rect)
            q, other = point(end) if end else (p, extent)
            assert extent == other
            assert nested['WAYLAND_DISPLAY'] == socket and socket != parent_socket
            report['input'].append({'mode': mode, 'from': p, 'to': q, 'extent': extent})
            subprocess.run([str(pointer), socket, *map(str, extent), *map(str, p), *map(str, q), mode],
                           env=nested, check=True, timeout=15)

        def key(*args):
            assert nested['WAYLAND_DISPLAY'] == socket and socket != parent_socket
            report['input'].append({'keyboard': list(args)})
            subprocess.run(['wtype', *args], env=nested, check=True, timeout=10)

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
            target = base / (label + '.png')
            subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=15)
            parts = raw.read_bytes().split(b'\n', 3)
            dimensions = [int(v) for v in parts[1].split()]
            current = wait(client, 'capture PID-owned window')
            report.setdefault('captures', []).append({'path': str(target), 'framebuffer_pixels': dimensions,
                                                     'client_logical_at': current['at'],
                                                     'client_logical_size': current['size']})

        def retained(a, b):
            for name in ('path', 'active', 'selected_raw', 'selection_raw', 'anchor_raw', 'generation', 'pane_id'):
                assert a[name] == b[name], (name, a[name], b[name])

        wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
        settled('initial')
        send('tab-gesture:select-file')
        send('tab-context-seed')
        initial = settled('seeded', lambda s: len(s['tabs']) == 3 and len(s['widget']['rows']) >= 3)
        assert initial['selection_raw'] and initial['anchor_raw']
        capture('seeded')
        rows = initial['widget']['rows']
        a, b, c = [next(r['key'] for r in rows if r['id'] == t['id']) for t in initial['tabs']]
        for part in ('body_rect', 'close_rect'):
            pointer_event(row(initial, b)[part], 'r')
            opened = settled('right-' + part, lambda s: s['context_open'])
            retained(initial, opened)
            assert opened['tabs'] == initial['tabs']
            assert opened['context_tab'] == b, opened
            capture('right-' + part)
            key('-k', 'Escape')
            settled('escape', lambda s: not s['context_open'])
            report['checks'].append('right ' + part + ' targets inactive key without activation')
        c_rect = row(initial, c)['body_rect']
        beyond_center = [c_rect[0] + c_rect[2] - 5, c_rect[1], 2, c_rect[3]]
        pointer_event(row(initial, b)['body_rect'], '0', beyond_center)
        moved = settled('drag', lambda s: [t['id'] for t in s['tabs']] == [a[1], c[1], b[1]])
        retained(initial, moved)
        report['checks'].append('drag reorders inactive stable key while preserving browser state')
        outside = [moved['widget']['rect'][0], moved['widget']['rect'][1] + 100, 2, 2]
        pointer_event(row(moved, b)['body_rect'], '0', outside)
        canceled = settled('drop-outside')
        retained(moved, canceled)
        assert canceled['tabs'] == moved['tabs'], canceled
        report['checks'].append('drop outside cancels reorder')
        key('-k', 'Escape')
        canvas = settled('browser-focus', lambda s: s['focus_browser'])
        key('-M', 'ctrl', '-M', 'alt', '-k', 't', '-m', 'alt', '-m', 'ctrl')
        focused = settled('keyboard-focus', lambda s: s['widget']['focus'] and s['widget']['keyboard_mode'])
        retained(canvas, focused)
        retained(moved, focused)
        key('-M', 'alt', '-k', 'Right', '-m', 'alt')
        cursor = settled('keyboard-cursor', lambda s: s['widget']['cursor'] != focused['widget']['cursor'])
        retained(focused, cursor)
        assert cursor['widget']['cursor'][1] != cursor['active'], cursor
        key('-M', 'ctrl', '-M', 'shift', '-k', 'F10', '-m', 'shift', '-m', 'ctrl')
        active_menu = settled('global-active-context', lambda s: s['context_open'])
        assert active_menu['context_tab'] == [active_menu['pane_id'], active_menu['active']], active_menu
        retained(cursor, active_menu)
        key('-k', 'Escape')
        cursor = settled('global-context-dismissed', lambda s: not s['context_open'] and s['widget']['focus'])
        key('-M', 'ctrl', '-M', 'shift', '-k', 'Right', '-m', 'shift', '-m', 'ctrl')
        reordered = settled('keyboard-reorder', lambda s: s['tabs'] != cursor['tabs'])
        retained(cursor, reordered)
        key('-k', 'Menu')
        menu = settled('keyboard-context', lambda s: s['context_open'])
        retained(reordered, menu)
        assert menu['context_tab'] == cursor['widget']['cursor'], menu
        key('-k', 'Escape')
        moved = settled('keyboard-context-dismissed', lambda s: not s['context_open'] and s['widget']['focus'])
        retained(reordered, moved)
        key('-k', 'Escape')
        moved = settled('keyboard-return-browser', lambda s: s['focus_browser'])
        retained(reordered, moved)
        report['checks'].append('native strip focus, global active context, cursor context, reorder and Escape preserve browser state')
        pointer_event(row(moved, b)['body_rect'], 'm')
        closed = settled('middle-body', lambda s: len(s['tabs']) == 2)
        retained(initial, closed)
        assert {t['id'] for t in closed['tabs']} == {a[1], c[1]}
        pointer_event(row(closed, c)['close_rect'], 'm')
        closed_cross = settled('middle-close', lambda s: len(s['tabs']) == 1)
        retained(initial, closed_cross)
        assert closed_cross['tabs'][0]['id'] == a[1]
        report['checks'].append('middle body and close remove inactive keys without activation')
        send('tab-context-seed')
        seeded = settled('reseed', lambda s: len(s['tabs']) == 3)
        b = next(r['key'] for r in seeded['widget']['rows'] if r['path'] == str(home / 'B'))
        pointer_event(row(seeded, b)['body_rect'], 'c1')
        selected = settled('primary-selection', lambda s: s['active'] == b[1])
        assert selected['path'] == str(home / 'B'), selected
        report['checks'].append('primary click selects the intended stable key')
        c = next(r['key'] for r in selected['widget']['rows'] if r['path'] == str(home / 'C'))
        send(f'tab-widget-focus:{c[1]}')
        before_enter = settled('keyboard-select-focus')
        retained(selected, before_enter)
        key('-k', 'Return')
        keyboard_selected = settled('keyboard-select', lambda s: s['active'] == c[1])
        assert keyboard_selected['path'] == str(home / 'C'), keyboard_selected
        report['checks'].append('Enter selects the keyboard cursor key')
        a_rect = row(keyboard_selected, a)['body_rect']
        pointer_event(a_rect, 'c1')
        first_active = settled('first-active', lambda s: s['active'] == a[1])
        assert first_active['path'] == str(home / 'A'), first_active
        send('tab-widget-seed-overflow')
        overflow = settled('overflow', lambda s: len(s['tabs']) == 8)
        last = overflow['tabs'][-1]['id']
        send(f'tab-widget-focus:{last}')
        revealed = settled('overflow-revealed', lambda s: any(r['id'] == last and r['body_rect']
                                                               for r in s['widget']['rows']))
        retained(overflow, revealed)
        assert revealed['widget']['cursor'][1] == last
        report['checks'].append('keyboard focus reveals an overflowed tab without activation')
        key('-k', 'Home')
        home_state = settled('overflow-home')
        first = home_state['tabs'][0]['id']
        first_row = next(r for r in home_state['widget']['rows'] if r['id'] == first)
        assert overflow['active'] == first == a[1], overflow
        rect = first_row['body_rect']
        strip = home_state['widget']['rect']
        assert home_state['widget']['cursor'] == [home_state['pane_id'], first], home_state
        assert rect and rect[0] >= strip[0] and rect[0] + rect[2] <= strip[0] + strip[2], (rect, strip)
        retained(overflow, home_state)
        report['checks'].append('Home reveals already-active first tab after focusing the overflowed last tab')
        send('tab-widget-focus-browser')
        before_global = settled('global-tab-browser', lambda s: s['focus_browser'])
        expected_next = before_global['tabs'][1]['id']
        key('-M', 'ctrl', '-k', 'Tab', '-m', 'ctrl')
        after_global = settled('global-tab-once', lambda s: s['active'] == expected_next)
        assert after_global['active'] != before_global['active'], (before_global, after_global)
        report['checks'].append('Ctrl+Tab changes the active tab once from browser focus')
        send('tab-widget-seed-dual')
        dual = settled('dual')
        left_pane = dual['pane_id']
        send('tab-widget-focus-browser')
        settled('dual-browser', lambda s: s['focus_browser'])
        key('-k', 'Tab')
        other = settled('other-pane', lambda s: s['pane_id'] != left_pane and s['focus_browser'])
        key('-k', 'Down')
        other = settled('other-pane-down', lambda s: bool(s['selection_raw']))
        other_key = next(r['key'] for r in other['widget']['rows'] if r['id'] == other['active'])
        assert other_key[0] != left_pane and other_key[1] == a[1], (other_key, a)
        pointer_event(row(other, other_key)['body_rect'], 'r')
        dual_menu = settled('dual-context', lambda s: s['context_open'])
        assert dual_menu['context_tab'] == other_key, dual_menu
        key('-k', 'Escape')
        settled('dual-dismiss', lambda s: not s['context_open'])
        report['checks'].append('Tab switches panes and Down selects in the focused browser; context uses compound pane and local tab identity')
        assert all(hashlib.sha256(p.read_bytes()).hexdigest() == h for p, h in originals.items())
        report['fixture_intact'] = True
        send('close')
        process.wait(timeout=15)
        assert process.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        report['normal_exit'] = True
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        report['error'] = repr(exc)
    finally:
        if process is not None and process.poll() is None:
            stop_tree(process, None)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid, start in identities.items():
                if start is not None and start_time(pid) == start:
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
            deadline = time.monotonic() + 5
            while any(start_time(p) == t for p, t in identities.items() if t is not None) and time.monotonic() < deadline:
                time.sleep(.1)
        report['nested_stopped'] = bool(identities) and all(start_time(p) != t for p, t in identities.items())
        if not report['nested_stopped']:
            report['functional_gate'] = 'FAIL'
        (base / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(base / 'report.json'), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
