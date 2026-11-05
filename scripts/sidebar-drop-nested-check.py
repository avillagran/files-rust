#!/usr/bin/env python3
"""A REAL Wayland drag and drop onto a row of the toolkit Sidebar (places rail).

Everything runs inside a private nested Hyprland on a hidden special workspace
of the current session (the same harness as preview-copy-nested-check.py),
stopped at the end. Two Files windows: A lists a source folder, B is the
target. The virtual pointer (scripts/tools/vpointer.c, refuses wayland-0/1)
presses on A's file, drags it over B's "Documentos" rail row and releases.
B must open its copy review for that row's folder; the fixture command
'sidebar-confirm' then starts the copy, and the bytes are compared.

HOME of both windows is inside the temporary tree, so the places are fixture
folders and nothing outside it is written.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
EVIDENCE = ROOT / 'docs/evidence/sidebar-drop-20261105'
spec = importlib.util.spec_from_file_location('nested', ROOT / 'scripts/preview-copy-nested-check.py')
nested_harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nested_harness)
hypr, build_pointer, start_nested = nested_harness.hypr, nested_harness.build_pointer, nested_harness.start_nested
PAYLOAD = b'arrastrado de verdad\n'


def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    ap.add_argument('--label', default='source')
    ap.add_argument('--onto', choices=['rail', 'body'], default='rail',
                    help='drop on the Documentos rail row (default) or on the target listing (control)')
    args = ap.parse_args()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    report = {'functional_gate': 'FAIL', 'label': args.label, 'binary_sha256': digest(args.binary),
              'method': 'private nested Hyprland on a hidden special workspace; real Wayland DnD from a Files '
                        'listing to another Files window\'s Sidebar row with the virtual pointer',
              'checks': {}, 'captures': [], 'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED'}
    out = EVIDENCE / f'{args.label}-{time.time_ns()}.json'
    processes = []
    nested_pid = None
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix='sidebar-drop-', dir=base_root) as tmp:
            tmp = Path(tmp)
            pointer = build_pointer(tmp)
            socket, signature, nested_pid = start_nested(tmp, report)
            assert socket != report['parent_socket'], 'refusing the session socket'
            nested = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
            monitor = json.loads(hypr(['monitors', '-j'], nested))[0]
            extent = (round(monitor['width'] / monitor['scale']), round(monitor['height'] / monitor['scale']))
            home = tmp / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'target']:
                (home / name).mkdir(parents=True)
            source = tmp / 'source'
            source.mkdir()
            dragged = source / 'arrastrado.txt'
            dragged.write_bytes(PAYLOAD)

            def launch(name, folder):
                capture = tmp / f'{name}.ppm'
                log = tmp / f'{name}.log'
                env = {**nested, 'HOME': str(home), 'XDG_CONFIG_HOME': str(tmp / f'{name}-xdg-config'),
                       'XDG_DATA_HOME': str(tmp / f'{name}-data'), 'FILES_CONFIG_DIR': str(tmp / f'{name}-config'),
                       'FILES_TEST_CAPTURE': str(capture), 'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_DND_TRACE': '1',
                       'TMPDIR': str(tmp), 'MAGICK_TEMPORARY_PATH': str(tmp)}
                with log.open('w') as output:
                    p = subprocess.Popen([str(args.binary), str(folder), '--size', '900x600', '--scale', '1',
                                          '--no-session', '--ui-smoke', '--exit-after', '90'],
                                         env=env, stdout=subprocess.DEVNULL, stderr=output, start_new_session=True)
                processes.append(p)
                return {'process': p, 'log': log, 'capture': capture, 'env': env}

            def wait(check, reason, timeout=15):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    v = check()
                    if v:
                        return v
                    for p in processes:
                        if p.poll() is not None:
                            raise AssertionError(f'{reason}; a Files window exited {p.returncode}')
                    time.sleep(.05)
                raise AssertionError(reason)

            def command(w, action, marker):
                before = len(w['log'].read_text())
                pending = w['capture'].with_suffix('.pending')
                pending.write_text(action)
                os.replace(pending, w['capture'].with_suffix('.popup-command'))
                wait(lambda: marker in w['log'].read_text()[before:], f'{action}: {marker}')
                return w['log'].read_text()[before:]

            def shot(w, label):
                capture = w['capture']
                capture.unlink(missing_ok=True)
                capture.with_suffix('.request').touch()

                def complete():
                    if not capture.exists():
                        return False
                    parts = capture.read_bytes().split(b'\n', 3)
                    if len(parts) != 4:
                        return False
                    dims = [int(v) for v in parts[1].split()]
                    return len(dims) == 2 and len(parts[3]) == dims[0] * dims[1] * 3
                wait(complete, 'capture ' + label)
                png = EVIDENCE / f'{args.label}-{label}-{time.time_ns()}.png'
                subprocess.run(['magick', str(capture), str(png)], check=True, env=w['env'], timeout=20)
                report['captures'].append(str(png))

            b = launch('target', home / 'target')
            wait(lambda: 'FILES_UI_SMOKE_OK' in b['log'].read_text(), 'target smoke')
            a = launch('source', source)
            wait(lambda: 'FILES_UI_SMOKE_OK' in a['log'].read_text(), 'source smoke')
            time.sleep(1.0)

            def client(p):
                return [c for c in json.loads(hypr(['clients', '-j'], nested)) if c.get('pid') == p.pid]
            wait(lambda: client(a['process']) and client(b['process']), 'windows')
            report['nested_extent'] = extent
            # Float both at a usable size: B over A's right side, A's first
            # entry and B's rail both visible.
            width = min(760, extent[0] - 340)
            height = min(560, extent[1] - 60)
            for p, (x, y) in [(a['process'], (10, 10)), (b['process'], (330, 40))]:
                addr = 'address:' + client(p)[0]['address']
                for d in (['dispatch', 'setfloating', addr], ['dispatch', 'resizewindowpixel', f'exact {width} {height},{addr}'],
                          ['dispatch', 'movewindowpixel', f'exact {x} {y},{addr}']):
                    report.setdefault('dispatch', []).append(hypr(d, nested).strip())
            hypr(['dispatch', 'focuswindow', 'address:' + client(b['process'])[0]['address']], nested)
            time.sleep(1.0)
            ca = client(a['process'])[0]
            cb = client(b['process'])[0]
            report['windows'] = {'source': [ca['at'], ca['size']], 'target': [cb['at'], cb['size']]}

            state = command(b, 'sidebar-state', 'FILES_TOOLKIT_SIDEBAR ')
            line = [l for l in state.splitlines() if l.startswith('FILES_TOOLKIT_SIDEBAR ')][-1]
            keys = line.split(' keys=')[1].split('|')
            rows = re.findall(r'(None|Some\(\((-?\d+), (-?\d+), (-?\d+), (-?\d+)\)\))', line.split(' rows=')[1].split(' keys=')[0])
            documents = f'place:{home / "Documents"}'
            index = keys.index(documents)
            row = rows[index]
            assert row[0] != 'None', 'Documentos row not in view'
            rx, ry, rw, rh = map(int, row[1:])
            target = (cb['at'][0] + rx + min(rw, 52) // 2, cb['at'][1] + ry + rh // 2)
            folder = home / 'Documents'
            if args.onto == 'body':
                target = (cb['at'][0] + 250, cb['at'][1] + 200)
                folder = home / 'target'
            report['onto'] = args.onto
            # The first entry of A's listing (Groups view): left of the inspector, first row.
            start = (ca['at'][0] + 150, ca['at'][1] + 139)
            report['checks']['drag'] = {'from': start, 'to': target, 'row': documents}
            shot(b, 'target-before')
            subprocess.run([str(pointer), socket, str(extent[0]), str(extent[1]), str(start[0]), str(start[1]),
                            str(target[0]), str(target[1]), 'd800'], check=True, timeout=20)
            time.sleep(1.0)
            shot(a, 'source-after-drag')

            def form():
                text = command(b, 'sidebar-form', 'FILES_TOOLKIT_SIDEBAR_FORM')
                return re.search(r'FILES_TOOLKIT_SIDEBAR_FORM open=true destination=(.*?) names=(\d+)', text)
            f = None
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline and not (f := form()):
                time.sleep(.2)
            report['checks']['target_log_tail'] = [l for l in b['log'].read_text().splitlines()
                                                   if 'SIDEBAR_FORM' in l][-1:]
            report['checks']['logs'] = {n: [l for l in w['log'].read_text().splitlines()
                                            if re.search('DND_TRACE|SIDEBAR_DROP', l)][-12:]
                                        for n, w in [('source', a), ('target', b)]}
            assert f, 'no copy review in the target window'
            report['checks']['review'] = {'destination': f[1], 'names': int(f[2])}
            assert f[1] == str(folder) and f[2] == '1', f.groups()
            assert not (folder / dragged.name).exists(), 'the review must not copy yet'
            shot(b, 'target-review')
            command(b, 'sidebar-confirm', 'FILES_TOOLKIT_FIXTURE action=sidebar-confirm')
            wait(lambda: (folder / dragged.name).exists()
                 and (folder / dragged.name).read_bytes() == PAYLOAD, f'copy into {folder.name}')
            assert dragged.read_bytes() == PAYLOAD, 'source changed'
            report['checks']['copied'] = True
            report['checks']['source_intact'] = True
            report['functional_gate'] = 'PASS'
    except Exception as e:
        report['error'] = repr(e)
    finally:
        for p in processes:
            if p.poll() is None:
                p.kill()
                p.wait()
        for signal in (15, 9):
            alive = []
            for pid in nested_pid or []:
                try:
                    os.kill(int(pid), signal)
                    alive.append(pid)
                except (OSError, ValueError):
                    pass
            deadline = time.monotonic() + 10
            while alive and time.monotonic() < deadline:
                alive = [pid for pid in alive if Path(f'/proc/{pid}').exists()]
                time.sleep(.2)
            if not alive:
                break
        report['nested_stopped'] = not any(Path(f'/proc/{pid}').exists() for pid in nested_pid or [])
    out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    print(report['functional_gate'], out, report.get('error', ''))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
