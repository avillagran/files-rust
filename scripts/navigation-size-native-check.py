#!/usr/bin/env python3
"""Check breadcrumb and size presentation in an isolated Sway session.

Private filesystem fixtures, compositor, session bus and evidence are retained.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import secrets
import select
from pathlib import Path
import shlex
import signal
import socket as socket_module
import stat
import struct
import subprocess
import sys
import time
import zlib
from xml.sax.saxutils import escape


ROOT = Path(__file__).resolve().parent.parent

def open_directory(path):
    """Open every absolute directory component without following symlinks."""
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('directory must be an absolute normalized path')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parts[1:]:
            nxt = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd)
            fd = nxt
        return fd
    except BaseException:
        os.close(fd)
        raise


def check_private_directory(path):
    fd = open_directory(path)
    info = os.fstat(fd)
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        os.close(fd)
        raise ValueError('fixture directory must be owned and mode 0700')
    return fd


def create_directory(parent, name, mode=0o700):
    fd = open_directory(parent)
    try:
        os.mkdir(name, mode, dir_fd=fd)
        child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
        os.close(child)
    finally:
        os.close(fd)
    return Path(parent) / name


def create_fixture(parent, prefix):
    for _ in range(32):
        name = prefix + secrets.token_hex(6)
        try:
            path = create_directory(parent, name)
        except FileExistsError:
            continue
        fd = check_private_directory(path)
        os.close(fd)
        return path
    raise RuntimeError('cannot allocate unique private fixture')


def entry_kind(path):
    path = Path(path)
    fd = open_directory(path.parent)
    try:
        try:
            info = os.stat(path.name, dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            return None
        if stat.S_ISLNK(info.st_mode):
            raise ValueError('symlink found at private fixture boundary')
        if info.st_uid != os.getuid():
            raise ValueError('private fixture entry is not owned by this user')
        if stat.S_ISREG(info.st_mode): return 'file'
        if stat.S_ISDIR(info.st_mode): return 'directory'
        if stat.S_ISSOCK(info.st_mode): return 'socket'
        raise ValueError('unexpected private fixture entry type')
    finally:
        os.close(fd)


def read_private(path):
    path = Path(path)
    directory = open_directory(path.parent)
    try:
        fd = os.open(path.name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
        with os.fdopen(fd, 'rb') as file:
            info = os.fstat(file.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise ValueError('private input is not a regular file')
            data = file.read(32 * 1024 * 1024 + 1)
            if len(data) > 32 * 1024 * 1024:
                raise ValueError('private input exceeds fixture limit')
            return data
    finally:
        os.close(directory)


def open_private_executable(path):
    path = Path(path)
    directory = open_directory(path.parent)
    try:
        fd = os.open(path.name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or not info.st_mode & 0o100:
            os.close(fd)
            raise ValueError('private helper is not an owned executable file')
        return fd
    finally:
        os.close(directory)


def write_private(path, value):
    path = Path(path)
    directory = open_directory(path.parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        fd = os.open(path.name, flags, 0o600, dir_fd=directory)
        with os.fdopen(fd, 'wb') as file:
            if not stat.S_ISREG(os.fstat(file.fileno()).st_mode):
                raise ValueError('private output is not a regular file')
            file.write(value if isinstance(value, bytes) else value.encode())
    finally:
        os.close(directory)


def fixture_png(color):
    """Small deterministic PNG; bytes are generated without external files."""
    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)
    width, height = 64, 48
    row = b'\x00' + bytes(color) * width
    header = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) +
            chunk(b'IDAT', zlib.compress(row * height)) + chunk(b'IEND', b''))


def open_private_output(path):
    path = Path(path)
    directory = open_directory(path.parent)
    try:
        fd = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o600, dir_fd=directory)
        return os.fdopen(fd, 'wb')
    finally:
        os.close(directory)


def waiting_files(path, phase):
    directory = check_private_directory(path)
    try:
        matches = []
        for name in os.listdir(directory):
            if name.startswith(phase + '-') and name.endswith('.waiting'):
                info = os.stat(name, dir_fd=directory, follow_symlinks=False)
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError('worker signal is not a regular file')
                matches.append(name)
        return matches
    finally:
        os.close(directory)


def runtime_in_use(runtime):
    hypr = Path(runtime) / 'hypr'
    kind = entry_kind(hypr)
    if kind is None:
        return False
    if kind != 'directory':
        raise ValueError('invalid compositor runtime entry')
    fd = open_directory(hypr)
    try:
        for name in os.listdir(fd):
            info = os.stat(name, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISLNK(info.st_mode):
                raise ValueError('symlink in private compositor runtime')
            if not stat.S_ISDIR(info.st_mode):
                continue
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            try:
                try:
                    sock = os.stat('.socket.sock', dir_fd=child, follow_symlinks=False)
                except FileNotFoundError:
                    continue
                if stat.S_ISLNK(sock.st_mode):
                    raise ValueError('symlink in private compositor runtime')
                if stat.S_ISSOCK(sock.st_mode):
                    return True
            finally:
                os.close(child)
        return False
    finally:
        os.close(fd)


def preserve_entry(path):
    """Move an existing regular fixture entry aside without deleting it."""
    path = Path(path)
    if entry_kind(path) is None:
        return
    if entry_kind(path) != 'file':
        raise ValueError('only regular fixture files may be preserved')
    directory = open_directory(path.parent)
    try:
        target = path.name + '.retained-' + secrets.token_hex(8)
        os.link(path.name, target, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False)
        # Rename the original name to a unique retained name; never unlink.
        os.rename(path.name, path.name + '.moved-' + secrets.token_hex(8),
                  src_dir_fd=directory, dst_dir_fd=directory)
    finally:
        os.close(directory)


def stage_command(path, value):
    """Publish an owned command without replacing an existing entry."""
    path = Path(path)
    directory = open_directory(path.parent)
    pending = '.command-' + secrets.token_hex(8)
    try:
        fd = os.open(pending, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o600, dir_fd=directory)
        with os.fdopen(fd, 'wb') as file:
            file.write(value.encode())
        os.link(pending, path.name, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False)
        # Keep the source link as evidence; the application owns the command link.
    finally:
        os.close(directory)


def policy_enabled(value):
    return value.get('bool') is True or (type(value.get('int')) is int and value['int'] == 1)


def wayland_socket_name(value):
    return isinstance(value, str) and value.startswith('wayland-') and value[8:].isdigit()


def started(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else int(fields[19])
    except (OSError, ValueError, IndexError):
        return None


def stop_tree(process, _sampler):
    # This child was started in a new session; an unreaped PID cannot be reused.
    if process.poll() is None:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            if process.poll() is not None:
                break
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                break
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass


class CacheScenarioComplete(Exception):
    pass


def main():
    if not __debug__:
        raise SystemExit('This check requires Python assertions to be enabled.')
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    ap.add_argument('--size', choices=('1000x650', '700x500'), default='1000x650')
    ap.add_argument('--animations', choices=('on', 'off'), default='on')
    ap.add_argument('--cache-error', action='store_true')
    ap.add_argument('--sway-payload', type=Path, default=Path('/var/tmp/files-m2-private-sway-20261107/payload/usr'))
    ap.add_argument('--private-child', action='store_true', help=argparse.SUPPRESS)
    args = ap.parse_args()
    cache = Path.home() / '.cache/files-test'
    if entry_kind(cache) is None:
        create_directory(cache.parent, cache.name)
    cache_fd = open_directory(cache)
    cache_info = os.fstat(cache_fd)
    if cache_info.st_uid != os.getuid() or stat.S_IMODE(cache_info.st_mode) & 0o022:
        os.close(cache_fd)
        raise ValueError('fixture parent must be owned and not group/world writable')
    os.close(cache_fd)
    if not args.private_child:
        directory = create_fixture(cache, 'feedback-ui-bus-')
        config = directory / 'bus.conf'
        write_private(config, '<busconfig><type>session</type><listen>unix:tmpdir=' + escape(str(directory)) +
                      '</listen><auth>EXTERNAL</auth><policy context="default">'
                      '<allow send_destination="*"/><allow receive_sender="*"/>'
                      '<allow own="*"/></policy></busconfig>')
        env = dict(os.environ)
        env['FILES_TEST_PARENT_BUS'] = env.get('DBUS_SESSION_BUS_ADDRESS', '')
        env['FILES_TEST_BUS_DIR'] = str(directory)
        for key in ('DBUS_SESSION_BUS_ADDRESS', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
            env.pop(key, None)
        extra = ['--cache-error'] if args.cache_error else []
        return subprocess.run(['dbus-run-session', '--config-file', str(config), '--', sys.executable,
                               str(Path(__file__).resolve()), '--private-child', '--binary', str(args.binary.resolve()),
                               '--size', args.size, '--animations', args.animations, '--sway-payload', str(args.sway_payload), *extra],
                              env=env, check=False).returncode
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS', '')
    if bus == os.environ.get('FILES_TEST_PARENT_BUS', '') or not bus.startswith(
            'unix:path=' + os.environ.get('FILES_TEST_BUS_DIR', '<absent>') + '/'):
        raise ValueError('refusing non-private bus')
    binary = args.binary.resolve(strict=True)
    base = create_fixture(cache, 'feedback-ui-native-')
    report = {'functional_gate': 'FAIL', 'fixture_dir': str(base), 'checks': [], 'states': [],
              'binary_path': str(binary), 'requested_size': args.size, 'animations': args.animations,
              'captures': [], 'animation_comparison': 'NOT_COMPARED',
              'scope': 'native input and private mutation fixtures; executable paths and compositor-owned paths are outside fixture file checks; this profile alone does not prove cross-profile motion',
              'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED'}
    service = origin = compositor = keyboard = pointer_process = None
    identities = {}
    try:
        for name, field in (('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                            ('files-html-helper', 'html_helper_sha256')):
            report[field] = hashlib.sha256((binary if name == 'files' else binary.parent / name).read_bytes()).hexdigest()
        runtime = create_fixture(Path('/tmp'), 'f-')
        config = base / 'sway.conf'
        width, height = args.size.split('x')
        write_private(config, f'output * resolution 1280x800\nseat seat0 fallback true\nfor_window [app_id=".*"] floating enable, resize set {width} {height}\n')
        sway_payload = args.sway_payload.resolve(strict=True)
        compositor_env = {**os.environ, 'XDG_RUNTIME_DIR': str(runtime), 'HOME': str(base),
                          'XDG_CONFIG_HOME': str(base), 'XDG_DATA_HOME': str(base), 'XDG_CACHE_HOME': str(base), 'TMPDIR': str(base),
                          'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'gles2',
                          'WLR_LIBINPUT_NO_DEVICES': '1', 'WLR_HEADLESS_OUTPUTS': '1',
                          'LD_LIBRARY_PATH': str(sway_payload / 'lib')}
        for name in ('WAYLAND_DISPLAY','WAYLAND_SOCKET','DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE',
                     'XDG_ACTIVATION_TOKEN','DESKTOP_STARTUP_ID','LIBGL_ALWAYS_SOFTWARE'):
            compositor_env.pop(name, None)
        with open_private_output(base / 'compositor.log') as output:
            compositor = subprocess.Popen([str(sway_payload / 'bin/sway'), '-c', str(config)],
                                          env=compositor_env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        identity = started(compositor.pid)
        assert identity is not None, 'private compositor did not start'
        identities = {compositor.pid: identity}
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and compositor.poll() is None:
            sockets = [name for name in os.listdir(runtime) if name.startswith('sway-ipc.') and name.endswith('.sock')]
            if len(sockets) == 1 and entry_kind(runtime / sockets[0]) == 'socket' and entry_kind(runtime / 'wayland-1') == 'socket':
                break
            time.sleep(.1)
        else:
            raise AssertionError('private Sway unavailable; see compositor.log')
        socket = 'wayland-1'
        ipc = runtime / sockets[0]
        compositor_pid, compositor_start = compositor.pid, identity
        report['nested'] = {'socket': socket, 'pid': compositor_pid, 'runtime': str(runtime), 'backend': 'private-sway-headless'}
        report['motion_and_hyprland'] = 'NOT_RUN'
        nested = {**compositor_env, 'WAYLAND_DISPLAY': socket, 'SWAYSOCK': str(ipc)}
        def sway(*args):
            validate_owned_wayland()
            p = subprocess.run([str(sway_payload / 'bin/swaymsg'), *args], env=nested, capture_output=True, text=True, timeout=5, check=True)
            return json.loads(p.stdout)
        def windows():
            tree = sway('-t','get_tree')
            def walk(n):
                yield n
                for c in n.get('nodes',[]) + n.get('floating_nodes',[]):
                    yield from walk(c)
            return list(walk(tree))
        home = create_directory(base, 'home')
        for name in ('Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'A', 'B'):
            create_directory(home, name)
        create_directory(home / 'A', 'Nested')
        leaf = home / 'A' / 'Nested'
        for segment in ('ancestor-with-readable-name', 'second-long-ancestor', 'third-long-ancestor', 'parent', 'leaf'):
            leaf = create_directory(leaf, segment)
        write_private(home / 'A' / 'Nested' / 'inside.txt', b'Private size fixture\n')
        write_private(home / 'A' / 'Nested' / 'thumbnail.png', fixture_png((210, 40, 55)))
        create_directory(base, 'retained-originals')
        for number in range(45):
            folder = create_directory(leaf, f'Folder{number:02d}')
            for child in range(30):
                write_private(folder / f'sample{child:02d}.txt', b'Private size fixture\n')
        payload = home / 'A' / 'photo1.txt'
        write_private(payload, b'Private first fixture\n')
        write_private(home / 'A' / 'photo2.txt', b'Private second fixture\n')
        write_private(home / 'A' / 'percent%name.txt', b'Private percent fixture\n')
        write_private(home / 'A' / os.fsdecode(b'raw-\xff\nname.txt'), b'Private raw fixture\n')
        gate = create_directory(base, 'worker-gate')
        for name in ('config', 'data', 'files-config', 'origin-config', 'origin-data', 'origin-files-config'):
            create_directory(base, name)
        if args.cache_error:
            cache_dir = create_directory(base / 'files-config', 'size-cache')
            write_private(cache_dir / 'observations.bin', b'incompatible')
        write_private(base / 'devices.json', '{"blockdevices": []}\n')
        write_private(base / 'shell.toml', '[font]\nbase-size = 14\n')
        # A private copy preserves the exact binary; the native helper proxy gates
        # only folder-size requests, then executes the unmodified real helper.
        programs = create_directory(base, 'programs')
        app_copy = programs / 'files'
        helper_real = programs / 'files-helper-real'
        write_private(app_copy, binary.read_bytes())
        write_private(helper_real, (binary.parent / 'files-helper').read_bytes())
        os.chmod(app_copy, 0o700)
        os.chmod(helper_real, 0o700)
        assert hashlib.sha256(read_private(app_copy)).hexdigest() == report['binary_sha256']
        assert hashlib.sha256(read_private(helper_real)).hexdigest() == report['helper_sha256']
        gate_release = base / 'size-release'
        proxy_source = programs / 'helper-proxy.c'
        write_private(proxy_source, r'''#include <sys/stat.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <stdlib.h>
int main(int argc, char **argv) {
  if (argc > 1 && (!strcmp(argv[1], "measure_directory") || !strcmp(argv[1], "measure_directory_allocated"))) {
    const char *gate = getenv("FILES_UI_SIZE_GATE");
    struct stat st; struct timespec delay = {0, 10000000};
    if (!gate || gate[0] != '/') return 2;
    for (int i = 0; i < 1500; i++) {
      if (lstat(gate, &st) == 0) { if (!S_ISREG(st.st_mode) || st.st_uid != getuid()) return 3; break; }
      if (i == 1499) return 4;
      nanosleep(&delay, 0);
    }
  }
  execv(REAL_HELPER, argv);
  return 5;
}
''')
        helper_proxy = programs / 'files-helper'
        subprocess.run(['cc', '-O2', '-DREAL_HELPER=' + json.dumps(str(helper_real)),
                        '-o', str(helper_proxy), str(proxy_source)], check=True)
        report['helper_gate'] = 'private proxy delays only size measurements until an owned release file'
        report['helper_proxy_sha256'] = hashlib.sha256(read_private(helper_proxy)).hexdigest()
        report['helper_proxy_source_sha256'] = hashlib.sha256(read_private(proxy_source)).hexdigest()
        raw, log = base / 'service.ppm', base / 'service.log'
        env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
               'XDG_DATA_HOME': str(base / 'data'), 'XDG_CACHE_HOME': str(base), 'FILES_CONFIG_DIR': str(base / 'files-config'),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
               'FILES_TEST_MUTATION_GATE_DIR': str(gate), 'FILES_TEST_MUTATIONS': '1',
               'FILES_METRICS_READY': '1',
               'FILES_TEST_DEVICES_JSON': str(base / 'devices.json'), 'FILES_CLOSE_TRACE': '1',
               'FILES_UI_SIZE_GATE': str(gate_release), 'OMARCHY_FLTK_ANIM': args.animations,
               'OMARCHY_FLTK_USER_SHELL': str(base / 'shell.toml'), 'TMPDIR': str(base),
               'MAGICK_TEMPORARY_PATH': str(base)}
        env.pop('FILES_TEST_ISOLATE_INPUT', None)
        with open_private_output(log) as out:
            service = subprocess.Popen([str(app_copy), str(leaf), '--size', args.size, '--scale', '1',
                                        '--no-session', '--ui-smoke', '--ui-scene', 'rename', '--exit-after', '180'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=out, start_new_session=True)

        def text():
            return read_private(log).decode(errors='replace')

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
            return [c for c in windows() if c.get('pid') == pid and c.get('type') in ('con', 'floating_con')]

        def active():
            return next((c.get('pid') for c in windows() if c.get('focused')), None)

        def validate_owned_wayland():
            fd = check_private_directory(runtime)
            os.close(fd)
            if (started(compositor_pid) != compositor_start or entry_kind(runtime / socket) != 'socket' or
                    entry_kind(ipc) != 'socket'):
                raise ValueError('private compositor identity or socket changed')

        def open_owned_wayland():
            """Preconnect a verified socket; child tools inherit only this fd."""
            validate_owned_wayland()
            directory = check_private_directory(runtime)
            connection = socket_module.socket(socket_module.AF_UNIX, socket_module.SOCK_STREAM)
            try:
                before = os.stat(socket, dir_fd=directory, follow_symlinks=False)
                if not stat.S_ISSOCK(before.st_mode) or before.st_uid != os.getuid():
                    raise ValueError('private Wayland entry is not an owned socket')
                connection.settimeout(5)
                # AF_UNIX connect has no dir_fd/O_NOFOLLOW variant. The 0700
                # parent, lstat checks and peer/starttime/inode checks reject a
                # changed peer before any input, but cannot make lookup atomic.
                connection.connect(str(runtime / socket))
                peer_pid, peer_uid, _ = struct.unpack('=iII', connection.getsockopt(
                    socket_module.SOL_SOCKET, socket_module.SO_PEERCRED, struct.calcsize('=iII')))
                after = os.stat(socket, dir_fd=directory, follow_symlinks=False)
                if (peer_pid != compositor_pid or peer_uid != os.getuid() or
                        not stat.S_ISSOCK(after.st_mode) or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino) or
                        started(compositor_pid) != compositor_start):
                    raise ValueError('private Wayland peer or socket identity changed')
                connection.settimeout(None)
                return connection
            except BaseException:
                connection.close()
                raise
            finally:
                os.close(directory)

        def focus_own(pid):
            client = wait(lambda: clients(pid)[0] if len(clients(pid)) == 1 else None, 'owned window')
            assert sway('[con_id=' + str(client['id']) + '] focus')[0]['success']
            wait(lambda: active() == pid, 'owned focus')

        def send(action, capture=raw, source_log=None):
            if source_log is None:
                source_log = log
            offset = len(read_private(source_log).decode(errors='replace'))
            stage_command(capture.with_suffix('.command'), action)
            wait(lambda: f'FILES_MUTATION_COMMAND_OK {action}\n' in read_private(source_log).decode(errors='replace')[offset:], action)
            return offset

        def send_popup(action):
            offset = len(text())
            stage_command(raw.with_suffix('.popup-command'), action)
            wait(lambda: f'FILES_TOOLKIT_FIXTURE action={action}\n' in text()[offset:], action)

        def state(label):
            offset = send('mutation-sheet-state')
            value = wait(lambda: next((json.loads(line[len(STATE):]) for line in text()[offset:].splitlines()
                                       if line.startswith(STATE)), None), 'sheet state ' + label)
            report['states'].append({'label': label, **value})
            return value

        def key(*keys):
            if active() != service.pid:
                raise ValueError('refusing keyboard input to a non-owned window')
            validate_owned_wayland()
            if keyboard.poll() is not None or any('\t' in s or '\n' in s for s in keys):
                raise ValueError('invalid or disconnected private keyboard')
            keyboard.stdin.write(('\t'.join(keys) + '\n').encode())
            keyboard.stdin.flush()
            if not select.select([keyboard.stdout], [], [], 10)[0] or keyboard.stdout.readline() != b'OK\n':
                raise ValueError('private keyboard command failed')
            validate_owned_wayland()

        def clipboard(text_value):
            # The nested socket is explicit: never access the user's clipboard.
            with open_owned_wayland() as connection:
                tool_env = {**nested, 'WAYLAND_SOCKET': str(connection.fileno())}
                subprocess.run(['wl-copy'], input=text_value, text=True, env=tool_env,
                               pass_fds=(connection.fileno(),), check=True, timeout=10)
            validate_owned_wayland()

        def capture(label):
            preserve_entry(raw)
            stage_command(raw.with_suffix('.request'), '')
            def complete():
                if entry_kind(raw) is None: return False
                parts = read_private(raw).split(b'\n', 3)
                if len(parts) != 4: return False
                try: w, h = map(int, parts[1].split())
                except ValueError: return False
                return len(parts[3]) == w * h * 3
            wait(complete, 'frame ' + label)
            path = base / (label + '.png')
            converted = subprocess.run(['magick', 'ppm:-', 'png:-'], input=read_private(raw),
                                       stdout=subprocess.PIPE, env=env, check=True, timeout=15)
            write_private(path, converted.stdout)
            report['captures'].append(str(path))

        def capture_desktop(label):
            with open_owned_wayland() as connection:
                capture_env = {**nested, 'WAYLAND_SOCKET': str(connection.fileno())}
                rendered = subprocess.run(['grim', '-'], stdout=subprocess.PIPE, env=capture_env,
                                          pass_fds=(connection.fileno(),), check=True, timeout=15)
            validate_owned_wayland()
            path = base / (label + '.png')
            write_private(path, rendered.stdout)
            report['captures'].append(str(path))

        wait(lambda: 'FILES_UI_SMOKE_OK' in text() and clients(service.pid), 'private window startup')
        wait(lambda: 'FILES_SCENE_READY view=groups loaded=true' in text(), 'private listing loaded')
        report['window_size'] = [clients(service.pid)[0]['rect']['width'], clients(service.pid)[0]['rect']['height']]
        assert report['window_size'][0] >= 600 and report['window_size'][1] >= 400
        focus_own(service.pid)
        tools_dir = ROOT / 'scripts/tools'
        header = base / 'virtual-keyboard-unstable-v1-client-protocol.h'
        protocol = base / 'virtual-keyboard-protocol.c'
        keyboard_exe = base / 'persistent-keyboard'
        subprocess.run(['wayland-scanner', 'client-header', str(tools_dir / 'virtual-keyboard-unstable-v1.xml'), str(header)], check=True)
        subprocess.run(['wayland-scanner', 'private-code', str(tools_dir / 'virtual-keyboard-unstable-v1.xml'), str(protocol)], check=True)
        flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'wayland-client', 'xkbcommon'], text=True).split()
        subprocess.run(['cc', '-O2', '-I' + str(base), '-o', str(keyboard_exe), str(tools_dir / 'persistent-keyboard.c'), str(protocol), *flags], check=True)
        executable_fd = open_private_executable(keyboard_exe)
        try:
            with open_owned_wayland() as connection, open_private_output(base / 'keyboard.log') as keyboard_log:
                key_env = {**nested, 'WAYLAND_SOCKET': str(connection.fileno())}
                keyboard = subprocess.Popen(['/proc/self/fd/' + str(executable_fd)], env=key_env,
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=keyboard_log,
                                            pass_fds=(connection.fileno(), executable_fd), start_new_session=True)
        finally:
            os.close(executable_fd)
        if not select.select([keyboard.stdout], [], [], 10)[0] or keyboard.stdout.readline() != b'READY\n':
            raise ValueError('persistent private keyboard did not initialize')
        pointer_header = base / 'wlr-virtual-pointer-unstable-v1-client.h'
        pointer_protocol = base / 'virtual-pointer-protocol.c'
        pointer_exe = base / 'private-pointer'
        pointer_xml = tools_dir / 'wlr-virtual-pointer-unstable-v1.xml'
        subprocess.run(['wayland-scanner', 'client-header', str(pointer_xml), str(pointer_header)], check=True)
        subprocess.run(['wayland-scanner', 'private-code', str(pointer_xml), str(pointer_protocol)], check=True)
        pointer_flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'wayland-client'], text=True).split()
        subprocess.run(['cc', '-O2', '-I' + str(base), '-o', str(pointer_exe),
                        str(tools_dir / 'compact-persistent-pointer.c'), str(pointer_protocol), *pointer_flags], check=True)
        fd = open_private_executable(pointer_exe)
        try:
            validate_owned_wayland()
            with open_private_output(base / 'pointer.log') as pointer_log:
                pointer_process = subprocess.Popen(['/proc/self/fd/' + str(fd), socket, str(compositor_pid), '1280', '800'],
                                                   env=nested, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                                   stderr=pointer_log, pass_fds=(fd,), start_new_session=True)
        finally:
            os.close(fd)
        if not select.select([pointer_process.stdout], [], [], 10)[0] or pointer_process.stdout.readline() != b'READY\n':
            raise ValueError('private pointer did not initialize')

        def pointer(x, y, click=False):
            own = clients(service.pid)
            if active() != service.pid or len(own) != 1:
                raise ValueError('refusing pointer input to a non-owned window')
            rect = own[0]['rect']
            if not (rect['x'] <= x < rect['x'] + rect['width'] and
                    rect['y'] <= y < rect['y'] + rect['height']):
                raise ValueError('refusing pointer input outside the owned window')
            validate_owned_wayland()
            if pointer_process.poll() is not None:
                raise ValueError('private pointer exited')
            pointer_process.stdin.write(f'{x} {y} {int(click)}\n'.encode())
            pointer_process.stdin.flush()
            if not select.select([pointer_process.stdout], [], [], 10)[0] or pointer_process.stdout.readline() != b'OK\n':
                raise ValueError('private pointer command failed')
            validate_owned_wayland()

        def usability(label, predicate=lambda _value: True):
            def read_state():
                offset = send('usability-state')
                value = next((json.loads(line[len('FILES_USABILITY_STATE '):])
                              for line in text()[offset:].splitlines()
                              if line.startswith('FILES_USABILITY_STATE ')), None)
                return value if value is not None and predicate(value) else None
            value = wait(read_state, label)
            report['states'].append({'label': label, **value})
            return value

        def row_center(rect):
            client = clients(service.pid)[0]
            window, inner = client['rect'], client.get('window_rect') or {'x': 0, 'y': 0}
            return (window['x'] + inner['x'] + rect[0] + rect[2] // 2,
                    window['y'] + inner['y'] + rect[1] + rect[3] // 2)

        def breadcrumbs(label):
            offset = len(text())
            send_popup('breadcrumb-state')
            value = wait(lambda: next((json.loads(line[len('FILES_BREADCRUMB_STATE '):])
                                      for line in text()[offset:].splitlines()
                                      if line.startswith('FILES_BREADCRUMB_STATE ')), None), label)
            report['states'].append({'label': label, **value})
            return value

        initial = usability('groups-initial', lambda s: s['loaded'] and s['size_busy'] and bool(s['toolbar']['items']))
        crumb = breadcrumbs('breadcrumbs-initial')
        assert crumb['overflow_rect'] and crumb['overflow'], crumb
        assert crumb['visible'] and crumb['visible'][-1]['path'] == str(leaf), crumb
        capture('groups-initial')
        report['observed_initial_busy'] = initial['size_busy']
        assert initial['size_busy'], 'size worker did not enter gated in-flight state'
        write_private(gate_release, b'release\n')
        report['initial_folder_labels'] = [v['size'] for v in initial['folders']]
        pointer(*row_center(crumb['overflow_rect']))
        time.sleep(.35)
        hovered = breadcrumbs('breadcrumbs-hover')
        assert hovered['overflow_rect'] == crumb['overflow_rect']
        pointer(*row_center(hovered['overflow_rect']), click=True)
        time.sleep(.35)
        pointer_opened = breadcrumbs('breadcrumbs-pointer-open')['popup_open']
        report['breadcrumb_pointer_opened'] = pointer_opened
        if not pointer_opened:
            key('-M', 'alt', '-k', 'Down', '-m', 'alt')
        opened = wait(lambda: (b if (b := breadcrumbs('breadcrumbs-menu'))['popup_open'] else None), 'breadcrumb menu')
        capture_desktop('breadcrumb-overflow-menu')
        key('-k', 'Escape')
        key('-M', 'ctrl', '-k', '2', '-m', 'ctrl')
        listing = usability('list-view', lambda s: any(i['label'] == 'Lista' and i['on'] for i in s['toolbar']['items']))
        capture('list-initial')
        settled = usability('list-sizes-settled', lambda s: not s['size_busy'] and any(v['size'] for v in s['folders']))
        capture('list-sizes-settled')
        report['settled_folder_labels'] = [v['size'] for v in settled['folders']]
        assert not any('\u2211' in (v['size'] or '') for v in settled['folders']), settled['folders']
        key('-M', 'ctrl', '-k', '1', '-m', 'ctrl')
        usability('groups-sizes-settled', lambda s: any(i['label'] == 'Grupos' and i['on'] for i in s['toolbar']['items']))
        capture('groups-sizes-settled')
        key('-M', 'ctrl', '-k', '2', '-m', 'ctrl')
        usability('list-before-child', lambda s: any(i['label'] == 'Lista' and i['on'] for i in s['toolbar']['items']))
        key('-k', 'Home')
        chosen = b'Folder20'.hex()
        for _ in range(45):
            selection = breadcrumbs('selection-before-child')
            if selection['selected_raw'] == chosen: break
            key('-k', 'Down')
        else: raise AssertionError('could not select Folder20 in list')
        before_child = usability('selected-before-child')
        report['before_child_scroll'] = before_child['scroll']
        assert before_child['scroll'] > 0, 'Folder20 must be reached with the List view scrolled'
        capture('selected-before-child')
        key('-k', 'Return')
        usability('entered-child', lambda s: s['loaded'] and bytes(s['path']) == os.fsencode(leaf / 'Folder20'))
        capture('entered-child')
        key('-k', 'BackSpace')
        usability('returned-parent', lambda s: s['loaded'] and bytes(s['path']) == os.fsencode(leaf))
        restored = wait(lambda: (b if (b := breadcrumbs('restored-selection'))['loaded'] and b['settled']
                                 and b['selected_raw'] == chosen and b['focus_browser'] else None), 'selection and browser focus restored')
        capture('returned-before-visible-check')
        after_child = usability('selected-after-return', lambda s: any(bytes(v['name']) == b'Folder20' for v in s['folders']))
        report['after_child_scroll'] = after_child['scroll']
        report['restored_selected_raw'] = restored['selected_raw']
        capture('selected-after-return')
        key('-M', 'alt', '-k', 'Up', '-m', 'alt')
        parent = usability('up-parent', lambda s: s['loaded'] and bytes(s['path']) == os.fsencode(leaf.parent))
        capture('up-parent')
        report['checks'] = ['deep breadcrumb overflow renders and opens; pointer outcome separately reported',
                            'group and list views rendered with bounded private folder fixtures',
                            'native Enter and Backspace restore Folder20 selection',
                            'native Alt+Up navigates to the actual parent',
                            'measured folder sizes carry no summation mark',
                            'List view reaches Folder20 scrolled; header pinning needs visual review']
        report['functional_scope'] = 'breadcrumb overflow, group and list folder size presentation'
        report['functional_gate'] = 'PASS'
        report['profile_gate'] = 'PASS'
        report['visual_review'] = 'REQUIRED'
    except CacheScenarioComplete:
        pass
    except Exception as exc:
        report['error'] = type(exc).__name__ + ': ' + (str(exc) if isinstance(exc, AssertionError) else 'see private logs')
    finally:
        for child in (origin, service, keyboard, pointer_process):
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
        write_private(base / 'report.json', json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(base / 'report.json'), 'functional_gate': report['functional_gate'], 'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
