#!/usr/bin/env python3
"""Verify private Wayland proxy framing, descriptor handling and safe cleanup.

Uses only owned socket pairs and a retained private fixture; no GUI, network,
filesystem tree deletion or symlink fixtures are involved.
"""
import array
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'scripts/tools/wayland-activation-proxy.py'
spec = importlib.util.spec_from_file_location('activation_proxy', SOURCE)
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


def message(obj, opcode, body=b''):
    return struct.pack('=II', obj, ((8 + len(body)) << 16) | opcode) + body


def number(n):
    return struct.pack('=I', n)


def string(value):
    value = value.encode() + b'\0'
    return number(len(value)) + value + b'\0' * (-len(value) % 4)


def exact(sock, count):
    data = b''
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise AssertionError('unexpected socket close')
        data += chunk
    return data


def control(path, command):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(3)
        client.connect(str(path))
        client.sendall((command + '\n').encode())
        return json.loads(client.recv(1024))


def main():
    cache = Path.home() / '.cache/files-test'
    cache.mkdir(parents=True, exist_ok=True)
    base = Path(tempfile.mkdtemp(prefix='wayland-proxy-check-', dir=cache))
    # Keep AF_UNIX paths short; this is a fresh mode-0700 fixture.
    short_root = Path('/var/tmp/files-filemanager')
    short_root.mkdir(mode=0o700, exist_ok=True)
    assert short_root.stat().st_uid == os.getuid()
    sockets = Path(tempfile.mkdtemp(prefix='pc-', dir=short_root))
    evidence = ROOT / 'docs/evidence/async-presentation-20261106/provenance'
    evidence.mkdir(parents=True, exist_ok=True)
    report = {'functional_gate': 'FAIL', 'fixture_dir': str(base), 'socket_fixture': str(sockets), 'checks': [],
              'sources': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in (Path(__file__).resolve(), SOURCE)}}
    proc = None
    sentinel = None
    try:
        a, b = socket.socketpair()
        c, d = socket.socketpair()
        for endpoint in (a, b, c, d): endpoint.settimeout(3)
        instance = proxy.Proxy(b, c)
        try:
            wire = (message(1, 1, number(4)) +
                    message(4, 0, number(7) + string('xdg_activation_v1') + number(1) + number(9)) +
                    message(9, 1, number(10)))
            a.sendall(wire[:13]); assert instance.from_client(); assert exact(d, 13) == wire[:13]
            a.sendall(wire[13:]); assert instance.from_client(); assert exact(d, len(wire)-13) == wire[13:]
            assert 10 in instance.tracker.tokens
            report['checks'].append('fragmented client framing identifies the activation token object')
            instance.tracker.armed = True
            done = message(10, 0, string('fixture-token'))
            deleted = message(1, 1, number(10))
            other = message(22, 1)
            d.sendall(done + deleted + other)
            assert instance.from_server() and exact(a, len(other)) == other
            assert instance.tracker.held == done and instance.tracker.held_delete == deleted
            instance.release()
            assert exact(a, len(done) + len(deleted)) == done + deleted
            assert 10 not in instance.tracker.tokens
            instance.tracker.armed = True
            reused = message(10, 0)
            d.sendall(reused); assert instance.from_server() and exact(a, len(reused)) == reused
            assert instance.tracker.held is None
            fragmented = message(23, 2, number(123))
            d.sendall(fragmented[:5]); assert instance.from_server()
            d.sendall(fragmented[5:]); assert instance.from_server()
            assert exact(a, len(fragmented)) == fragmented
            report['checks'].append('done and delete_id remain ordered while other events flow; recycled ID is retired')
            read_fd, write_fd = os.pipe()
            try:
                payload = message(30, 2)
                d.sendmsg([payload], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', [write_fd]))])
                assert instance.from_server()
                got, anc, flags, _ = a.recvmsg(32, socket.CMSG_SPACE(array.array('i').itemsize))
                assert got == payload and not flags and len(anc) == 1
                fds = array.array('i'); fds.frombytes(anc[0][2][:fds.itemsize])
                assert len(fds) == 1
                try:
                    os.write(fds[0], b'X'); assert os.read(read_fd, 1) == b'X'
                finally:
                    os.close(fds[0])
            finally:
                os.close(read_fd); os.close(write_fd)
            report['checks'].append('SCM_RIGHTS descriptor survives forwarding')
            instance.tracker.tokens.add(11); instance.tracker.armed = True
            read_fd, write_fd = os.pipe()
            try:
                d.sendmsg([message(11, 0, string('fixture-token'))],
                          [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', [write_fd]))])
                try:
                    instance.from_server()
                except ValueError as error:
                    assert str(error) == 'FDs share delayed Wayland packet'
                else:
                    raise AssertionError('ambiguous descriptors were not rejected')
            finally:
                os.close(read_fd); os.close(write_fd)
            report['checks'].append('FDs mixed with retained token event fail closed')
        finally:
            instance.close(); a.close(); d.close()

        # Existing listening socket must survive a failed bind of the proxy.
        existing = sockets / 'existing.sock'
        sentinel = socket.socket(socket.AF_UNIX); sentinel.bind(str(existing)); sentinel.listen(1)
        other_control = sockets / 'unused-control.sock'
        failed = subprocess.run([sys.executable, str(SOURCE), '--listen', str(existing),
                                 '--upstream', str(sockets / 'unused-upstream.sock'), '--control', str(other_control)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5, check=False)
        assert failed.returncode != 0 and existing.is_socket() and not other_control.exists()
        with socket.socket(socket.AF_UNIX) as probe:
            probe.settimeout(3); probe.connect(str(existing))
            peer, _ = sentinel.accept(); peer.close()
        report['checks'].append('failed bind leaves an existing socket untouched')
        listener, control_path = sockets / 'own.sock', sockets / 'control.sock'
        log = (base / 'proxy.log').open('w')
        try:
            proc = subprocess.Popen([sys.executable, str(SOURCE), '--listen', str(listener),
                                     '--upstream', str(sockets / 'unused-upstream.sock'), '--control', str(control_path)],
                                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and proc.poll() is None and not control_path.is_socket():
                time.sleep(.03)
            assert proc.poll() is None and listener.is_socket() and control_path.is_socket()
            assert control(control_path, 'stop') == {'ok': True}
            proc.wait(timeout=5)
            assert proc.returncode == 0 and not listener.exists() and not control_path.exists() and existing.is_socket()
        finally:
            if proc is not None and proc.poll() is None:
                proc.terminate(); proc.wait(timeout=5)
            log.close()
        report['checks'].append('normal cleanup removes only sockets owned by the proxy')
        report['functional_gate'] = 'PASS'
    except Exception as error:
        report['error'] = repr(error)
    finally:
        if sentinel is not None: sentinel.close()
        target = evidence / ('wayland-proxy-check-' + str(time.time_ns()) + '.json')
        target.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'functional_gate': report['functional_gate'], 'report': str(target), 'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
