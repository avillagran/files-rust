#!/usr/bin/env python3
"""Capture Files titlebars in a retained, private headless Hyprland session."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent

def starttime(pid):
    try:
        data = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(data[19]) if data[0] != 'Z' else None
    except (OSError, IndexError, ValueError):
        return None

def write(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as out:
        out.write(data if isinstance(data, bytes) else data.encode())

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--plugin', type=Path, required=True)
    p.add_argument('--asset-dir', type=Path, default=ROOT)
    p.add_argument('--parent-payload', type=Path, required=True)
    p.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    p.add_argument('--fixture', type=Path, help=argparse.SUPPRESS)
    a = p.parse_args()
    if not a.private_child:
        base = Path(tempfile.mkdtemp(prefix='files-titlebar-', dir='/tmp'))
        os.chmod(base, 0o700)
        print('Retained fixture:', base, flush=True)
        config = base / 'bus.conf'
        write(config, f'<busconfig><type>session</type><listen>unix:tmpdir={base}</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
        env = dict(os.environ)
        env.pop('DBUS_SESSION_BUS_ADDRESS', None)
        return subprocess.run(['dbus-run-session', '--config-file', str(config), '--', sys.executable, str(Path(__file__).resolve()), '--private-child', '--fixture', str(base), '--binary', str(a.binary.resolve(strict=True)), '--plugin', str(a.plugin.resolve(strict=True)), '--asset-dir', str(a.asset_dir.resolve(strict=True)), '--parent-payload', str(a.parent_payload.resolve(strict=True))], env=env).returncode
    base = a.fixture
    if not base or base.parent != Path('/tmp') or not base.name.startswith('files-titlebar-'):
        raise ValueError('invalid fixture')
    info = base.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError('fixture must be an owned private directory')
    if not os.environ.get('DBUS_SESSION_BUS_ADDRESS', '').startswith('unix:path=' + str(base) + '/'):
        raise ValueError('bus is not private')
    for name in ('home', 'config', 'data', 'files-config', 'tmp', 'cache'):
        (base / name).mkdir(mode=0o700)
    for name in ('applications', 'icons'):
        (base / 'data' / name).mkdir(mode=0o700)
    (base / 'home' / 'A').mkdir(mode=0o700)
    (base / 'home' / 'A' / 'Nested').mkdir(mode=0o700)
    write(base / 'data/applications/files.desktop', (a.asset_dir / 'packaging/files.desktop').read_bytes())
    # Absolute icon path in a private desktop entry avoids depending on the host icon theme.
    icon = base / 'data/icons/files.svg'
    write(icon, (a.asset_dir / 'assets/files.svg').read_bytes())
    desktop = base / 'data/applications/files-private.desktop'
    write(desktop, '[Desktop Entry]\nType=Application\nName=Files\nExec=files\nIcon=' + str(icon) + '\nStartupWMClass=files\n')
    # One entry claims the class: preserve the copied project entry as evidence outside applications.
    os.rename(base / 'data/applications/files.desktop', base / 'data/files.desktop.evidence')
    version = subprocess.check_output(['Hyprland', '--version'], text=True)
    conf = base / 'hyprland.lua'
    write(conf, '''hl.monitor({ output = "", mode = "1280x800@60", position = "0x0", scale = 1 })
hl.config({ general = { gaps_in = 8, gaps_out = 8, border_size = 2 }, animations = { enabled = false }, misc = { disable_hyprland_logo = true, disable_splash_rendering = true } })
hl.window_rule({ name = "test-float", match = { class = ".*" }, float = true, size = { 1000, 650 }, center = true })
if hl.plugin and hl.plugin.hyprbars then hl.config({ plugin = { hyprbars = { enabled = true, bar_height = 32, bar_text_align = "left", bar_text_size = 15 } } }) end
''')
    runtime = Path(tempfile.mkdtemp(prefix='h-', dir='/tmp'))
    parent_runtime = Path(tempfile.mkdtemp(prefix='s-', dir='/tmp'))
    env = dict(os.environ, HOME=str(base / 'home'), XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(base / 'config'), XDG_DATA_HOME=str(base / 'data'), FILES_CONFIG_DIR=str(base / 'files-config'), XDG_CACHE_HOME=str(base / 'cache'), TMPDIR=str(base / 'tmp'), HYPRLAND_NO_SD_NOTIFY='1', HYPRLAND_NO_SD_VARS='1')
    for key in ('WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DISPLAY', 'HYPRLAND_INSTANCE_SIGNATURE', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
        env.pop(key, None)
    for key in tuple(env):
        if key.startswith(('QT_', 'GTK_', 'GDK_')):
            env.pop(key, None)
    log = os.fdopen(os.open(base / 'compositor.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb')
    compositor = app = parent_proc = None
    report = {'fixture': str(base), 'binary_sha256': hashlib.sha256(a.binary.read_bytes()).hexdigest(), 'plugin_sha256': hashlib.sha256(a.plugin.read_bytes()).hexdigest(), 'version': version, 'status': 'FAIL'}
    try:
        payload = a.parent_payload
        parent_env = dict(env, XDG_RUNTIME_DIR=str(parent_runtime), LD_LIBRARY_PATH=str(payload / 'lib'), QT_PLUGIN_PATH=str(payload / 'lib/qt6/plugins'), QML_IMPORT_PATH=str(payload / 'lib/qt6/qml'), XDG_DATA_DIRS=str(payload / 'share') + ':/usr/local/share:/usr/share')
        parent_log = os.fdopen(os.open(base / 'parent.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb')
        parent_proc = subprocess.Popen([str(payload / 'bin/kwin_wayland'), '--virtual', '--width', '1280', '--height', '800', '--no-lockscreen', '--socket', 'wayland-parent'], env=parent_env, stdout=parent_log, stderr=parent_log, start_new_session=True)
        parent_born = starttime(parent_proc.pid)
        for _ in range(150):
            if parent_proc.poll() is not None: raise RuntimeError('private KWin exited')
            sockets = [p for p in parent_runtime.iterdir() if p.name == 'wayland-parent' and stat.S_ISSOCK(p.lstat().st_mode)]
            if len(sockets) == 1: break
            time.sleep(.1)
        else: raise RuntimeError('private KWin socket unavailable')
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
            peer.connect(str(sockets[0]))
            pid, uid, _ = struct.unpack('=iII', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('=iII')))
            if pid != parent_proc.pid or uid != os.getuid() or starttime(parent_proc.pid) != parent_born: raise RuntimeError('KWin socket peer mismatch')
            # Registry and sync requests; inspect advertised protocol versions before
            # starting Hyprland, which otherwise aborts when the parent lacks v6.
            peer.settimeout(5)
            peer.sendall(struct.pack('=IIIIII', 1, (12 << 16) | 1, 2, 1, 12 << 16, 3))
            data = b''
            globals_seen = {}
            done = False
            while not done:
                chunk = peer.recv(65536)
                if not chunk: raise RuntimeError('KWin registry closed')
                data += chunk
                while len(data) >= 8:
                    obj, header = struct.unpack_from('=II', data)
                    size, opcode = header >> 16, header & 0xffff
                    if size < 8 or size % 4: raise RuntimeError('invalid Wayland event')
                    if len(data) < size: break
                    event, data = data[:size], data[size:]
                    if obj == 2 and opcode == 0:
                        name_len = struct.unpack_from('=I', event, 12)[0]
                        end = 16 + ((name_len + 3) & ~3)
                        if end + 4 > len(event): raise RuntimeError('invalid registry event')
                        name = event[16:16 + name_len - 1].decode('utf-8')
                        globals_seen[name] = struct.unpack_from('=I', event, end)[0]
                    if obj == 3 and opcode == 0: done = True
            report['parent_globals'] = globals_seen
            if globals_seen.get('xdg_wm_base', 0) < 6 or globals_seen.get('wl_compositor', 0) < 6 or globals_seen.get('wl_seat', 0) < 9:
                raise RuntimeError('private parent lacks Aquamarine required protocol versions: ' + repr(globals_seen))
        for fd in Path(f'/proc/{parent_proc.pid}/fd').iterdir():
            try: target = os.readlink(fd)
            except FileNotFoundError: continue
            if target.startswith(('/dev/dri/card', '/dev/input/')): raise RuntimeError('Sway opened real device: ' + target)
        env['WAYLAND_DISPLAY'] = str(sockets[0])
        report['parent_compositor'] = {'pid': parent_proc.pid, 'runtime': str(parent_runtime), 'socket': str(sockets[0])}
        compositor = subprocess.Popen(['Hyprland', '--config', str(conf)], env=env, stdout=log, stderr=log, start_new_session=True)
        born = starttime(compositor.pid)
        def valid():
            if starttime(parent_proc.pid) != parent_born or parent_proc.poll() is not None:
                raise RuntimeError('private KWin identity changed')
            if not born or starttime(compositor.pid) != born or compositor.poll() is not None:
                raise RuntimeError('owned compositor identity changed')
            for fd in Path(f'/proc/{compositor.pid}/fd').iterdir():
                try: target = os.readlink(fd)
                except FileNotFoundError: continue
                if target.startswith(('/dev/dri/card', '/dev/input/')):
                    raise RuntimeError('compositor opened a real display or input device: ' + target)
            for path in (runtime, runtime / 'hypr' / env['HYPRLAND_INSTANCE_SIGNATURE']):
                i = path.lstat()
                if not stat.S_ISDIR(i.st_mode) or i.st_uid != os.getuid(): raise RuntimeError('invalid runtime')
        def ctl(*args):
            valid()
            ipc = runtime / 'hypr' / env['HYPRLAND_INSTANCE_SIGNATURE'] / '.socket.sock'
            before = ipc.lstat()
            if not stat.S_ISSOCK(before.st_mode) or before.st_uid != os.getuid():
                raise RuntimeError('invalid private IPC socket')
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
                peer.connect(str(ipc))
                pid, uid, _ = struct.unpack('=iII', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('=iII')))
                after = ipc.lstat()
                if pid != compositor.pid or uid != os.getuid() or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                    raise RuntimeError('private IPC peer mismatch')
            p = subprocess.run(['hyprctl', *args], env=env, capture_output=True, text=True, timeout=5, check=True)
            if p.stdout.startswith('Err') or "Couldn't" in p.stdout: raise RuntimeError(p.stdout)
            return p.stdout
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if compositor.poll() is not None: raise RuntimeError('compositor exited')
            root = runtime / 'hypr'
            if root.is_dir():
                dirs = [x for x in root.iterdir() if x.is_dir() and not x.is_symlink()]
                if len(dirs) == 1 and (dirs[0] / '.socket.sock').exists():
                    env['HYPRLAND_INSTANCE_SIGNATURE'] = dirs[0].name
                    break
            time.sleep(.1)
        else: raise RuntimeError('no private compositor socket')
        instances = json.loads(ctl('instances', '-j'))
        instance = next(x for x in instances if x['instance'] == env['HYPRLAND_INSTANCE_SIGNATURE'])
        if instance.get('pid') != compositor.pid: raise RuntimeError('instance PID mismatch')
        env['WAYLAND_DISPLAY'] = instance['wl_socket']
        def connection():
            valid()
            path = runtime / env['WAYLAND_DISPLAY']
            before = path.lstat()
            if not stat.S_ISSOCK(before.st_mode) or before.st_uid != os.getuid(): raise RuntimeError('invalid Wayland socket')
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.connect(str(path))
            pid, uid, _ = struct.unpack('=iII', sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('=iII')))
            after = path.lstat()
            if pid != compositor.pid or uid != os.getuid() or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                sock.close(); raise RuntimeError('Wayland peer mismatch')
            valid()
            return sock
        def wl(command, capture=False):
            with connection() as sock:
                e = dict(env, WAYLAND_SOCKET=str(sock.fileno()))
                e.pop('WAYLAND_DISPLAY', None)
                return subprocess.run(command, env=e, pass_fds=(sock.fileno(),), stdout=subprocess.PIPE if capture else subprocess.DEVNULL, stderr=subprocess.PIPE, check=True, timeout=10).stdout
        # The only parent output is private KWin's headless backend.
        valid()
        monitors = json.loads(ctl('monitors', '-j'))
        if not monitors or any(not m['name'].startswith(('WL-', 'WAYLAND-')) for m in monitors):
            raise RuntimeError('compositor did not use only private nested outputs: ' + repr(monitors))
        report['monitors'] = monitors
        ctl('plugin', 'load', str(a.plugin))
        errors = ctl('configerrors').strip()
        if errors: raise RuntimeError('private configuration errors: ' + errors)
        report['defaults'] = {name: json.loads(ctl('getoption', 'plugin:hyprbars:' + name, '-j')) for name in ('bar_icon', 'bar_title_source', 'bar_title_separator')}
        defaults = report['defaults']
        if not (defaults['bar_icon'].get('bool') is True or defaults['bar_icon'].get('int') == 1) or defaults['bar_title_source'].get('str') != 'title' or defaults['bar_title_separator'].get('str') != '›':
            raise RuntimeError('unexpected plugin title defaults: ' + repr(defaults))
        app_log = os.fdopen(os.open(base / 'app.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb')
        app = subprocess.Popen([str(a.binary), str(base / 'home/A/Nested'), '--size', '1000x650', '--scale', '1', '--no-session', '--exit-after', '90'], env=env, stdout=app_log, stderr=app_log, start_new_session=True)
        def clients(): return json.loads(ctl('clients', '-j'))
        def wait_title(path):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                matches = [c for c in clients() if c.get('pid') == app.pid]
                if len(matches) == 1 and (matches[0].get('title') == str(path) or matches[0].get('title', '').startswith(str(path) + ' · ')):
                    return matches[0]
                if app.poll() is not None: raise RuntimeError('Files exited')
                time.sleep(.1)
            raise RuntimeError('Files title did not update')
        first = wait_title(base / 'home/A/Nested')
        time.sleep(1)
        write(base / 'initial.png', wl(['grim', '-'], True))
        ctl('plugin', 'unload', str(a.plugin))
        ctl('plugin', 'load', str(a.plugin))
        if ctl('configerrors').strip(): raise RuntimeError('configuration errors after plugin reload')
        wait_title(base / 'home/A/Nested')
        time.sleep(.5)
        write(base / 'after-reload.png', wl(['grim', '-'], True))
        active = json.loads(ctl('activewindow', '-j'))
        if active.get('pid') != app.pid: raise RuntimeError('Files is not focused in private compositor')
        wl(['wtype', '-M', 'alt', '-k', 'Up', '-m', 'alt'])
        second = wait_title(base / 'home/A')
        time.sleep(.5)
        write(base / 'parent.png', wl(['grim', '-'], True))
        report.update(status='CAPTURED_VISUAL_REVIEW_REQUIRED', initial=first, parent=second, compositor_pid=compositor.pid, wayland=env['WAYLAND_DISPLAY'])
    finally:
        write(base / 'report.json', json.dumps(report, indent=2))
        for proc in (app, compositor, parent_proc):
            if proc and proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try: proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL); proc.wait(timeout=5)
        log.close()
    print(json.dumps(report, indent=2))
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
