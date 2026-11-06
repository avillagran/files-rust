#!/usr/bin/env python3
"""Check operation notices with native input on an isolated compositor.

The private bus has no service activation. Fixtures and all evidence are kept
under ~/.cache/files-test, including on failure.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from xml.sax.saxutils import escape

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('nested_harness', ROOT / 'scripts/preview-copy-nested-check.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
PREFIX = 'FILES_OPERATION_NOTICE_STATE '


def started(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--size', choices=('1000x650', '600x650'), default='1000x650')
    parser.add_argument('--animations', choices=('on', 'off'), default='on')
    parser.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    if not args.private_child:
        busdir = Path(tempfile.mkdtemp(prefix='operation-notices-bus-', dir=cache))
        config = busdir / 'bus.conf'
        config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + escape(str(busdir)) +
                          '</listen><auth>EXTERNAL</auth><policy context="default">'
                          '<allow send_destination="*"/><allow receive_sender="*"/>'
                          '<allow own="*"/></policy></busconfig>')
        env = dict(os.environ)
        env['FILES_TEST_PARENT_BUS'] = env.get('DBUS_SESSION_BUS_ADDRESS', '')
        env['FILES_TEST_BUS_DIR'] = str(busdir)
        env.pop('DBUS_SESSION_BUS_ADDRESS', None)
        return subprocess.run(['dbus-run-session', '--config-file', str(config), '--', sys.executable,
                               str(Path(__file__).resolve()), '--private-child', '--binary', str(args.binary.resolve()),
                               '--size', args.size, '--animations', args.animations], env=env).returncode
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS', '')
    assert bus != os.environ.get('FILES_TEST_PARENT_BUS', '') and bus.startswith(
        'unix:path=' + os.environ.get('FILES_TEST_BUS_DIR', '<absent>') + '/'), 'refusing non-private bus'
    binary = args.binary.resolve(strict=True)
    base = Path(tempfile.mkdtemp(prefix='operation-notices-native-', dir=cache))
    report = {'functional_gate': 'FAIL', 'binary_path': str(binary), 'requested_size': args.size,
              'animations': args.animations, 'fixture_dir': str(base), 'checks': [], 'states': [], 'captures': [],
              'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED'}
    process = None
    identities = {}
    try:
        for name, field in (('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                            ('files-html-helper', 'html_helper_sha256')):
            report[field] = hashlib.sha256((binary if name == 'files' else binary.parent / name).read_bytes()).hexdigest()
        pointer = harness.build_pointer(base)
        parent = os.environ.get('WAYLAND_DISPLAY')
        socket, signature, pids = harness.start_nested(base, report)
        identities = {int(pid): started(int(pid)) for pid in pids}
        assert identities and all(identities.values())
        assert socket not in (parent, report['parent_socket'], 'wayland-0', 'wayland-1')
        assert signature != os.environ.get('HYPRLAND_INSTANCE_SIGNATURE')
        nested = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
        nested.pop('DISPLAY', None)
        home = base / 'home'
        for name in ('Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'source', 'target'):
            (home / name).mkdir(parents=True)
        for name, data in (('rename-before.txt', b'private rename fixture\n'), ('copy-payload.txt', b'private copy fixture\n'),
                           ('focus.txt', b'private focus fixture\n')):
            (home / 'source' / name).write_bytes(data)
        for name in ('config', 'data', 'files-config'):
            (base / name).mkdir()
        (base / 'devices.json').write_text('{"blockdevices": []}\n')
        (base / 'shell.toml').write_text('[font]\nbase-size = 14\n')
        raw, log = base / 'frame.ppm', base / 'app.log'
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1',
               'FILES_TEST_OPERATION_NOTICES': '1',
               'FILES_TEST_HIDDEN_CONTEXT': '1', 'FILES_TEST_DEVICES_JSON': str(base / 'devices.json'),
               'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_ANIM': args.animations,
               'OMARCHY_FLTK_USER_SHELL': str(base / 'shell.toml'), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        with log.open('w') as output:
            process = subprocess.Popen([str(binary), str(home / 'source'), '--size', args.size, '--scale', '1',
                                        '--no-session', '--ui-smoke', '--exit-after', '180'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=output, start_new_session=True)

        def text():
            return log.read_text(errors='replace')

        def wait(fn, reason, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                value = fn()
                if value:
                    return value
                if process.poll() is not None:
                    raise AssertionError(f'{reason}: process exited {process.returncode}')
                time.sleep(.04)
            raise AssertionError(reason)

        def send(action):
            offset = len(text())
            pending = raw.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(raw.with_suffix('.popup-command'))
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in text()[offset:], action)
            assert 'FILES_TOOLKIT_FIXTURE_INVALID' not in text()[offset:]
            return offset

        def state(label):
            offset = send('operation-notice-state')
            result = wait(lambda: next((json.loads(line[len(PREFIX):]) for line in text()[offset:].splitlines()
                                        if line.startswith(PREFIX)), None), 'state ' + label)
            report['states'].append({'label': label, **result})
            return result

        def until(label, condition):
            return wait(lambda: (s if condition(s) else None) if (s := state(label)) else None, label)

        def key(*keys):
            assert nested['WAYLAND_DISPLAY'] == socket and socket != parent
            subprocess.run(['wtype', *keys], env=nested, check=True, timeout=10)

        def capture(label):
            raw.unlink(missing_ok=True)
            raw.with_suffix('.request').touch()
            def complete():
                if not raw.exists():
                    return False
                parts = raw.read_bytes().split(b'\n', 3)
                if len(parts) != 4:
                    return False
                try:
                    w, h = map(int, parts[1].split())
                except ValueError:
                    return False
                return len(parts[3]) == w * h * 3
            wait(complete, 'capture ' + label)
            target = base / (label + '.png')
            subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=15)
            report['captures'].append(str(target))

        def clients():
            return [c for c in json.loads(harness.hypr(['clients', '-j'], nested)) if c.get('pid') == process.pid]

        def click(rect):
            assert rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0
            client = wait(lambda: clients()[0] if len(clients()) == 1 else None, 'owned client')
            monitor = json.loads(harness.hypr(['monitors', '-j'], nested))[0]
            assert [monitor.get('x', 0), monitor.get('y', 0)] == [0, 0]
            extent = [round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale'])]
            x, y = client['at'][0] + rect[0] + rect[2] // 2, client['at'][1] + rect[1] + rect[3] // 2
            assert 0 <= x < extent[0] and 0 <= y < extent[1], (client['at'], rect, [x, y], extent)
            subprocess.run([str(pointer), socket, *map(str, extent), str(x), str(y), str(x), str(y), 'c1'],
                           env=nested, check=True, timeout=10)

        wait(lambda: 'FILES_UI_SMOKE_OK' in text() and len(clients()) == 1, 'startup')
        if args.size == '1000x650':
            address = clients()[0]['address']
            target = 'address:' + address
            quoted = json.dumps(target)
            for legacy, lua in ((['setfloating', target],
                                  f'hl.dsp.window.float({{action="on", window={quoted}}})'),
                                 (['resizewindowpixel', 'exact 1000 650,' + target],
                                  f'hl.dsp.window.resize({{x=1000, y=650, relative=false, window={quoted}}})')):
                result = harness.hypr(['dispatch', lua], nested).strip()
                if result.lower() != 'ok':
                    result = harness.hypr(['dispatch', *legacy], nested).strip()
                assert result.lower() == 'ok', result
            wait(lambda: clients() and clients()[0]['size'] == [1000, 650], 'window size')
        report['window_size'] = clients()[0]['size']
        time.sleep(1)
        before = until('browser-initial', lambda s: s['browser_focus'] and not s['card'])
        send('operation-notice-clock-freeze')
        send('operation-notice-info')
        shown = until('info', lambda s: s['card'] and s['card']['visible'])
        assert shown['browser_focus'] and shown['capacity'] == 1 and shown['request_capacity'] == 8
        assert shown['selection_raw'] == before['selection_raw'] and shown['anchor_raw'] == before['anchor_raw']
        capture('info-start')
        send('operation-notice-clock-step:80'); capture('info-middle')
        send('operation-notice-clock-step:1000'); capture('info-settled')
        old = shown['card']['revision']
        send('operation-notice-replace')
        replaced = until('replacement', lambda s: s['card'] and s['card']['revision'] != old)
        assert replaced['browser_focus'] and replaced['queue_len'] <= 8
        capture('replacement-start')
        send('operation-notice-clock-step:80'); capture('replacement-middle')
        send('operation-notice-info')
        until('rapid-reversal', lambda s: s['card'] and s['card']['revision'] != replaced['card']['revision'])
        capture('reversal-start')
        send('operation-notice-clock-step:1000'); capture('reversal-settled')
        key('-M', 'ctrl', '-M', 'alt', '-k', 'n', '-m', 'alt', '-m', 'ctrl')
        focused = until('notice-focus', lambda s: s['card'] and s['card']['focused'])
        assert focused['focus_rect'] != before['focus_rect']
        key('-k', 'Escape')
        until('escape-browser-focus', lambda s: s['browser_focus'])
        send('operation-notice-clear')
        send('operation-notice-clock-step:1000')
        until('clear', lambda s: not s['card'] or not s['card']['visible'])
        report['checks'].append('native focus, replacement, reversal and browser-focus preservation')
        send('operation-notice-clock-resume')
        # Real operation setup is provided by the UI fixture; actions are native input only.
        send('operation-notice-fixture-rename')
        until('rename-ready', lambda s: s.get('rename_form') and s['rename_form']['prepared'])
        key('-k', 'Return')
        renamed = home / 'source' / 'rename-after.txt'
        original = home / 'source' / 'rename-before.txt'
        wait(lambda: renamed.exists() and not original.exists(), 'rename completed')
        rename = until('rename', lambda s: s['card'] and s['card']['undo_rect'] and s['can_undo'])
        capture('rename-undo')
        key('-k', 'End')
        until('select-renamed', lambda s: s['selected_raw'] == list(b'rename-after.txt'))
        key('-k', 'F2')
        until('unapplied-rename-preview', lambda s: s.get('rename_form') and s['rename_form']['prepared'])
        key('-k', 'Escape')
        rename = until('undo-after-cancelled-preview', lambda s: not s.get('rename_form') and s['card']
                       and s['card']['undo_rect'] and s['can_undo'])
        if args.size == '1000x650':
            key('-M', 'ctrl', '-M', 'alt', '-k', 'n', '-m', 'alt', '-m', 'ctrl')
            until('undo-focused', lambda s: s['card'] and s['card']['focused'])
            key('-k', 'Return')
        else:
            click(rename['card']['undo_rect'])
        wait(lambda: original.exists() and not renamed.exists(), 'native undo completed')
        assert original.read_bytes() == b'private rename fixture\n'
        until('redo-ready', lambda s: s['card'] and s['card']['redo_rect'] and s['can_redo'])
        key('-M', 'ctrl', '-M', 'alt', '-k', 'n', '-m', 'alt', '-m', 'ctrl')
        until('redo-focused', lambda s: s['card'] and s['card']['focused'])
        key('-k', 'Return')
        wait(lambda: renamed.exists() and not original.exists(), 'native redo completed')
        assert renamed.read_bytes() == b'private rename fixture\n'
        until('undo-again-ready', lambda s: s['card'] and s['card']['undo_rect'] and s['can_undo'])
        key('-M', 'ctrl', '-M', 'alt', '-k', 'n', '-m', 'alt', '-m', 'ctrl')
        until('undo-again-focused', lambda s: s['card'] and s['card']['focused'])
        key('-k', 'Return')
        wait(lambda: original.exists() and not renamed.exists(), 'second native undo completed')
        assert original.read_bytes() == b'private rename fixture\n'
        report['checks'].append('native undo, redo and undo preserve real rename history and bytes')
        send('operation-notice-fixture-copy')
        until('copy-ready', lambda s: s.get('copy_form') and not s['copy_form']['preparing'])
        key('-k', 'Return')
        target = home / 'target' / 'copy-payload.txt'
        wait(lambda: target.exists() and target.read_bytes() == b'private copy fixture\n', 'copy completed')
        copied = until('copy-result', lambda s: s['card'] and s['card']['details_rect'] and s['copy_result'])
        capture('copy-result')
        if args.size == '1000x650':
            key('-M', 'ctrl', '-M', 'alt', '-k', 'n', '-m', 'alt', '-m', 'ctrl')
            until('details-focused', lambda s: s['card'] and s['card']['focused'])
            key('-k', 'Return')
        else:
            click(copied['card']['details_rect'])
        until('copy-details', lambda s: s['modal'] and s['copy_result'] == copied['copy_result'])
        report['checks'].append('native details opens the latest real copy report')
        key('-k', 'Escape')
        send('close')
        process.wait(timeout=20)
        assert process.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        report['checks'].append('close drains workers')
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        report['error'] = repr(exc)
    finally:
        if process is not None and process.poll() is None:
            stop_tree(process, None)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid, stamp in identities.items():
                if stamp is not None and started(pid) == stamp:
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
            deadline = time.monotonic() + 5
            while any(started(pid) == stamp for pid, stamp in identities.items() if stamp is not None) and time.monotonic() < deadline:
                time.sleep(.1)
        report['nested_stopped'] = bool(identities) and all(started(pid) != stamp for pid, stamp in identities.items())
        if not report['nested_stopped']:
            report['functional_gate'] = 'FAIL'
        (base / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(base / 'report.json'), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
