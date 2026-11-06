#!/usr/bin/env python3
"""Exercise FileManager1 on a private bus and hidden nested compositor.

All fixtures and evidence are retained under ~/.cache/files-test. Run only with
an already built native binary; this script does not build or install anything.
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
from urllib.parse import quote_from_bytes
from xml.sax.saxutils import escape

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('nested_harness', ROOT / 'scripts/preview-copy-nested-check.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
NAME = 'org.freedesktop.FileManager1'
OBJECT = '/org/freedesktop/FileManager1'
PREFIX = 'FILES_FILE_MANAGER_STATE '


def start_time(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--size', choices=('1000x650', '600x650'), default='1000x650')
    parser.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.private_child:
        cache = Path.home() / '.cache/files-test'
        cache.mkdir(parents=True, exist_ok=True)
        bus_dir = Path(tempfile.mkdtemp(prefix='filemanager1-bus-', dir=cache))
        config = bus_dir / 'bus.conf'
        config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + escape(str(bus_dir)) +
                          '</listen><auth>EXTERNAL</auth><policy context="default">'
                          '<allow send_destination="*"/><allow receive_sender="*"/>'
                          '<allow own="*"/></policy></busconfig>')
        env = dict(os.environ)
        env['FILES_TEST_PARENT_BUS'] = env.get('DBUS_SESSION_BUS_ADDRESS', '')
        env['FILES_TEST_BUS_DIR'] = str(bus_dir)
        env.pop('DBUS_SESSION_BUS_ADDRESS', None)
        return subprocess.run(['dbus-run-session', '--config-file', str(config), '--', sys.executable, str(Path(__file__).resolve()),
                               '--private-child', '--binary', str(args.binary.resolve()), '--size', args.size], env=env).returncode
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS', '')
    assert bus and bus != os.environ.get('FILES_TEST_PARENT_BUS', ''), 'refusing inherited session bus'
    bus_dir = os.environ.get('FILES_TEST_BUS_DIR', '')
    assert bus_dir and bus.startswith('unix:path=' + bus_dir + '/'), 'refusing a non-private bus socket'
    binary = args.binary.resolve(strict=True)
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    base = Path(tempfile.mkdtemp(prefix='filemanager1-native-', dir=cache))
    report = {'functional_gate': 'FAIL', 'binary': str(binary), 'checks': [], 'states': [],
              'fixture_dir': str(base), 'bus_address': bus, 'resource_gate': 'NOT_MEASURED',
              'startup_gate': 'NOT_MEASURED'}
    process = cli = None
    identities = {}
    handles = []
    nested = None
    try:
        for name, field in (('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                            ('files-html-helper', 'html_helper_sha256')):
            artifact = binary if name == 'files' else binary.parent / name
            report[field] = hashlib.sha256(artifact.read_bytes()).hexdigest()
        parent_socket = os.environ.get('WAYLAND_DISPLAY')
        socket, signature, pids = harness.start_nested(base, report)
        identities = {int(pid): start_time(int(pid)) for pid in pids}
        assert identities and all(identities.values()), 'unidentified compositor'
        assert socket not in (parent_socket, report['parent_socket'], 'wayland-0', 'wayland-1')
        assert signature != os.environ.get('HYPRLAND_INSTANCE_SIGNATURE')
        nested = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
        nested.pop('DISPLAY', None)
        home = base / 'home'
        for name in ('A', 'B', 'Downloads', 'Documents', 'Pictures', 'Music', 'Videos'):
            (home / name).mkdir(parents=True)
        for name in ('config', 'data', 'files-config'):
            (base / name).mkdir()
        devices, shell = base / 'devices.json', base / 'shell.toml'
        devices.write_text('{"blockdevices": []}\n')
        shell.write_text('[font]\nbase-size = 14\n')
        raw = base / 'frame.ppm'
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1',
               'FILES_TEST_HIDDEN_CONTEXT': '1', 'FILES_TEST_DEVICES_JSON': str(devices),
               'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_USER_SHELL': str(shell),
               'TMPDIR': str(base), 'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        fixtures = [os.fsencode(home / 'A') + b'/' + n for n in
                    (b'first.txt', b'second.txt', b'raw-\xff', b'percent%name', b'line\nname', b'.secret')]
        fixtures += [os.fsencode(home / 'B') + b'/other.txt']
        for path in fixtures:
            with open(path, 'wb') as f:
                f.write(b'Private FileManager1 fixture\n')
        odd_dir = os.fsencode(home) + b'/directory-\xff'
        os.mkdir(odd_dir)
        hashes = {p: hashlib.sha256(open(p, 'rb').read()).hexdigest() for p in fixtures}
        uri = lambda p: 'file://' + quote_from_bytes(p, safe='/')
        log = base / 'service.log'
        output = log.open('w'); handles.append(output)
        process = subprocess.Popen([str(binary), '--file-manager-service', '--size', args.size, '--scale', '1',
                                    '--no-session', '--ui-smoke', '--exit-after', '180'], env=env,
                                   stdout=subprocess.DEVNULL, stderr=output, start_new_session=True)

        def text():
            return log.read_text(errors='replace')

        def wait(fn, reason, timeout=15):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                value = fn()
                if value:
                    return value
                if process.poll() is not None:
                    raise AssertionError(f'{reason}: service exited {process.returncode}')
                time.sleep(.05)
            raise AssertionError(reason)

        def call(dest, path, method, *arguments, timeout=10):
            result = subprocess.run(['gdbus', 'call', '--session', '--dest', dest, '--object-path', path,
                                     '--method', method, *arguments], env=env, capture_output=True,
                                    text=True, timeout=timeout)
            report.setdefault('calls', []).append({'method': method, 'args': list(arguments),
                'code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
            return result

        def owner():
            r = call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                     'org.freedesktop.DBus.GetNameOwner', NAME)
            return r.stdout.strip().split("'")[1] if r.returncode == 0 else None

        def clients(pid):
            return [c for c in json.loads(harness.hypr(['clients', '-j'], nested)) if c.get('pid') == pid]

        def rpc(method, paths, ok=True):
            # JSON strings are valid GVariant string literals, and argv avoids a shell.
            array = '[' + ', '.join(json.dumps(x, ensure_ascii=True) for x in paths) + ']'
            r = call(NAME, OBJECT, NAME + '.' + method, array, '')
            assert (r.returncode == 0) == ok, (method, paths, r.stdout, r.stderr)
            return r

        def command(action):
            offset = len(text())
            pending = raw.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(raw.with_suffix('.popup-command'))
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in text()[offset:], action)
            assert 'FILES_TOOLKIT_FIXTURE_INVALID' not in text()[offset:]
            return offset

        def state(label):
            offset = command('file-manager-state')
            def read():
                return next((json.loads(line[len(PREFIX):]) for line in text()[offset:].splitlines()
                             if line.startswith(PREFIX)), None)
            value = wait(read, 'state ' + label)
            report['states'].append({'label': label, **value})
            return value

        def settled(label, condition=lambda s: True):
            return wait(lambda: (s if s.get('loaded') and s.get('settled') and not s.get('refreshing')
                                  and not s.get('pending') and condition(s) else None)
                        if (s := state(label)) else None, label)

        def hx(p):
            return p.hex()

        def name_hex(p):
            return os.path.basename(p).hex()

        def capture(label):
            raw.unlink(missing_ok=True)
            raw.with_suffix('.request').touch()
            def ready():
                if not raw.exists():
                    return False
                parts = raw.read_bytes().split(b'\n', 3)
                if len(parts) != 4:
                    return False
                try:
                    width, height = (int(x) for x in parts[1].split())
                except ValueError:
                    return False
                return len(parts[3]) == width * height * 3
            wait(ready, 'capture ' + label)
            image = base / (label + '.png')
            subprocess.run(['magick', str(raw), str(image)], env=env, check=True, timeout=15)
            report.setdefault('captures', []).append(str(image))

        def view(s):
            return {k: s.get(k) for k in ('path_raw', 'active', 'pane_id', 'tabs', 'selection_raw',
                                           'selected_raw', 'anchor_raw', 'properties')}

        wait(lambda: 'FILES_FILE_MANAGER_STATUS owned' in text(), 'service ownership')
        original_owner = wait(owner, 'bus owner')
        pid_result = call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                          'org.freedesktop.DBus.GetConnectionUnixProcessID', original_owner)
        assert pid_result.returncode == 0 and str(process.pid) in pid_result.stdout
        assert not clients(process.pid), 'service mapped a window before a valid request'
        for bad in ('https://example.invalid/a', 'file://remote-host/path', 'file:///bad%XX'):
            rpc('ShowItems', [bad], False)
        rpc('ShowItems', [uri(p) for p in fixtures[:1]] * 33, False)
        invalid = call(NAME, OBJECT, 'org.example.Invalid.ShowItems', '[]', '')
        assert invalid.returncode != 0
        invalid = subprocess.run(['dbus-send', '--session', '--print-reply', '--dest=' + NAME,
                                  OBJECT, NAME + '.ShowItems', 'string:wrong', 'string:'],
                                 env=env, capture_output=True, text=True, timeout=10)
        report['bad_signature'] = {'code': invalid.returncode, 'stdout': invalid.stdout, 'stderr': invalid.stderr}
        assert invalid.returncode != 0 and not clients(process.pid)
        report['checks'].append('invalid URI, overload, interface and signature produce no window')
        rpc('ShowItems', [uri(p) for p in fixtures[:2]])
        wait(lambda: 'FILES_UI_SMOKE_OK' in text() and clients(process.pid), 'first mapped window')
        window = clients(process.pid)
        assert len(window) == 1
        address = window[0]['address']
        if args.size == '1000x650':
            target = 'address:' + address
            quoted = json.dumps(target)
            for legacy, lua in (
                (['setfloating', target], f'hl.dsp.window.float({{action="on", window={quoted}}})'),
                (['resizewindowpixel', f'exact 1000 650,{target}'],
                 f'hl.dsp.window.resize({{x=1000, y=650, relative=false, window={quoted}}})'),
            ):
                result = harness.hypr(['dispatch', lua], nested).strip()
                if result.lower() != 'ok':
                    result = harness.hypr(['dispatch', *legacy], nested).strip()
                assert result.lower() == 'ok', (legacy, result)
            wait(lambda: clients(process.pid) and clients(process.pid)[0]['size'] == [1000, 650], 'window geometry')
        report['window_size'] = clients(process.pid)[0]['size']
        first = settled('same-parent', lambda s: set(s.get('selection_raw', [])) == {name_hex(p) for p in fixtures[:2]})
        assert first['path_raw'] == hx(os.fsencode(home / 'A'))
        capture('multiple-selection')
        report['checks'].append('same-parent batch selects both files')
        for p in fixtures[2:6]:
            rpc('ShowItems', [uri(p)])
            s = settled('bytes-' + hx(p), lambda s: name_hex(p) in s.get('selection_raw', []))
            if p.endswith(b'/.secret'):
                assert s['reveal_hidden'] and s['show_hidden_preference'] == first['show_hidden_preference']
        report['checks'].append('non-UTF-8, percent, newline and hidden filenames resolve byte-exactly')
        rpc('ShowItems', [uri(fixtures[0]), uri(fixtures[-1])])
        multi = settled('multiple-parents', lambda s: {hx(os.fsencode(home / 'A')), hx(os.fsencode(home / 'B'))}
                        <= {t['path_raw'] for t in s['tabs']})
        report['checks'].append('multiple parents use tabs')
        rpc('ShowFolders', [uri(odd_dir)])
        settled('raw-directory', lambda s: s['path_raw'] == hx(odd_dir))
        report['checks'].append('ShowFolders preserves non-UTF-8 directory')
        rpc('ShowItemProperties', [uri(fixtures[0]), uri(fixtures[1])])
        prop = settled('properties', lambda s: s.get('properties') and not s['properties'].get('pending'))['properties']
        assert prop['object_count'] == 2
        assert {bytes(x).hex() for x in prop['objects_raw']} == {hx(p) for p in fixtures[:2]}
        capture('multiple-properties')
        command('file-manager-prop-next')
        nxt = settled('property-next', lambda s: s.get('properties') and not s['properties'].get('pending')
                      and s['properties']['object_index'] != prop['object_index'])['properties']
        command('file-manager-prop-prev')
        settled('property-prev', lambda s: s.get('properties') and not s['properties'].get('pending')
                and s['properties']['object_index'] == prop['object_index'])
        command('file-manager-prop-close')
        settled('properties-closed', lambda s: not s.get('properties'))
        report['checks'].append('properties navigate the requested objects')
        rpc('ShowItemProperties', ['file:///'])
        root_prop = settled('root-properties', lambda s: s.get('properties') and not s['properties'].get('pending'))['properties']
        assert not root_prop['editable'] and root_prop['observed'] is not None, root_prop
        command('file-manager-prop-close')
        settled('root-properties-closed', lambda s: not s.get('properties'))
        report['checks'].append('filesystem root properties are read-only')
        duplicate_log = (base / 'duplicate.log').open('w'); handles.append(duplicate_log)
        duplicate = subprocess.Popen([str(binary), '--file-manager-service'], env=env,
                                     stdout=subprocess.DEVNULL, stderr=duplicate_log, start_new_session=True)
        duplicate.wait(timeout=15)
        assert duplicate.returncode != 0 and owner() == original_owner and not clients(duplicate.pid)
        report['checks'].append('duplicate service cannot replace owner or map a window')
        command('file-manager-protect')
        protected = state('protected-before')
        busy = rpc('ShowItems', [uri(fixtures[-1])], False)
        report['protected_error'] = busy.stderr
        command('file-manager-unprotect')
        after = settled('protected-after')
        assert view(after) == view(protected)
        time.sleep(1)
        assert view(settled('protected-no-replay')) == view(after)
        report['checks'].append('protected modal refuses immediately without deferred activation')
        before = settled('invalid-before')
        rpc('ShowItems', ['file:///bad%XX'], False)
        assert view(settled('invalid-after')) == view(before)
        for flag, operand in ((b'--select', fixtures[2]), (b'--show-folders', odd_dir),
                              (b'--properties', fixtures[0])):
            label = flag.decode().lstrip('-')
            cli_raw, cli_log = base / (label + '.ppm'), base / (label + '.log')
            cli_env = {**env, 'FILES_TEST_CAPTURE': str(cli_raw)}
            output = cli_log.open('w'); handles.append(output)
            cli = subprocess.Popen([os.fsencode(binary), b'--size', b'1000x650', b'--scale', b'1', b'--no-session',
                                    b'--ui-smoke', b'--exit-after', b'180', flag, b'--', operand], env=cli_env,
                                   stdout=subprocess.DEVNULL, stderr=output, start_new_session=True)
            wait(lambda: clients(cli.pid) and 'FILES_UI_SMOKE_OK' in cli_log.read_text(errors='replace'), 'CLI window')
            def cli_command(action):
                offset = len(cli_log.read_text(errors='replace'))
                pending = cli_raw.with_suffix('.pending'); pending.write_text(action)
                pending.replace(cli_raw.with_suffix('.popup-command'))
                wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in cli_log.read_text(errors='replace')[offset:],
                     'CLI command ' + action)
                return offset
            def cli_state():
                offset = cli_command('file-manager-state')
                return wait(lambda: next((json.loads(line[len(PREFIX):]) for line in
                                         cli_log.read_text(errors='replace')[offset:].splitlines()
                                         if line.startswith(PREFIX)), None), 'CLI state')
            def expected(s):
                if flag == b'--select':
                    return name_hex(operand) in s['selection_raw']
                if flag == b'--show-folders':
                    return s['path_raw'] == hx(operand)
                return bool(s.get('properties') and not s['properties']['pending'] and
                            bytes(s['properties']['current_raw']) == operand)
            cli_view = wait(lambda: (s if expected(s) else None) if (s := cli_state()) else None, 'CLI admission')
            report['states'].append({'label': 'cli-' + label, **cli_view})
            assert owner() == original_owner
            if flag == b'--properties':
                cli_command('file-manager-prop-close')
            cli_command('close')
            cli.wait(timeout=20)
            assert cli.returncode == 0 and 'FILES_CLOSE_DRAINED' in cli_log.read_text(errors='replace')
            report['checks'].append('CLI ' + flag.decode() + ' opens expected target without bus takeover')
        assert all(hashlib.sha256(open(p, 'rb').read()).hexdigest() == h for p, h in hashes.items())
        command('close')
        process.wait(timeout=20)
        assert process.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        assert owner() is None
        report['checks'].append('fixtures unchanged and service drains and releases its name')
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        report['error'] = repr(exc)
    finally:
        for child in (cli, process):
            if child is not None and child.poll() is None:
                stop_tree(child, None)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid, started in identities.items():
                if started is not None and start_time(pid) == started:
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
            deadline = time.monotonic() + 5
            while any(start_time(pid) == started for pid, started in identities.items() if started is not None) and time.monotonic() < deadline:
                time.sleep(.1)
        report['nested_stopped'] = bool(identities) and all(start_time(pid) != started for pid, started in identities.items())
        if not report['nested_stopped']:
            report['functional_gate'] = 'FAIL'
        for handle in handles:
            handle.close()
        (base / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(base / 'report.json'), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
