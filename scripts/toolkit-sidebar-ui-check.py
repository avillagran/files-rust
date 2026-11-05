#!/usr/bin/env python3
"""Toolkit Sidebar (places rail) in the Files window.

Fixture commands drive the toolkit's own handlers (expand, focus::click_at on
item rectangles); no OS input. HOME points into the fixture so the places
are fixture directories, and nothing outside the temporary tree is touched.
Devices come from a fixture `lsblk` (FILES_TEST_DEVICES_JSON) and `udisksctl`
is a recorder (FILES_TEST_UDISKSCTL): no real disk is mounted or ejected.
Run it under a private X server: xvfb-run -a scripts/toolkit-sidebar-ui-check.py
"""
import hashlib, json, os, re, stat, subprocess, tempfile, time
from pathlib import Path
from memcheck import stop_tree
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'docs/evidence/0.0.4'
BASE = Path.home() / '.cache/files-test'
OUT.mkdir(parents=True, exist_ok=True); BASE.mkdir(parents=True, exist_ok=True)
W, H = 1000, 650

STATE = re.compile(r'FILES_TOOLKIT_SIDEBAR expanded=(\w+) rail=(\w+) visible=(\w+) '
                   r'rect=\((-?\d+), (-?\d+), (-?\d+), (-?\d+)\) active=(\S+) path=(.*?) scroll=.*? rows=.*? keys=(.*)')

