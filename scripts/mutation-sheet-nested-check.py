#!/usr/bin/env python3
"""Exercise mutation Sheets with native input in an isolated Wayland session.

Private filesystem fixtures, compositor, session bus and evidence are retained.
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
STATE = 'FILES_MUTATION_SHEET_STATE '


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
    ap.add_argument('--size', choices=('1000x650', '600x650'), default='1000x650')
    ap.add_argument('--animations', choices=('on', 'off'), default='on')
    ap.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    args = ap.parse_args()
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    if not args.private_child:
        directory = Path(tempfile.mkdtemp(prefix='mutation-sheet-bus-', dir=cache))
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
                               str(Path(__file__).resolve()), '--private-child', '--binary', str(args.binary.resolve()),
                               '--size', args.size, '--animations', args.animations],
                              env=env, check=False).returncode
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS', '')
    assert bus != os.environ.get('FILES_TEST_PARENT_BUS', '') and bus.startswith(
        'unix:path=' + os.environ.get('FILES_TEST_BUS_DIR', '<absent>') + '/'), 'refusing non-private bus'
    binary = args.binary.resolve(strict=True)
    base = Path(tempfile.mkdtemp(prefix='mutation-sheet-native-', dir=cache))
    report = {'functional_gate': 'FAIL', 'fixture_dir': str(base), 'checks': [], 'states': [],
              'binary_path': str(binary), 'requested_size': args.size, 'animations': args.animations,
              'captures': [], 'animation_comparison': 'NOT_COMPARED',
              'scope': 'native input and real private mutation fixtures; this profile alone does not prove cross-profile motion',
              'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED'}
    service = origin = compositor = None
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
        payload = home / 'A' / 'photo1.txt'
        payload.write_bytes(b'Private first fixture\n')
        (home / 'A' / 'photo2.txt').write_bytes(b'Private second fixture\n')
        (home / 'A' / 'percent%name.txt').write_bytes(b'Private percent fixture\n')
        with open(os.fsencode(home / 'A') + b'/raw-\xff\nname.txt', 'wb') as f:
            f.write(b'Private raw fixture\n')
        gate = base / 'worker-gate'
        gate.mkdir(mode=0o700)
        for name in ('config', 'data', 'files-config', 'origin-config', 'origin-data', 'origin-files-config'):
            (base / name).mkdir()
        (base / 'devices.json').write_text('{"blockdevices": []}\n')
        (base / 'shell.toml').write_text('[font]\nbase-size = 14\n')
        raw, log = base / 'service.ppm', base / 'service.log'
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
               'FILES_TEST_MUTATION_GATE_DIR': str(gate), 'FILES_TEST_MUTATIONS': '1',
               'FILES_METRICS_READY': '1',
               'FILES_TEST_DEVICES_JSON': str(base / 'devices.json'), 'FILES_CLOSE_TRACE': '1',
               'OMARCHY_FLTK_ANIM': args.animations,
               'OMARCHY_FLTK_USER_SHELL': str(base / 'shell.toml'), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        with log.open('w') as out:
            service = subprocess.Popen([str(binary), str(home / 'A'), '--size', args.size, '--scale', '1',
                                        '--no-session', '--ui-smoke', '--ui-scene', 'rename', '--exit-after', '180'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)

        def text():
            return log.read_text(errors='replace')

        def wait(fn, reason, timeout=20):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                value = fn()
                if value:
                    return value
                if service.poll() is not None:
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

        def send(action, capture=raw, source_log=log):
            offset = len(source_log.read_text(errors='replace'))
            pending = capture.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(capture.with_suffix('.command'))
            wait(lambda: f'FILES_MUTATION_COMMAND_OK {action}\n' in source_log.read_text(errors='replace')[offset:], action)
            return offset

        def send_popup(action):
            offset = len(text())
            pending = raw.with_suffix('.pending')
            pending.write_text(action)
            pending.replace(raw.with_suffix('.popup-command'))
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in text()[offset:], action)

        def state(label):
            offset = send('mutation-sheet-state')
            value = wait(lambda: next((json.loads(line[len(STATE):]) for line in text()[offset:].splitlines()
                                       if line.startswith(STATE)), None), 'sheet state ' + label)
            report['states'].append({'label': label, **value})
            return value

        def key(*keys):
            subprocess.run(['wtype', *keys], env=nested, check=True, timeout=10)

        def clipboard(text_value):
            # The nested socket is explicit: never access the user's clipboard.
            subprocess.run(['wl-copy'], input=text_value, text=True, env=nested, check=True, timeout=10)

        def capture(label):
            raw.unlink(missing_ok=True)
            raw.with_suffix('.request').touch()
            def complete():
                if not raw.exists(): return False
                parts = raw.read_bytes().split(b'\n', 3)
                if len(parts) != 4: return False
                try: w, h = map(int, parts[1].split())
                except ValueError: return False
                return len(parts[3]) == w * h * 3
            wait(complete, 'frame ' + label)
            path = base / (label + '.png')
            subprocess.run(['magick', str(raw), str(path)], env=env, check=True, timeout=15)
            report['captures'].append(str(path))

        tools = ROOT / 'scripts/tools'
        xml = tools / 'wlr-virtual-pointer-unstable-v1.xml'
        header, generated = base / 'wlr-virtual-pointer-unstable-v1-client.h', base / 'pointer-protocol.c'
        subprocess.run(['wayland-scanner', 'client-header', str(xml), str(header)], check=True)
        subprocess.run(['wayland-scanner', 'private-code', str(xml), str(generated)], check=True)
        flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'wayland-client'], text=True).split()
        pointer = base / 'private-sheet-pointer'
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-I' + str(base), '-o', str(pointer),
                        str(tools / 'private-sheet-pointer.c'), str(generated), *flags], check=True)

        def click(rect):
            assert rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0
            client = wait(lambda: clients(service.pid)[0] if len(clients(service.pid)) == 1 else None, 'owned window')
            monitors = json.loads(harness.hypr(['monitors', '-j'], nested))
            assert len(monitors) == 1 and client['monitor'] == monitors[0]['id']
            monitor = monitors[0]
            assert [monitor.get('x', 0), monitor.get('y', 0)] == [0, 0]
            width, height = round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale'])
            x, y = client['at'][0] + rect[0] + rect[2] // 2, client['at'][1] + rect[1] + rect[3] // 2
            assert 0 <= x < width and 0 <= y < height, 'refusing click outside private monitor'
            subprocess.run([str(pointer), str(runtime / socket), str(width), str(height), str(x), str(y), 'click'],
                           env=nested, check=True, timeout=10)

        def click_outside(sheet_state):
            sx, sy, sw, sh = sheet_state['rect']
            client = clients(service.pid)[0]
            width, height = client['size']
            points = [(4, 4), (width - 5, 4), (4, height - 5), (width - 5, height - 5)]
            point = next(((x, y) for x, y in points if not (sx <= x < sx + sw and sy <= y < sy + sh)), None)
            assert point, 'no outside point in the private window'
            click([point[0], point[1], 1, 1])

        wait(lambda: 'FILES_UI_SMOKE_OK' in text() and clients(service.pid), 'private window startup')
        wait(lambda: 'FILES_SCENE_READY view=groups loaded=true' in text(), 'private listing loaded')
        if args.size == '1000x650':
            address = clients(service.pid)[0]['address']
            target = 'address:' + address
            quoted = json.dumps(target)
            for legacy, lua in ((['setfloating', target], f'hl.dsp.window.float({{action="on", window={quoted}}})'),
                                (['resizewindowpixel', 'exact 1000 650,' + target],
                                 f'hl.dsp.window.resize({{x=1000, y=650, relative=false, window={quoted}}})')):
                result = harness.hypr(['dispatch', lua], nested).strip()
                if result.lower() != 'ok': result = harness.hypr(['dispatch', *legacy], nested).strip()
                assert result.lower() == 'ok', 'private window resize failed'
            wait(lambda: clients(service.pid) and clients(service.pid)[0]['size'] == [1000, 650], 'wide window size')
        report['window_size'] = clients(service.pid)[0]['size']
        focus_own(service.pid)
        capture('raw-and-percent-listing')

        def sheet(label, predicate=lambda s: True):
            return wait(lambda: (s if predicate(s) else None) if (s := state(label)) else None, label)

        def open_sheet(command):
            send(command)
            value = sheet(command, lambda s: s.get('open') and s.get('built') and s.get('geometry'))
            assert not value['model']['legacy_canvas_dialog_enabled']
            geometry = value['geometry']
            count = 5 if value['model']['mode'] == 'rename' else 1
            assert geometry['control_count']['fields'] == count and len(geometry['fields']) == count
            assert geometry['control_count']['preview'] == (1 if count == 5 else 0)
            return value

        def set_field(value):
            clipboard(value)
            key('-M', 'ctrl', '-k', 'a', '-m', 'ctrl')
            key('-M', 'ctrl', '-k', 'v', '-m', 'ctrl')
            return sheet('field-value', lambda s: s.get('geometry') and s['geometry']['values'][0] == value)

        def closed(label):
            return sheet(label, lambda s: not s.get('open'))

        def arm(phase):
            (gate / (phase + '.arm')).touch(mode=0o600)

        def held(phase):
            matches = wait(lambda: list(gate.glob(phase + '-*.waiting')), phase + ' worker gate')
            assert len(matches) == 1
            (gate / (phase + '.arm')).unlink()
            prefix = matches[0].name.removesuffix('.waiting')
            return prefix

        def release(prefix):
            (gate / (prefix + '.release')).touch(mode=0o600)

        # Dismiss and reopen must not preserve a stale draft or native undo state.
        send_popup('operation-notice-clock-freeze')
        first = open_sheet('create-file')
        capture('open-start')
        send_popup('operation-notice-clock-step:80')
        capture('open-middle')
        send_popup('operation-notice-clock-step:1000')
        capture('open-settled')
        send_popup('operation-notice-clock-resume')
        assert first['builds'] == 1 and first['dismissible'] and first['geometry']['focus_field'] == 0
        set_field('cover-draft.txt')
        key('-k', 'Home')
        key('-k', 'Right')
        key('-M', 'shift', '-k', 'Right', '-m', 'shift')
        selected = sheet('cover-selection', lambda s: s['geometry']['focus_field'] == 0 and
                         s['geometry']['positions'][0] != s['geometry']['marks'][0])
        caret = (selected['geometry']['positions'][0], selected['geometry']['marks'][0])
        send_popup('operation-notice-clock-freeze')
        send('mutation-sheet-cover')
        covered = sheet('warm-covered', lambda s: not s['open'] and s['built'])
        assert covered['epoch'] == first['epoch'] and covered['builds'] == 1
        capture('cover-start')
        send_popup('operation-notice-clock-step:80')
        capture('cover-middle')
        key('-k', 'Escape')
        warm = sheet('warm-return', lambda s: s['open'] and s['epoch'] == first['epoch'] and
                     s['geometry'] and s['geometry']['focus_field'] == 0)
        assert warm['builds'] == 1 and warm['geometry']['values'][0] == 'cover-draft.txt'
        assert (warm['geometry']['positions'][0], warm['geometry']['marks'][0]) == caret
        capture('cover-return-start')
        send_popup('operation-notice-clock-step:80')
        capture('cover-return-middle')
        send_popup('operation-notice-clock-step:1000')
        capture('cover-return-settled')
        send('mutation-sheet-cover')
        sheet('cold-covered', lambda s: not s['open'] and s['built'] and s['epoch'] == first['epoch'])
        for _ in range(31):
            send_popup('operation-notice-clock-step:1000')
        released = sheet('cold-released', lambda s: not s['open'] and not s['built'] and s['geometry'] is None)
        assert released['epoch'] == first['epoch'] and released['builds'] == 1
        key('-k', 'Escape')
        cold = sheet('cold-return', lambda s: s['open'] and s['built'] and s['epoch'] == first['epoch'] and
                     s['geometry'] and s['geometry']['focus_field'] == 0)
        assert cold['builds'] == 2 and cold['geometry']['values'][0] == 'cover-draft.txt'
        assert (cold['geometry']['positions'][0], cold['geometry']['marks'][0]) == caret
        send_popup('operation-notice-clock-step:1000')
        send_popup('operation-notice-clock-resume')
        report['checks'].append('same epoch preserves draft and selection after warm cover and 30-second cold rebuild')
        for _ in range(6):
            key('-k', 'Tab')
            trapped = state('tab-trap')
            assert trapped['open'] and not trapped['model']['browser_focus']
        send_popup('operation-notice-clock-freeze')
        key('-k', 'Escape')
        closed('tab-trap-escaped')
        capture('close-start')
        send_popup('operation-notice-clock-step:80')
        capture('close-middle')
        wait(lambda: state('browser-focus-after-escape')['model']['browser_focus'], 'focus returns to browser')
        report['checks'].append('native Tab stays within Sheet; Escape returns focus to browser')
        first = open_sheet('create-file')
        capture('reopen-start')
        send_popup('operation-notice-clock-step:80')
        capture('reopen-middle')
        send_popup('operation-notice-clock-step:1000')
        capture('reopen-settled')
        send_popup('operation-notice-clock-resume')
        if args.size == '600x650':
            assert first.get('close')
            click(first['close'])
            closed('closed-with-X')
            outside = open_sheet('create-file')
            click_outside(outside)
            closed('closed-outside')
            first = open_sheet('create-file')
            report['checks'].append('native close button and outside click dismiss an idle Sheet')
        set_field('draft-only.txt')
        capture('create-draft')
        key('-k', 'Escape')
        closed('create-escaped')
        reopened = open_sheet('create-file')
        assert reopened['epoch'] != first['epoch'] and reopened['builds'] == 1
        assert reopened['geometry']['values'][0] != 'draft-only.txt'
        report['checks'].append('Escape restores browser and a new opening does not reuse a stale draft')

        # Clipboard stays within the nested session; edit and undo use TextField.
        created_name = 'café%report.txt'
        set_field(created_name)
        key('-k', 'End')
        key('x')
        sheet('typed-suffix', lambda s: s.get('geometry') and s['geometry']['values'][0] == created_name + 'x')
        key('-M', 'ctrl', '-k', 'z', '-m', 'ctrl')
        sheet('native-undo', lambda s: s.get('geometry') and s['geometry']['values'][0] in (created_name, 'created.txt'))
        set_field(created_name)
        capture('create-ready')
        arm('create')
        key('-k', 'Return')
        ticket = held('create')
        applying = sheet('create-applying', lambda s: s.get('applying') and not s.get('dismissible'))
        assert not applying['geometry']['apply_enabled'] and not any(applying['geometry']['field_enabled'])
        key('-k', 'Escape')
        still = sheet('create-escape-blocked', lambda s: s.get('open') and s.get('applying'))
        assert still['epoch'] == applying['epoch'] and not (home / 'A' / created_name).exists()
        if args.size == '600x650' and still.get('close'):
            click(still['close'])
            sheet('create-close-blocked', lambda s: s.get('open') and s.get('applying'))
            click_outside(still)
            sheet('create-outside-blocked', lambda s: s.get('open') and s.get('applying'))
        release(ticket)
        wait(lambda: (home / 'A' / created_name).is_file(), 'create file ACK')
        assert (home / 'A' / created_name).read_bytes() == b''
        closed('created-file')
        report['checks'].append('native Unicode paste and undo; create remains visible during Apply until ACK')

        folder = 'Carpeta%ñ'
        open_sheet('create-folder')
        set_field(folder)
        key('-k', 'Return')
        wait(lambda: (home / 'A' / folder).is_dir(), 'create folder ACK')
        closed('created-folder')
        report['checks'].append('native create folder preserves Unicode and percent basename')

        open_sheet('create-file')
        set_field('photo1.txt')
        before_conflict = len(text())
        key('-k', 'Return')
        wait(lambda: any('FILES_MUTATION_FINISHED id=' in line and 'success=false' in line
                         for line in text()[before_conflict:].splitlines()), 'conflict ACK')
        conflict = sheet('create-conflict', lambda s: bool(s['model']['notice_open']) or bool(s.get('error')))
        assert (home / 'A' / 'photo1.txt').read_bytes() == b'Private first fixture\n'
        capture('create-conflict')
        if conflict['model']['notice_open']:
            key('-k', 'Escape')
        report['checks'].append('conflicting create reports failure without changing the existing file')

        raw_name = os.fsencode(home / 'A') + b'/raw-\xff\nname.txt'
        open_sheet('rename-raw')
        rejected = sheet('raw-name-rejected', lambda s: s.get('open') and not s.get('planning') and
                         bool(s.get('error')) and not s.get('can_apply'))
        assert 'UTF-8' in rejected['error'] and not rejected['geometry']['apply_enabled']
        with open(raw_name, 'rb') as f:
            assert f.read() == b'Private raw fixture\n'
        capture('raw-name-rejected')
        key('-k', 'Escape')
        closed('raw-name-cancelled')
        report['checks'].append('raw non-UTF-8 basename is reported and preserved without lossy rename')

        # Hold an already-computed Plan A, cancel its dialog, open B natively,
        # then deliver A. The new dialog must only accept its own fresh plan.
        arm('ack-plan')
        stale = open_sheet('rename-single')
        old_ticket = held('ack-plan')
        set_field('old-{name}{dotext}')
        key('-k', 'Escape')
        closed('old-plan-cancelled')
        key('-k', 'F2')
        newer = sheet('reopened-rename', lambda s: s.get('open') and s.get('epoch') != stale['epoch'])
        set_field('fresh-{name}{dotext}')
        release(old_ticket)
        fresh = sheet('fresh-plan-after-stale-ack', lambda s: s.get('open') and not s.get('planning') and
                      s.get('can_apply') and s.get('preview_total') == 1 and
                      s.get('geometry', {}).get('values', [''])[0] == 'fresh-{name}{dotext}')
        assert fresh['epoch'] == newer['epoch'] and (home / 'A' / 'photo1.txt').read_bytes() == b'Private first fixture\n'
        key('-k', 'Escape')
        closed('fresh-plan-cancelled')
        report['checks'].append('a delayed old Plan ACK cannot replace the reopened dialog plan')

        # Fixture selects the private source; editing and Apply are native.
        open_sheet('rename-single')
        set_field('single{dotext}')
        sheet('single-plan', lambda s: not s.get('planning') and s.get('can_apply') and s.get('preview_total') == 1)
        capture('single-preview')
        key('-k', 'Return')
        wait(lambda: (home / 'A' / 'single.txt').is_file(), 'single rename ACK')
        assert (home / 'A' / 'single.txt').read_bytes() == b'Private first fixture\n'
        key('-M', 'ctrl', '-k', 'z', '-m', 'ctrl')
        wait(lambda: (home / 'A' / 'photo1.txt').is_file() and not (home / 'A' / 'single.txt').exists(), 'undo single rename')
        sheet('undo-ack', lambda s: not s['model']['mutation_busy'] and not s['model']['mutation_applied'])
        report['checks'].append('single rename plan/apply and native undo preserve bytes')

        open_sheet('rename-preview')
        set_field('{name}-batch{dotext}')
        plan = sheet('bulk-plan', lambda s: not s.get('planning') and s.get('can_apply') and s.get('preview_total') == 2)
        capture('bulk-preview')
        arm('ack-apply')
        key('-k', 'Return')
        apply_ticket = held('ack-apply')
        held_apply = sheet('bulk-apply-held', lambda s: s['open'] and s['applying'] and s['geometry']['focus_cancel'])
        assert held_apply['preview_total'] == plan['preview_total'] and held_apply['preview_rows'] == plan['preview_rows']
        assert not held_apply['geometry']['apply_enabled'] and not any(held_apply['geometry']['field_enabled'])
        capture('bulk-apply-held')
        release(apply_ticket)
        wait(lambda: (home / 'A' / 'photo1-batch.txt').is_file() and
             (home / 'A' / 'photo2-batch.txt').is_file(), 'bulk rename ACK')
        assert (home / 'A' / 'photo1-batch.txt').read_bytes() == b'Private first fixture\n'
        assert (home / 'A' / 'photo2-batch.txt').read_bytes() == b'Private second fixture\n'
        assert (home / 'A' / 'percent%name.txt').read_bytes() == b'Private percent fixture\n'
        with open(os.fsencode(home / 'A') + b'/raw-\xff\nname.txt', 'rb') as f:
            assert f.read() == b'Private raw fixture\n'
        report['checks'].append('bulk apply retains reviewed rows and focuses Cancel until ACK; preserves each file')

        # Hold a real Create ACK after the filesystem operation, then close the
        # owned top-level window. The Sheet must remain until the worker drains.
        sheet('bulk-ack', lambda s: not s['model']['mutation_busy'] and not s['model']['mutation_applied'])
        open_sheet('create-file')
        set_field('owner-close.txt')
        arm('ack-create')
        key('-k', 'Return')
        ticket = held('ack-create')
        assert (home / 'A' / 'owner-close.txt').is_file()
        before_close = sheet('owner-close-held', lambda s: s['open'] and s['applying'])
        window = clients(service.pid)
        assert len(window) == 1
        target = 'address:' + window[0]['address']
        reply = harness.hypr(['dispatch', f'hl.dsp.window.close({{window={json.dumps(target)}}})'], nested).strip()
        if reply.lower() != 'ok':
            reply = harness.hypr(['dispatch', 'closewindow', target], nested).strip()
        assert reply.lower() == 'ok', 'private owner close dispatch failed'
        closing = sheet('owner-close-wait', lambda s: s['model']['closing'] and s['open'] and s['applying']
                        and s['cancel_requested'] and not s['geometry']['cancel_enabled'])
        assert closing['epoch'] == before_close['epoch'] and not closing['dismissible']
        assert not closing['geometry']['apply_enabled'] and not any(closing['geometry']['field_enabled'])
        assert closing['cancel_requested'] and not closing['geometry']['cancel_enabled']
        assert 'Cancelación solicitada' in closing['geometry']['status']
        release(ticket)
        service.wait(timeout=20)
        assert service.returncode == 0 and 'FILES_CLOSE_DRAINED' in text()
        request_id = ticket.removeprefix('ack-create-')
        assert f'FILES_MUTATION_FINISHED id={request_id} ' in text()
        report['checks'].append('owner close keeps the applying Sheet visible until the real worker ACK drains')
        report['functional_gate'] = 'PASS'
    except Exception as exc:
        # Never stringify subprocess exceptions: their command can contain startup context.
        report['error'] = type(exc).__name__ + ': ' + (str(exc) if isinstance(exc, AssertionError) else 'see private logs')
    finally:
        for child in (origin, service):
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
