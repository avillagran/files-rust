#!/usr/bin/env python3
"""Check asynchronous FileManager1 presentation with a selective private proxy.

Uses owned windows, a private D-Bus session and a selective Wayland proxy.
Token events remain private and are never written to reports or logs. Native
focus is arranged only for input liveness; this is not a focus-grant test.
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
import socket as socket_module
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


def policy_enabled(value):
    return value.get('bool') is True or (type(value.get('int')) is int and value['int'] == 1)


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
    service = origin = compositor = token_origin = proxy = None
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
        report['policy_eval_response'] = 'ok' if policy.lower() == 'ok' else ('empty' if not policy else 'other')
        if 'error' in policy.lower() or 'unknown' in policy.lower() or not policy:
            policy = harness.hypr(['keyword', 'misc:focus_on_activate', 'true'], nested).strip()
            report['policy_keyword_response'] = 'ok' if policy.lower() == 'ok' else ('empty' if not policy else 'other')
        option = harness.hypr(['getoption', 'misc:focus_on_activate', '-j'], nested)
        try:
            value = json.loads(option)
        except ValueError:
            raise AssertionError('cannot verify private activation policy') from None
        report['private_focus_on_activate'] = value
        assert policy_enabled(value), 'private activation policy was not enabled'
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
        proxy_path, control_path = runtime / 'files-present-proxy', runtime / 'files-present-control'
        with (base / 'proxy.log').open('w') as output:
            proxy = subprocess.Popen([sys.executable, str(ROOT / 'scripts/tools/wayland-activation-proxy.py'),
                                      '--listen', str(proxy_path), '--upstream', str(runtime / socket),
                                      '--control', str(control_path)], env=nested, stdout=output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and proxy.poll() is None and not control_path.is_socket():
            time.sleep(.04)
        assert proxy.poll() is None and proxy_path.is_socket() and control_path.is_socket(), 'private proxy startup'
        def proxy_command(command):
            with socket_module.socket(socket_module.AF_UNIX) as control:
                control.settimeout(3)
                control.connect(str(control_path))
                control.sendall((command + '\n').encode())
                return json.loads(control.recv(1024))
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
               'FILES_TEST_DEVICES_JSON': str(base / 'devices.json'), 'FILES_CLOSE_TRACE': '1',
               'OMARCHY_FLTK_ANIM': 'on',
               'OMARCHY_FLTK_USER_SHELL': str(base / 'shell.toml'), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        env['WAYLAND_DISPLAY'] = str(proxy_path)
        with log.open('w') as out:
            service = subprocess.Popen([str(binary), '--file-manager-service', '--size', '900x600', '--scale', '1',
                                        '--no-session', '--ui-smoke', '--exit-after', '180'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)
        origin_raw, origin_log = base / 'origin.ppm', base / 'origin.log'
        origin_env = {**env, 'WAYLAND_DISPLAY': socket, 'FILES_TEST_CAPTURE': str(origin_raw), 'XDG_CONFIG_HOME': str(base / 'origin-config'),
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
            # The compositor sees SO_PEERCRED for the proxy, not Files.
            # Only the service's sole private proxied connection uses it.
            observed_pid = proxy.pid if pid == service.pid else pid
            result = [c for c in json.loads(harness.hypr(['clients', '-j'], nested))
                      if c.get('pid') == observed_pid]
            if pid == service.pid:
                assert len(result) <= 1 and all('files' in str(c.get('class', '')).lower() for c in result)
            return result

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
            expected_pid = proxy.pid if pid == service.pid else pid
            wait(lambda: active() == expected_pid, 'origin focus')

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
                                           'queued', 'awaiting-paint', 'pending-native', 'stale', 'expired', 'window-lost')
            assert safe_last['backend'] in (None, 'wayland', 'x11', 'windows', 'macos', 'other')
            assert safe_last['error'] is None or safe_last['error'] in (
                'wrong-thread', 'deleted-window', 'child-window', 'invalid-hint', 'busy', 'timed-out', 'native-error')
            report['states'].append({'label': label, 'presentation': {
                **{k: p.get(k) for k in ('pending', 'pending_stage', 'native_polls', 'requests', 'attempts', 'dispatches', 'cancelled_native', 'superseded', 'skipped', 'errors')},
                'last': safe_last}, 'ui': {k: (value.get('ui') or {}).get(k) for k in ('pump_revision', 'paint_revision', 'native_key_events', 'sidebar_width')}})
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
        wait(lambda: (s if s.get('loaded') and s.get('settled') and not s.get('refreshing')
                      and not s['presentation']['pending'] else None)
             if (s := state('initial-settled')) else None, 'initial listing settled')
        focus_own(service.pid)  # Input setup only; this is not an activation assertion.
        report['scope'] = ('private Wayland proxy delays token.done; native input and UI progress while pending; '
                           'compositor sees proxy credentials; no activation-grant or production PID identity claim')
        report['service_process_pid'] = service.pid
        report['proxy_process_pid'] = proxy.pid

        def wait_held(label):
            wait(lambda: proxy_command('status').get('held'), label + ' token event held', timeout=4)
            current = state(label)
            p = current['presentation']
            assert p['pending'] and p['pending_stage'] == 'native' and p['last']['status'] == 'pending-native', p
            return current

        def arm_request(label):
            assert proxy_command('arm').get('armed')
            request('', label)
            return wait_held(label)

        def require_pending(label):
            current = state(label)
            p = current['presentation']
            assert p['pending'] and p['pending_stage'] == 'native' and p['last']['status'] == 'pending-native', p
            assert proxy_command('status')['held'], 'delayed token lost before release'
            return current

        tools = ROOT / 'scripts/tools'
        xml = tools / 'wlr-virtual-pointer-unstable-v1.xml'
        header, code = base / 'wlr-virtual-pointer-unstable-v1-client.h', base / 'wlr-pointer.c'
        subprocess.run(['wayland-scanner', 'client-header', str(xml), str(header)], check=True)
        subprocess.run(['wayland-scanner', 'private-code', str(xml), str(code)], check=True)
        flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'wayland-client'], text=True).split()
        hover = base / 'private-hover'
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-I' + str(base), '-o', str(hover),
                        str(tools / 'private-hover.c'), str(code), *flags], check=True)
        def move_pointer(local_x, local_y):
            client = wait(lambda: clients(service.pid)[0] if len(clients(service.pid)) == 1 else None, 'service window')
            monitors = json.loads(harness.hypr(['monitors', '-j'], nested))
            assert len(monitors) == 1 and client['monitor'] == monitors[0]['id']
            monitor = monitors[0]
            assert [monitor.get('x', 0), monitor.get('y', 0)] == [0, 0]
            width, height = round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale'])
            x, y = client['at'][0] + local_x, client['at'][1] + local_y
            assert 0 <= x < width and 0 <= y < height, 'hover target outside private monitor'
            subprocess.run([str(hover), str(runtime / socket), str(width), str(height), str(x), str(y)],
                           env=nested, check=True, timeout=3)

        before = arm_request('liveness')
        assert before['ui']['sidebar_width'] == 52, 'sidebar did not start collapsed'
        subprocess.run(['wtype', '-k', 'F12'], env=nested, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=True, timeout=3)
        def progressed():
            s = require_pending('liveness-progress')
            ui, old = s['ui'], before['ui']
            return s if (s['presentation']['native_polls'] > before['presentation']['native_polls'] and
                         ui['pump_revision'] > old['pump_revision'] and ui['paint_revision'] > old['paint_revision'] and
                         ui['native_key_events'] > old['native_key_events']) else None
        after = wait(progressed, 'native input, UI pump and paint progress', timeout=1.5)
        move_pointer(20, 180)
        def animated():
            s = require_pending('sidebar-animated')
            return s if (52 < s['ui']['sidebar_width'] < 170 and
                         s['ui']['paint_revision'] > before['ui']['paint_revision']) else None
        animated_state = wait(animated, 'sidebar hover animation advances', timeout=1.3)
        def expanded():
            s = require_pending('sidebar-expanded')
            return s if (s['ui']['sidebar_width'] == 170 and
                         s['ui']['paint_revision'] > animated_state['ui']['paint_revision']) else None
        wait(expanded, 'sidebar hover animation reaches endpoint', timeout=.8)
        proxy_command('release')
        wait(lambda: not state('liveness-completed')['presentation']['pending'], 'release completion', timeout=4)
        report['checks'].append('native input, UI pump, paint, polling and sidebar animation advance while token reply is held')

        before = arm_request('modal-cancel')
        send('file-manager-protect')
        def cancelled():
            s = state('modal-cancelled')
            return s if not s['presentation']['pending'] and s['presentation']['cancelled_native'] > before['presentation']['cancelled_native'] else None
        cancelled_state = wait(cancelled, 'modal cancels pending request', timeout=1.5)
        dispatches = cancelled_state['presentation']['dispatches']
        proxy_command('release')
        time.sleep(.15)
        after = state('modal-late-reply')
        assert after['presentation']['dispatches'] == dispatches and not after['presentation']['pending']
        send('file-manager-unprotect')
        report['checks'].append('modal cancels pending native request; delayed reply cannot dispatch')

        before = arm_request('navigation-cancel')
        subprocess.run(['wtype', '-M', 'ctrl', '-k', 'l', '-m', 'ctrl'], env=nested, check=True, timeout=3)
        subprocess.run(['wtype', str(home / 'B')], env=nested, check=True, timeout=3)
        subprocess.run(['wtype', '-k', 'Return'], env=nested, check=True, timeout=3)
        def navigation_cancelled():
            s = state('navigation-cancelled')
            return s if not s['presentation']['pending'] and s['presentation']['cancelled_native'] > before['presentation']['cancelled_native'] else None
        cancelled_state = wait(navigation_cancelled, 'navigation cancels pending request', timeout=1.5)
        dispatches = cancelled_state['presentation']['dispatches']
        proxy_command('release'); time.sleep(.15)
        assert state('navigation-late-reply')['presentation']['dispatches'] == dispatches
        report['checks'].append('navigation cancels pending native request; delayed reply cannot dispatch')

        # Return to the known fixture path using an explicit invalid token, then
        # replace a held request with a later real D-Bus call.
        request('invalid-private-token', 'return-to-A')
        wait(lambda: (s if not s['presentation']['pending'] and s.get('loaded') and s.get('settled')
                      and not s.get('refreshing') and s.get('path_raw') == os.fsencode(home / 'A').hex() else None)
             if (s := state('return-to-A')) else None, 'return to settled fixture')
        before = arm_request('replacement-old')
        request('invalid-private-token', 'replacement-new')
        def replacement_cancelled():
            s = state('replacement-cancelled')
            return s if (s['presentation']['cancelled_native'] > before['presentation']['cancelled_native']
                         and not s['presentation']['pending']) else None
        replacement = wait(replacement_cancelled, 'replacement cancels pending request', timeout=1.5)
        dispatches = replacement['presentation']['dispatches']
        proxy_command('release'); time.sleep(.15)
        after = state('replacement-late-reply')
        assert after['presentation']['dispatches'] == dispatches and not after['presentation']['pending']
        report['checks'].append('replacement cancels older native request without replaying its delayed reply')

        before = arm_request('close-cancel')
        send('close')
        # Release immediately after the real callback while the private
        # connection remains alive; disconnect itself is also safe.
        try:
            proxy_command('release')
        except (OSError, ValueError):
            pass
        service.wait(timeout=20)
        assert service.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        final = next((json.loads(line[len('FILES_WINDOW_PRESENTATION_FINAL '):]) for line in text().splitlines()
                      if line.startswith('FILES_WINDOW_PRESENTATION_FINAL ')), None)
        assert final and not final['pending'] and final['cancelled_native'] > before['presentation']['cancelled_native']
        assert final['dispatches'] == before['presentation']['dispatches']
        report['close_final'] = {k: final[k] for k in ('pending', 'cancelled_native', 'dispatches')}
        pending = origin_raw.with_suffix('.pending'); pending.write_text('close')
        pending.replace(origin_raw.with_suffix('.popup-command'))
        origin.wait(timeout=20)
        assert origin.returncode == 0 and 'FILES_CLOSE_DRAINED' in origin_log.read_text(errors='replace')
        assert payload.read_bytes() == b'Private presentation fixture\n'
        report['checks'].append('close cancels pending native request and drains without replay')
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        # Never stringify subprocess exceptions: their command can contain startup context.
        report['error'] = type(exc).__name__ + ': ' + (str(exc) if isinstance(exc, AssertionError) else 'see private logs')
    finally:
        for child in (token_origin, origin, service, proxy):
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
