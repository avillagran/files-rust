#!/usr/bin/env python3
"""Private Wayland test proxy that can delay one activation-token done event.

Never logs wire payloads. Control protocol is newline-delimited JSON with
commands arm, release and status. Limits and ambiguous FD/control cases fail
closed. Run only against a compositor and clients owned by the test.
"""
import argparse
import array
import json
import os
from pathlib import Path
import selectors
import socket
import struct
import sys

MAX_BUFFER = 1024 * 1024
MAX_FDS = 64
MAX_MESSAGE = 65532


def frames(data):
    offset = 0
    while len(data) - offset >= 8:
        obj, word = struct.unpack_from('=II', data, offset)
        length, opcode = word >> 16, word & 0xffff
        if not obj or length < 8 or length % 4 or length > MAX_MESSAGE:
            raise ValueError('invalid Wayland message header')
        if len(data) - offset < length:
            break
        yield obj, opcode, data[offset:offset + length]
        offset += length


def string_at(message, offset):
    if offset + 4 > len(message):
        raise ValueError('missing string length')
    length = struct.unpack_from('=I', message, offset)[0]
    end = offset + 4 + length
    if not length or end > len(message) or message[end - 1] != 0:
        raise ValueError('invalid string')
    return message[offset + 4:end - 1], (end + 3) & ~3


class Tracker:
    def __init__(self):
        self.registries = set()
        self.activations = set()
        self.tokens = set()
        self.armed = False
        self.held = None
        self.held_object = None
        self.held_delete = None
        self.seen = 0
        self.released = 0
        self.activate_requests = 0

    def client(self, obj, opcode, msg):
        if obj == 1 and opcode == 1:  # wl_display.get_registry(new_id)
            if len(msg) != 12: raise ValueError('invalid get_registry')
            self.registries.add(struct.unpack_from('=I', msg, 8)[0])
        elif obj in self.registries and opcode == 0:  # wl_registry.bind
            name, pos = string_at(msg, 12)
            if pos + 8 != len(msg): raise ValueError('invalid bind')
            new_id = struct.unpack_from('=I', msg, pos + 4)[0]
            if name == b'xdg_activation_v1': self.activations.add(new_id)
        elif obj in self.activations and opcode == 1:  # get_activation_token(new_id)
            if len(msg) != 12: raise ValueError('invalid get_activation_token')
            self.tokens.add(struct.unpack_from('=I', msg, 8)[0])
        elif obj in self.activations and opcode == 2:  # activate(token, surface)
            self.activate_requests += 1

    def should_hold(self, obj, opcode, msg):
        if obj in self.tokens and opcode == 0:  # xdg_activation_token_v1.done
            self.seen += 1
            if self.armed:
                if self.held is not None: raise ValueError('more than one delayed token')
                self.held = msg
                self.held_object = obj
                self.armed = False
                return True
        if obj == 1 and opcode == 1 and self.held_object is not None:  # wl_display.delete_id
            if len(msg) != 12: raise ValueError('invalid delete_id')
            if struct.unpack_from('=I', msg, 8)[0] == self.held_object:
                if self.held_delete is not None: raise ValueError('duplicate delayed delete_id')
                self.held_delete = msg
                return True
        return False

    def status(self):
        return {'armed': self.armed, 'held': self.held is not None, 'delete_held': self.held_delete is not None,
                'token_done_seen': self.seen, 'released': self.released, 'activate_requests': self.activate_requests}

    def retire(self, msg):
        obj, word = struct.unpack_from('=II', msg)
        if obj == 1 and (word & 0xffff) == 1:
            if len(msg) != 12: raise ValueError('invalid delete_id')
            deleted = struct.unpack_from('=I', msg, 8)[0]
            self.tokens.discard(deleted)
            self.activations.discard(deleted)
            self.registries.discard(deleted)


def send_packet(sock, payload, fds=()):
    ancillary = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', fds))] if fds else []
    while True:
        try:
            count = sock.sendmsg([payload], ancillary)
            break
        except InterruptedError:
            continue
    if count != len(payload):
        # Partial sends with rights cannot safely be repeated; close connection.
        raise OSError('partial Wayland send')


def receive(sock):
    data, anc, flags, _ = sock.recvmsg(65536, socket.CMSG_SPACE(MAX_FDS * array.array('i').itemsize))
    fds = []
    for level, kind, payload in anc:
        if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
            raise ValueError('unexpected ancillary data')
        values = array.array('i')
        values.frombytes(payload[:len(payload) - len(payload) % values.itemsize])
        fds.extend(values)
    if flags & (socket.MSG_CTRUNC | socket.MSG_TRUNC) or len(fds) > MAX_FDS:
        for fd in fds: os.close(fd)
        raise ValueError('truncated or excessive ancillary data')
    return data, fds


