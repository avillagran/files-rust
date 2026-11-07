#!/usr/bin/env python3
"""Check FileManager1 presentation on a private nested Wayland session.

Uses owned windows and a private D-Bus session. A real private keyboard gesture
supplies a token through an anonymous pipe; it is never written to reports or
logs. Requested is recorded as a request, not a focus grant.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import shlex
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
STATE = 'FILES_FILE_MANAGER_STATE '


def started(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    ap.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    args = ap.parse_args()
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    if not args.private_child:
        directory = Path(tempfile.mkdtemp(prefix='presentation-bus-', dir=cache))
        config = directory / 'bus.conf'
        config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + escape(str(directory)) +
                          '</listen><auth>EXTERNAL</auth><policy context="default">'
                          '<allow send_destination="*"/><allow receive_sender="*"/>'
                          '<allow own="*"/></policy></busconfig>')
        env = dict(os.environ)
        env['FILES_TEST_PARENT_BUS'] = env.get('DBUS_SESSION_BUS_ADDRESS', '')
        env['FILES_TEST_BUS_DIR'] = str(directory)
        for key in ('DBUS_SESSION_BUS_ADDRESS', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
            env.pop(key, None)
        return subprocess.run(['dbus-run-session', '--config-file', str(config), '--', sys.executable,
                               str(Path(__file__).resolve()), '--private-child', '--binary', str(args.binary.resolve())],
                              env=env, check=False).returncode
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS', '')
    assert bus != os.environ.get('FILES_TEST_PARENT_BUS', '') and bus.startswith(
        'unix:path=' + os.environ.get('FILES_TEST_BUS_DIR', '<absent>') + '/'), 'refusing non-private bus'
    binary = args.binary.resolve(strict=True)
    base = Path(tempfile.mkdtemp(prefix='filemanager-presentation-', dir=cache))
    report = {'functional_gate': 'FAIL', 'fixture_dir': str(base), 'checks': [], 'states': [],
              'binary_path': str(binary), 'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED'}
    service = origin = compositor = token_origin = None
    identities = {}
    try:
        for name, field in (('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                            ('files-html-helper', 'html_helper_sha256')):
            report[field] = hashlib.sha256((binary if name == 'files' else binary.parent / name).read_bytes()).hexdigest()
        runtime = base / 'runtime'
        runtime.mkdir(mode=0o700)
        config = base / 'headless.conf'
        config.write_text('monitor = HEADLESS-1,1280x800@60,0x0,1\n'
                          'misc {\n    disable_hyprland_logo = true\n    disable_splash_rendering = true\n'
                          '    focus_on_activate = true\n}\n')
        compositor_env = {**os.environ, 'XDG_RUNTIME_DIR': str(runtime), 'HOME': str(base),
                          'XDG_CONFIG_HOME': str(base), 'AQ_BACKEND': 'headless', 'WLR_BACKENDS': 'headless',
                          'LIBGL_ALWAYS_SOFTWARE': '1', 'HYPRLAND_NO_SD_NOTIFY': '1', 'HYPRLAND_NO_SD_VARS': '1'}
        for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'HYPRLAND_INSTANCE_SIGNATURE', 'WAYLAND_DEBUG',
                    'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
            compositor_env.pop(key, None)
        with (base / 'compositor.log').open('w') as output:
            compositor = subprocess.Popen(['Hyprland', '--config', str(config)], env=compositor_env,
                                          stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        identity = started(compositor.pid)
        assert identity is not None, 'headless compositor did not start'
        identities = {compositor.pid: identity}
        deadline = time.monotonic() + 15
        match = []
        while time.monotonic() < deadline and compositor.poll() is None:
            try:
                match = json.loads(harness.hypr(['instances', '-j'], compositor_env))
            except (ValueError, subprocess.TimeoutExpired):
                match = []
            match = [item for item in match if (runtime / item.get('wl_socket', '')).is_socket()]
            if len(match) == 1:
                break
            time.sleep(.1)
        if len(match) != 1:
            report['headless_attempt'] = 'unavailable; see compositor.log'
            if compositor.poll() is None:
                stop_tree(compositor, None)
            short_root = Path('/var/tmp/files-filemanager')
            short_root.mkdir(mode=0o700, exist_ok=True)
            assert short_root.is_dir() and short_root.stat().st_uid == os.getuid()
            # Hyprland's IPC socket embeds a long instance signature; this
            # private 0700 root must stay short enough for AF_UNIX paths.
            runtime = short_root
            assert not list((runtime / 'hypr').glob('*/.socket.sock')), 'private runtime already in use'
            parent_runtime = Path(os.environ['XDG_RUNTIME_DIR'])
            parent_display = os.environ['WAYLAND_DISPLAY']
            if not os.path.isabs(parent_display):
                parent_display = str(parent_runtime / parent_display)
            compositor_env.update(XDG_RUNTIME_DIR=str(runtime), AQ_BACKEND='wayland', WAYLAND_DISPLAY=parent_display)
            compositor_env.pop('WLR_BACKENDS', None)
            assignments = [f'{key}={value}' for key, value in compositor_env.items()
                           if key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_RUNTIME_DIR', 'AQ_BACKEND',
                                      'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'LIBGL_ALWAYS_SOFTWARE',
                                      'HYPRLAND_NO_SD_NOTIFY', 'HYPRLAND_NO_SD_VARS')]
            command = shlex.join(['env', '-u', 'HYPRLAND_INSTANCE_SIGNATURE', '-u', 'DISPLAY', '-u', 'WAYLAND_DEBUG',
                                  '-u', 'XDG_ACTIVATION_TOKEN', '-u', 'DESKTOP_STARTUP_ID', *assignments,
                                  'Hyprland', '--config', str(config)]) + ' > ' + shlex.quote(str(base / 'nested.log')) + ' 2>&1'
            lua = f'hl.dsp.exec_cmd({json.dumps(command)}, {{ workspace = "special:files-present-check silent" }})'
            reply = harness.hypr(['dispatch', lua]).strip()
            if reply.lower() != 'ok':
                reply = harness.hypr(['dispatch', 'exec', '[workspace special:files-present-check silent] ' + command]).strip()
            assert reply.lower() == 'ok', 'cannot start private compositor on hidden workspace'
            deadline = time.monotonic() + 15
            match = []
            while time.monotonic() < deadline:
                try:
                    match = json.loads(harness.hypr(['instances', '-j'], compositor_env))
                except (ValueError, subprocess.TimeoutExpired):
                    match = []
                match = [item for item in match if (runtime / item.get('wl_socket', '')).is_socket()]
                if len(match) == 1:
                    break
                time.sleep(.1)
            assert len(match) == 1, 'private nested compositor unavailable; inspect nested.log'
            found = subprocess.run(['pgrep', '-x', '-f', f'Hyprland --config {config}'],
                                   capture_output=True, text=True, check=False).stdout.split()
            identities = {int(pid): started(int(pid)) for pid in found}
            assert identities and all(identities.values()), 'unidentified private compositor'
        socket, signature = match[0]['wl_socket'], match[0]['instance']
        assert signature != os.environ.get('HYPRLAND_INSTANCE_SIGNATURE')
        report['nested'] = {'socket': socket, 'pids': list(identities), 'runtime': str(runtime),
                            'backend': 'nested' if report.get('headless_attempt') else 'headless'}
        nested = {**compositor_env, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
        for key in ('DISPLAY', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
            nested.pop(key, None)
        # Set compositor policy only in this isolated instance. Lua-based and
        # legacy Hyprland use different configuration APIs.
        policy = harness.hypr(['eval', 'hl.config({ misc = { focus_on_activate = true } })'], nested).strip()
        if 'error' in policy.lower() or 'unknown' in policy.lower() or not policy:
            policy = harness.hypr(['keyword', 'misc:focus_on_activate', 'true'], nested).strip()
        option = harness.hypr(['getoption', 'misc:focus_on_activate', '-j'], nested)
        try:
            value = json.loads(option)
        except ValueError:
            raise AssertionError('cannot verify private activation policy') from None
        report['private_focus_on_activate'] = value
        assert value.get('int') == 1, 'private activation policy was not enabled'
        home = base / 'home'
        for name in ('Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'A', 'B'):
            (home / name).mkdir(parents=True)
        payload = home / 'A' / 'file.txt'
        payload.write_bytes(b'Private presentation fixture\n')
        for name in ('config', 'data', 'files-config', 'origin-config', 'origin-data', 'origin-files-config'):
            (base / name).mkdir()
        (base / 'devices.json').write_text('{"blockdevices": []}\n')
        (base / 'shell.toml').write_text('[font]\nbase-size = 14\n')
        raw, log = base / 'service.ppm', base / 'service.log'
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
               'FILES_TEST_DEVICES_JSON': str(base / 'devices.json'), 'FILES_CLOSE_TRACE': '1',
               'OMARCHY_FLTK_USER_SHELL': str(base / 'shell.toml'), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        with log.open('w') as out:
            service = subprocess.Popen([str(binary), '--file-manager-service', '--size', '900x600', '--scale', '1',
                                        '--no-session', '--ui-smoke', '--exit-after', '180'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)
        origin_raw, origin_log = base / 'origin.ppm', base / 'origin.log'
        origin_env = {**env, 'FILES_TEST_CAPTURE': str(origin_raw), 'XDG_CONFIG_HOME': str(base / 'origin-config'),
                      'XDG_DATA_HOME': str(base / 'origin-data'), 'FILES_CONFIG_DIR': str(base / 'origin-files-config')}
        with origin_log.open('w') as out:
            origin = subprocess.Popen([str(binary), str(home / 'B'), '--size', '900x600', '--scale', '1',
                                       '--no-session', '--ui-smoke', '--exit-after', '180'], env=origin_env,
                                      stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)

        def text():
            return log.read_text(errors='replace')

        def wait(fn, reason, timeout=20):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                value = fn()
                if value:
                    return value
                if service.poll() is not None or origin.poll() is not None:
                    raise AssertionError(reason + ': owned process exited')
                time.sleep(.04)
            raise AssertionError(reason)

        def clients(pid):
            return [c for c in json.loads(harness.hypr(['clients', '-j'], nested)) if c.get('pid') == pid]

        def active():
            current = json.loads(harness.hypr(['activewindow', '-j'], nested) or '{}')
            return current.get('pid')

        def focus_own(pid):
            client = wait(lambda: clients(pid)[0] if len(clients(pid)) == 1 else None, 'origin window')
            target = 'address:' + client['address']
            result = harness.hypr(['dispatch', f'hl.dsp.focus({{window={json.dumps(target)}}})'], nested).strip()
            if result.lower() != 'ok':
                result = harness.hypr(['dispatch', 'focuswindow', target], nested).strip()
            assert result.lower() == 'ok', 'cannot focus private origin'
            wait(lambda: active() == pid, 'origin focus')

        def origin_focus():
            focus_own(origin.pid)

        def send(action, capture=raw, source_log=log):
            offset = source_log.stat().st_size
            pending = capture.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(capture.with_suffix('.popup-command'))
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in source_log.read_text(errors='replace')[offset:], action)
            return offset

        def state(label):
            offset = send('file-manager-state')
            value = wait(lambda: next((json.loads(line[len(STATE):]) for line in text()[offset:].splitlines()
                                       if line.startswith(STATE)), None), 'state ' + label)
            # Record only the bridge's explicitly redacted marker fields.
            p = value.get('presentation') or {}
            last = p.get('last') or {}
            safe_last = {key: last.get(key) for key in
                         ('sequence', 'has_startup_id', 'visible', 'status', 'backend', 'error')}
            assert safe_last['status'] in (None, 'requested', 'granted', 'unsupported', 'refused', 'error',
                                           'queued', 'awaiting-paint', 'stale', 'expired', 'window-lost')
            assert safe_last['backend'] in (None, 'wayland', 'x11', 'windows', 'macos', 'other')
            assert safe_last['error'] is None or safe_last['error'] in (
                'wrong-thread', 'deleted-window', 'child-window', 'invalid-hint', 'busy', 'timed-out', 'native-error')
            report['states'].append({'label': label, 'presentation': {
                **{k: p.get(k) for k in ('pending', 'requests', 'attempts', 'superseded', 'skipped', 'errors')},
                'last': safe_last}})
            return value

        def presentation(label, previous=0):
            def match():
                s = state(label)
                p = s.get('presentation') or {}
                last = p.get('last') or {}
                return s if last.get('sequence', 0) > previous and not p.get('pending') else None
            return wait(match, 'presentation ' + label)

        uri = 'file://' + quote_from_bytes(os.fsencode(payload), safe='/')
        def request(hint, label, allow_close_disconnect=False):
            # Capture neither arguments nor native error text: either can contain a token.
            call = ['gdbus', 'call', '--session', '--dest', NAME, '--object-path', OBJECT,
                    '--method', NAME + '.ShowItems', json.dumps([uri]), hint]
            try:
                result = subprocess.run(call, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                        check=False, timeout=10)
            except subprocess.TimeoutExpired:
                raise AssertionError('FileManager1 admission timed out') from None
            report.setdefault('calls', []).append({'label': label, 'has_hint': bool(hint),
                                                   'hint_length': len(hint.encode()), 'returncode': result.returncode})
            assert result.returncode == 0 or allow_close_disconnect, 'FileManager1 admission failed'

        wait(lambda: 'FILES_FILE_MANAGER_STATUS owned' in text() and clients(origin.pid), 'private startup')
        assert not clients(service.pid)
        request('', 'initial-map')
        wait(lambda: clients(service.pid) and 'FILES_UI_SMOKE_OK' in text(), 'service mapped')
        initial = presentation('initial')
        first = initial['presentation']['last']
        assert first['visible'] and not first['has_startup_id']
        assert first['backend'] == 'wayland' and first['status'] == 'requested'
        report['checks'].append('explicit service maps after valid admission')
        origin_focus()
        sequence = initial['presentation']['last']['sequence']
        request('invalid-private-token', 'invalid')
        invalid = presentation('invalid', sequence)
        last = invalid['presentation']['last']
        assert last['status'] == 'requested' and last['backend'] == 'wayland' and last['has_startup_id'] and last['visible']
        time.sleep(.2)
        assert active() == origin.pid, 'invalid token changed focus'
        report['checks'].append('invalid token requested without forcing focus')
        # The origin uses a real private Wayland keyboard serial and sends its
        # opaque token only through an inherited anonymous pipe.
        protocols = Path('/usr/share/wayland-protocols')
        for label, xml in (('xdg-shell', protocols / 'stable/xdg-shell/xdg-shell.xml'),
                           ('xdg-activation', protocols / 'staging/xdg-activation/xdg-activation-v1.xml')):
            subprocess.run(['wayland-scanner', 'client-header', str(xml), str(base / (label + '-client.h'))], check=True)
            subprocess.run(['wayland-scanner', 'private-code', str(xml), str(base / (label + '-protocol.c'))], check=True)
        flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'wayland-client'], text=True).split()
        token_helper = base / 'token-origin'
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-I' + str(base), '-o', str(token_helper),
                        str(ROOT / 'scripts/tools/presentation-token-origin.c'),
                        str(base / 'xdg-shell-protocol.c'), str(base / 'xdg-activation-protocol.c'), *flags], check=True)
        fd, write_fd = os.pipe()
        try:
            token_origin = subprocess.Popen([str(token_helper), str(write_fd)], env=nested, pass_fds=(write_fd,),
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            os.close(write_fd)
            write_fd = -1
            focus_own(token_origin.pid)
            subprocess.run(['wtype', '-k', 'a'], env=nested, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=True, timeout=10)
            assert select.select([fd], [], [], 5)[0], 'private native token timeout'
            token = os.read(fd, 8192).decode()
            assert token and len(token.encode()) <= 4096, 'private native origin provided no bounded token'
            report['native_token'] = {'has_hint': True, 'hint_length': len(token.encode())}
            sequence = last['sequence']
            request(token, 'fresh-native')
        finally:
            os.close(fd)
            if write_fd >= 0:
                os.close(write_fd)
        fresh = presentation('fresh', sequence)
        last = fresh['presentation']['last']
        assert last['status'] == 'requested' and last['backend'] == 'wayland' and last['has_startup_id'] and last['visible']
        wait(lambda: active() == service.pid, 'fresh token did not activate owned service')
        report['checks'].append('fresh native gesture token requests and reaches owned service focus')
        origin_focus()
        sequence = last['sequence']
        request(token, 'reused-native')
        token = None
        reused = presentation('reused', sequence)
        last = reused['presentation']['last']
        assert last['status'] == 'requested' and last['backend'] == 'wayland' and last['has_startup_id'] and last['visible']
        time.sleep(.2)
        reused_focus = active()
        report['reused_token_focus'] = ('origin' if reused_focus == origin.pid else
                                        'service' if reused_focus == service.pid else 'other')
        report['checks'].append('reused token dispatched; compositor focus observation recorded')
        assert payload.read_bytes() == b'Private presentation fixture\n'
        send('close')
        service.wait(timeout=20)
        assert service.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        # A new service owns the same private bus only after the first exits.
        # The fixture invokes the actual close callback at the deferred map.
        log = base / 'close-on-map.log'
        close_env = {**env, 'FILES_TEST_CAPTURE': str(base / 'close-on-map.ppm'),
                     'FILES_TEST_PRESENTATION_CLOSE_ON_MAP': '1'}
        with log.open('w') as out:
            service = subprocess.Popen([str(binary), '--file-manager-service', '--size', '900x600', '--scale', '1',
                                        '--no-session', '--ui-smoke', '--exit-after', '180'], env=close_env,
                                       stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)
        wait(lambda: 'FILES_FILE_MANAGER_STATUS owned' in text(), 'close-on-map ownership')
        request('', 'close-on-map', allow_close_disconnect=True)
        service.wait(timeout=20)
        assert service.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        marker = next((json.loads(line[len('FILES_WINDOW_PRESENTATION_FINAL '):]) for line in text().splitlines()
                       if line.startswith('FILES_WINDOW_PRESENTATION_FINAL ')), None)
        assert marker and marker['requests'] == 1 and marker['attempts'] == 0 and marker['skipped'] == 1
        assert not marker['pending']
        report['close_on_map'] = {k: marker[k] for k in ('requests', 'attempts', 'skipped', 'pending')}
        report['checks'].append('close at first-map boundary skips presentation without replay')
        pending = origin_raw.with_suffix('.pending'); pending.write_text('close')
        pending.replace(origin_raw.with_suffix('.popup-command'))
        origin.wait(timeout=20)
        assert origin.returncode == 0 and 'FILES_CLOSE_DRAINED' in origin_log.read_text(errors='replace')
        report['checks'].append('owned windows close cleanly and fixture remains unchanged')
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        # Never stringify subprocess exceptions: their command can contain startup context.
        report['error'] = type(exc).__name__ + ': ' + (str(exc) if isinstance(exc, AssertionError) else 'see private logs')
    finally:
        for child in (token_origin, origin, service):
            if child is not None and child.poll() is None:
                stop_tree(child, None)
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