def main():
    result = {'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED',
              'method': 'Files window framebuffer + toolkit Sidebar expand / focus::click_at fixture; no native input',
              'captures': [], 'states': {}}
    process = None
    temporary = tempfile.TemporaryDirectory(prefix='toolkit-sidebar-', dir=BASE)
    base = Path(temporary.name); log = base/'app.log'; raw = base/'frame.ppm'
    try:
        home = base/'home'
        for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
            (home/name).mkdir(parents=True)
        (home/'Downloads'/'fixture-download.txt').write_text('fixture\n')
        directory = home/'files'; directory.mkdir(); (directory/'example.txt').write_text('fixture\n')
        config = base/'config'; config.mkdir()
        source = base/'outside'; source.mkdir(); dropped = source/'dropped.txt'; dropped.write_bytes(b'dropped on the rail\n')
        pen = base/'mnt/PENDRIVE'; pen.mkdir(parents=True); (pen/'foto.jpg').write_bytes(b'x')
        devices = {'blockdevices': [
            {'name': 'sdz', 'path': '/dev/sdz', 'type': 'disk', 'fstype': None, 'size': 32000000000, 'rm': True, 'hotplug': True, 'mountpoints': [None], 'children': [
                {'name': 'sdz1', 'path': '/dev/sdz1', 'type': 'part', 'fstype': 'exfat', 'label': 'PENDRIVE', 'size': 31000000000, 'rm': True, 'hotplug': True, 'mountpoints': [str(pen)]}]},
            {'name': 'sdy', 'path': '/dev/sdy', 'type': 'disk', 'fstype': None, 'size': 500000000000, 'rm': False, 'hotplug': False, 'mountpoints': [None], 'children': [
                {'name': 'sdy2', 'path': '/dev/sdy2', 'type': 'part', 'fstype': 'ext4', 'label': 'DATOS', 'size': 499000000000, 'rm': False, 'hotplug': False, 'mountpoints': [None]}]}]}
        lsblk = base/'lsblk.json'; lsblk.write_text(json.dumps(devices))
        calls = base/'udisks-calls'; tool = base/'udisksctl'
        tool.write_text(f'#!/bin/sh\necho "$@" >> {calls}\nexit 0\n')
        tool.chmod(tool.stat().st_mode | stat.S_IEXEC)
        shell = base/'shell.toml'; shell.write_text('[font]\nbase-size = 18\n')
        env = {**os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base/'xdg-config'),
               'XDG_DATA_HOME': str(base/'xdg-data'), 'FILES_CONFIG_DIR': str(config),
               'FILES_TEST_DEVICES_JSON': str(lsblk), 'FILES_TEST_UDISKSCTL': str(tool),
               'FILES_TEST_HIDDEN_CONTEXT': '1', 'FILES_TEST_SIDEBAR_DROP': str(dropped),
               'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_FIXED_SIZE': '1', 'FILES_TEST_ISOLATE_INPUT': '1',
               'FILES_TEST_TOOLKIT_POPUP': '1', 'OMARCHY_FLTK_ANIM': 'off', 'FILES_CLOSE_TRACE': '1',
               'OMARCHY_FLTK_USER_SHELL': str(shell), 'TMPDIR': str(base), 'MAGICK_TEMPORARY_PATH': str(base)}
        with log.open('w') as stream:
            process = subprocess.Popen([str(ROOT/'target/release/files'), str(directory), '--size', f'{W}x{H}',
                '--scale', '1', '--ui-smoke', '--exit-after', '30'], stderr=stream, stdout=subprocess.DEVNULL,
                env=env, start_new_session=True)
        def wait(check, reason, timeout=12):
            until = time.monotonic()+timeout
            while time.monotonic() < until:
                if check(): return
                if process.poll() is not None: raise RuntimeError(f'{reason}: exit {process.returncode}')
                time.sleep(.03)
            raise RuntimeError(reason)
        wait(lambda: 'FILES_UI_SMOKE_OK' in log.read_text(), 'smoke')
        def command(action, marker=None):
            marker = marker or f'FILES_TOOLKIT_FIXTURE action={action}'
            before = len(log.read_text()); tmp = raw.with_suffix('.pending'); tmp.write_text(action)
            os.replace(tmp, raw.with_suffix('.popup-command'))
            wait(lambda: marker in log.read_text()[before:], f'{action}: {marker}')
            return log.read_text()[before:]
        def state(label):
            time.sleep(.15)  # one paint: the canvas syncs the rail before drawing it
            command('sidebar-state'); time.sleep(.15)
            m = None
            for m in STATE.finditer(command('sidebar-state')): pass
            if not m: raise RuntimeError('no sidebar state')
            s = {'expanded': m[1] == 'true', 'rail': m[2] == 'true', 'visible': m[3] == 'true',
                 'rect': [int(m[i]) for i in range(4, 8)], 'active': m[8], 'path': m[9], 'keys': m[10].split('|')}
            result['states'][label] = s
            return s
        def capture(label):
            raw.unlink(missing_ok=True); raw.with_suffix('.request').touch()
            def ready():
                if not raw.exists(): return False
                parts = raw.read_bytes().split(b'\n', 3)
                return len(parts) == 4 and parts[1] == f'{W} {H}'.encode() and len(parts[3]) == W*H*3
            wait(ready, 'framebuffer dimensions')
            target = OUT/f'toolkit-sidebar-{label}-{time.time_ns()}.png'
            subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=10)
            result['captures'].append(str(target))
            return target

        s = state('start')
        assert s['visible'] and not s['expanded'] and not s['rail'], s
        assert s['rect'][:3] == [0, 90, 52] and s['rect'][3] > 0, s
        assert s['keys'][0] == 'fav' and s['keys'][-1] == 'trash', s
        downloads = f'place:{home/"Downloads"}'
        assert downloads in s['keys'], (downloads, s['keys'])
        collapsed = capture('collapsed')

        command('sidebar-expand')
        s = state('expanded')
        assert s['expanded'] and s['rail'] and s['rect'][2] == 170, s
        expanded = capture('expanded')
        assert collapsed.read_bytes() != expanded.read_bytes(), 'expanding did not change the frame'

        click = command(f'sidebar-click:{downloads}', f'FILES_TOOLKIT_SIDEBAR_CLICK key={downloads}')
        assert 'rect=Some(' in click, click
        wait(lambda: str(home/'Downloads') in (state('downloads')['path']), 'navigate to Downloads')
        s = result['states']['downloads']
        assert s['path'] == str(home/'Downloads') and s['active'] == downloads, s
        capture('downloads')

        # Devices: the mounted stick has the eject button (recorder only).
        s = result['states']['downloads']
        assert {'h:devices', 'dev:/dev/sdz1', 'dev:/dev/sdy2'} <= set(s['keys']), s['keys']
        click = command('sidebar-click:dev:/dev/sdz1', 'FILES_TOOLKIT_SIDEBAR_CLICK key=dev:/dev/sdz1')
        assert 'rect=Some(' in click, click
        wait(lambda: state('device')['path'] == str(pen), 'open the mounted stick')
        assert result['states']['device']['active'] == 'dev:/dev/sdz1', result['states']['device']
        capture('device')
        click = command('sidebar-secondary:dev:/dev/sdz1', 'FILES_TOOLKIT_SIDEBAR_CLICK key=dev:/dev/sdz1')
        assert 'rect=Some(' in click, click
        wait(lambda: calls.exists() and '/dev/sdz1' in calls.read_text(), 'eject reached udisksctl')
        result['udisks_calls'] = calls.read_text().splitlines()
        assert any(c.startswith('unmount') for c in result['udisks_calls']), result['udisks_calls']

        # Context menu of a place: End + Enter picks its last entry, "Abrir en
        # pestaña nueva" (a new tab, then the place).
        documents = f'place:{home/"Documents"}'
        opened = command(f'sidebar-context:{documents}', 'FILES_TOOLKIT_SIDEBAR_MENU')
        assert 'entries=2' in opened, opened
        time.sleep(.1); capture('context-menu')
        command('end'); picked = command('pick', 'FILES_TOOLKIT_SIDEBAR_PICK')
        assert 'entry=1' in picked and 'TabNew' in picked, picked
        tabs = command('sidebar-tabs', 'FILES_TOOLKIT_SIDEBAR_TABS')
        assert re.search(r'count=2 path=' + re.escape(str(home/'Documents')), tabs), tabs
        result['context_menu'] = {'row': documents, 'picked': 'Abrir en pestaña nueva', 'tabs': 2}

        # A file dropped on a rail row (private-pipe fixture through Files'
        # native transfer): the copy review targets that row's folder.
        hover = command(f'sidebar-hover:{documents}', 'FILES_TOOLKIT_SIDEBAR_DROP')
        assert 'result=Ok(())' in hover and f'hover=Some("{home/"Documents"}")' in hover, hover
        capture('drop-hover')
        drop = command(f'sidebar-drop:{documents}', 'FILES_TOOLKIT_SIDEBAR_DROP')
        assert 'result=Ok(())' in drop, drop
        def form():
            text = command('sidebar-form', 'FILES_TOOLKIT_SIDEBAR_FORM')
            return re.search(r'FILES_TOOLKIT_SIDEBAR_FORM open=true destination=(.*?) names=(\d+)', text)
        f = None
        until = time.monotonic() + 10
        while time.monotonic() < until and not (f := form()): time.sleep(.1)
        assert f and f[1] == str(home/'Documents') and f[2] == '1', f and f.groups()
        assert not (home/'Documents'/'dropped.txt').exists(), 'review must not copy yet'
        capture('drop-review')
        command('sidebar-confirm')
        wait(lambda: (home/'Documents'/'dropped.txt').exists() and
             (home/'Documents'/'dropped.txt').read_bytes() == b'dropped on the rail\n', 'copy into Documents')
        assert dropped.read_bytes() == b'dropped on the rail\n'
        result['drop'] = {'row': documents, 'destination': str(home/'Documents'), 'copied': True, 'source_intact': True}

        command('sidebar-click:trash', 'FILES_TOOLKIT_SIDEBAR_CLICK key=trash')
        command('trash-state', 'FILES_TRASH_BROWSER open=true')
        s = state('trash')
        assert not s['visible'], 'rail must hide under the trash browser'
        command('trash-close', 'FILES_TRASH_BROWSER open=false')
        s = state('trash-closed')
        assert s['visible'], s

        # Bookmarks: add Descargas and Música from the Favoritos row menu
        # (End + Enter = "Añadir esta carpeta a favoritos"), then drag Música
        # above Descargas; the saved order follows.
        def bookmark_here(place):
            command(f'sidebar-click:{place}', f'FILES_TOOLKIT_SIDEBAR_CLICK key={place}')
            wait(lambda: state('nav')['path'] == place.split(':', 1)[1], 'navigate ' + place)
            command('sidebar-context:fav', 'FILES_TOOLKIT_SIDEBAR_MENU')
            command('end'); picked = command('pick', 'FILES_TOOLKIT_SIDEBAR_PICK')
            assert 'BookmarkAdd' in picked, picked
            wait(lambda: f'mark:{place.split(":", 1)[1]}' in state('marks')['keys'], 'bookmark ' + place)
        music = f'place:{home/"Music"}'
        bookmark_here(downloads); bookmark_here(music)
        mark_d, mark_m = f'mark:{home/"Downloads"}', f'mark:{home/"Music"}'
        keys = state('marks-before')['keys']
        assert keys.index(mark_d) < keys.index(mark_m), keys
        capture('bookmarks-before')
        moved = command(f'sidebar-drag:{mark_m}|{mark_d}', 'FILES_TOOLKIT_SIDEBAR_DRAG')
        assert 'found=true' in moved and f'FILES_TOOLKIT_SIDEBAR_MOVE key={mark_m} index=0 requested=true' in log.read_text(), moved
        time.sleep(.5)
        keys = state('marks-after')['keys']
        assert keys.index(mark_m) < keys.index(mark_d), keys
        capture('bookmarks-after')
        result['bookmark_reorder'] = {'moved': mark_m, 'before': mark_d, 'saved_order': [k for k in keys if k.startswith('mark:')]}

        command('sidebar-collapse')
        s = state('collapsed-again')
        assert not s['expanded'] and not s['rail'] and s['rect'][2] == 52, s

        command('close')
        wait(lambda: process.poll() is not None, 'normal close', timeout=15)
        assert process.returncode == 0 and 'FILES_CLOSE_DRAINED' in log.read_text()
        result.update(functional_gate='PASS', normal_exit=True, viewport=[W, H])
    except BaseException as error:
        result['error'] = repr(error)
    finally:
        if process is not None and process.poll() is None: stop_tree(process, None)
        if log.exists(): result['log'] = log.read_text()[-20000:]
        for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'), ('files-html-helper', 'html_helper_sha256')]:
            path = ROOT/'target/release'/name
            if path.exists(): result[key] = hashlib.sha256(path.read_bytes()).hexdigest()
        report = OUT/f'toolkit-sidebar-ui-{time.time_ns()}.json'
        with report.open('x') as stream: json.dump(result, stream, indent=2)
        temporary.cleanup(); print(json.dumps({'report': str(report), 'functional_gate': result['functional_gate'],
                                               'error': result.get('error')}))
    return 0 if result['functional_gate'] == 'PASS' else 1
if __name__ == '__main__': raise SystemExit(main())