class Proxy:
    def __init__(self, client, server):
        self.client_socket, self.server_socket = client, server
        self.tracker = Tracker()
        self.client_buffer = b''
        self.server_buffer = b''
        self.server_fds = []

    def from_client(self):
        data, fds = receive(self.client_socket)
        if not data:
            for fd in fds: os.close(fd)
            return False
        try:
            send_packet(self.server_socket, data, fds)
        finally:
            for fd in fds: os.close(fd)
        self.client_buffer += data
        if len(self.client_buffer) > MAX_BUFFER: raise ValueError('client inspection buffer limit')
        parsed = list(frames(self.client_buffer))
        for obj, opcode, msg in parsed: self.tracker.client(obj, opcode, msg)
        self.client_buffer = self.client_buffer[sum(len(msg) for _, _, msg in parsed):]
        return True

    def from_server(self):
        data, fds = receive(self.server_socket)
        if not data:
            for fd in fds: os.close(fd)
            return False
        self.server_buffer += data
        self.server_fds.extend(fds)
        if len(self.server_buffer) > MAX_BUFFER or len(self.server_fds) > MAX_FDS:
            raise ValueError('server buffer limit')
        parsed = list(frames(self.server_buffer))
        if not parsed: return True
        count = sum(len(msg) for _, _, msg in parsed)
        held = []
        forwarded = []
        for obj, opcode, msg in parsed:
            if self.tracker.should_hold(obj, opcode, msg): held.append(msg)
            else:
                self.tracker.retire(msg)
                forwarded.append(msg)
        # No known FD-bearing message may share a withheld packet; the byte/FD
        # association is otherwise ambiguous. Leave the connection untouched.
        if held and self.server_fds: raise ValueError('FDs share delayed Wayland packet')
        if forwarded:
            try:
                send_packet(self.client_socket, b''.join(forwarded), self.server_fds)
            finally:
                for fd in self.server_fds: os.close(fd)
                self.server_fds.clear()
        self.server_buffer = self.server_buffer[count:]
        return True

    def release(self):
        if self.tracker.held is None: raise ValueError('no delayed token event')
        send_packet(self.client_socket, self.tracker.held)
        if self.tracker.held_delete is not None:
            send_packet(self.client_socket, self.tracker.held_delete)
            self.tracker.retire(self.tracker.held_delete)
        self.tracker.held = None
        self.tracker.held_object = None
        self.tracker.held_delete = None
        self.tracker.released += 1

    def close(self):
        for fd in self.server_fds: os.close(fd)
        self.server_fds.clear()
        self.client_socket.close()
        self.server_socket.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--listen', required=True, type=Path)
    ap.add_argument('--upstream', required=True, type=Path)
    ap.add_argument('--control', required=True, type=Path)
    args = ap.parse_args()
    assert args.listen.is_absolute() and args.upstream.is_absolute() and args.control.is_absolute()
    assert args.listen != args.upstream and args.control not in (args.listen, args.upstream)
    listener, control = socket.socket(socket.AF_UNIX), socket.socket(socket.AF_UNIX)
    proxy = None
    bound = []
    selector = selectors.DefaultSelector()
    try:
        listener.bind(str(args.listen)); bound.append(args.listen); os.chmod(args.listen, 0o600); listener.listen(1)
        control.bind(str(args.control)); bound.append(args.control); os.chmod(args.control, 0o600); control.listen(8)
        selector.register(listener, selectors.EVENT_READ, 'accept')
        selector.register(control, selectors.EVENT_READ, 'control')
        while True:
            for key, _ in selector.select(timeout=1):
                if key.data == 'accept':
                    if proxy is not None: raise ValueError('multiple clients refused')
                    client, _ = listener.accept()
                    server = socket.socket(socket.AF_UNIX); server.connect(str(args.upstream))
                    proxy = Proxy(client, server)
                    selector.register(client, selectors.EVENT_READ, 'client')
                    selector.register(server, selectors.EVENT_READ, 'server')
                elif key.data == 'control':
                    peer, _ = control.accept()
                    with peer:
                        peer.settimeout(2)
                        command = peer.recv(256).decode('ascii').strip()
                        if command == 'status': response = proxy.tracker.status() if proxy else {'connected': False}
                        elif command == 'arm' and proxy:
                            if proxy.tracker.held is not None: raise ValueError('token already held')
                            proxy.tracker.armed = True; response = proxy.tracker.status()
                        elif command == 'release' and proxy:
                            proxy.release(); response = proxy.tracker.status()
                        elif command == 'stop':
                            peer.sendall(b'{"ok":true}\n'); return 0
                        else: response = {'error': 'invalid command'}
                        peer.sendall((json.dumps(response) + '\n').encode())
                elif key.data == 'client':
                    if not proxy.from_client(): return 0
                elif key.data == 'server':
                    if not proxy.from_server(): return 0
    except (OSError, ValueError, UnicodeError):
        # Wire bytes and exception details are intentionally never logged.
        print('Wayland proxy failed closed', file=sys.stderr)
        return 1
    finally:
        selector.close()
        if proxy: proxy.close()
        listener.close(); control.close()
        # Socket paths are unique private fixture entries; no tree deletion.
        for path in bound:
            if path.is_socket(): path.unlink()


if __name__ == '__main__':
    raise SystemExit(main())
