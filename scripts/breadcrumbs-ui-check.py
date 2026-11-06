#!/usr/bin/env python3
"""Check long-path breadcrumbs through private Files fixture handlers.

Run under Xvfb or scripts/nested-run.py. The fixture opens the production
ancestor popup through its painted overflow hit and drives toolkit keys.
This verifies rendered app behavior, not native OS input delivery. HOME,
config, data and every browsed directory belong to a private fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'docs/evidence/0.0.4'
STATE = 'FILES_BREADCRUMB_STATE '
W, H = 1000, 650


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--animations', choices=['on', 'off'], default='off')
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    identity = str(time.time_ns())
    report_path = OUT / f'breadcrumbs-ui-{identity}.json'
    saved_log = OUT / f'breadcrumbs-ui-{identity}.txt'
    report = {
        'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED',
        'startup_gate': 'NOT_MEASURED', 'binary_path': str(binary),
        'method': 'Private painted breadcrumb hit and toolkit popup/key handlers with framebuffer captures; no native input',
        'log_path': str(saved_log), 'states': [], 'checks': [], 'captures': [],
        'viewport': [W, H], 'animations': args.animations,
    }
    process = None
    with tempfile.TemporaryDirectory(prefix='breadcrumbs-', dir=base_root) as tmp:
        base = Path(tmp)
        log = base / 'app.log'
        raw = base / 'frame.ppm'
        try:
            home = base / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
                (home / name).mkdir(parents=True)
            listing = (home / 'ancestor-with-a-long-readable-name' / 'second-long-ancestor'
                       / 'third-long-ancestor' / 'parent' / 'leaf')
            listing.mkdir(parents=True)
            fixture = listing / 'example.txt'
            fixture.write_bytes(b'Breadcrumb navigation fixture\n')
            original_hash = digest(fixture)
            (base / 'data').mkdir()
            (base / 'config').mkdir()
            devices = base / 'devices.json'
            devices.write_text('{"blockdevices": []}\n')
            shell = base / 'shell.toml'
            shell.write_text('[font]\nbase-size = 14\n')
            for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                              ('files-html-helper', 'html_helper_sha256')]:
                artifact = binary if name == 'files' else binary.parent / name
                if artifact.is_file():
                    report[key] = digest(artifact)
            env = {
                **os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
                'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
                'FILES_TEST_DEVICES_JSON': str(devices), 'FILES_TEST_CAPTURE': str(raw),
                'FILES_TEST_FIXED_SIZE': '1', 'FILES_TEST_ISOLATE_INPUT': '1',
                'FILES_TEST_TOOLKIT_POPUP': '1', 'FILES_TEST_HIDDEN_CONTEXT': '1',
                'FILES_CLOSE_TRACE': '1', 'OMARCHY_FLTK_ANIM': args.animations,
                'OMARCHY_FLTK_USER_SHELL': str(shell), 'TMPDIR': str(base),
                'MAGICK_TEMPORARY_PATH': str(base),
            }
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(listing), '--size', f'{W}x{H}', '--scale', '1',
                     '--no-session', '--ui-smoke', '--exit-after', '60'],
                    env=env, stdout=subprocess.DEVNULL, stderr=stream, start_new_session=True)

            def text():
                return log.read_text(errors='replace')

            def wait(check, reason, timeout=15):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = check()
                    if value:
                        return value
                    if process.poll() is not None:
                        raise RuntimeError(f'{reason}: app exit {process.returncode}')
                    time.sleep(.02)
                raise RuntimeError(f'{reason}: timed out')

            def send(action):
                before = len(text())
                pending = raw.with_suffix('.pending')
                pending.write_text(action)
                pending.replace(raw.with_suffix('.popup-command'))
                marker = f'FILES_TOOLKIT_FIXTURE action={action}'
                wait(lambda: marker in text()[before:].split('\n')[:-1], action)
                return before

            def state(label):
                before = send('breadcrumb-state')

                def complete():
                    for line in text()[before:].split('\n')[:-1]:
                        if line.startswith(STATE):
                            return json.loads(line[len(STATE):])
                    return None

                value = wait(complete, f'{label}: complete breadcrumb state')
                report['states'].append({'label': label, **value})
                return value

            def ready_path(path, label):
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    value = state(label)
                    if (value['path'] == str(path) and value.get('loaded') is True
                            and value.get('settled') is True and value.get('refreshing') is False
                            and value.get('visible') and value.get('popup_open') is False):
                        return value
                    time.sleep(.03)
                raise RuntimeError(f'{label}: loaded, settled path was not ready')

            def capture(label):
                raw.unlink(missing_ok=True)
                raw.with_suffix('.request').touch()

                def complete():
                    if not raw.exists():
                        return False
                    parts = raw.read_bytes().split(b'\n', 3)
                    return (len(parts) == 4 and parts[1] == f'{W} {H}'.encode()
                            and len(parts[3]) == W * H * 3)

                wait(complete, f'{label}: framebuffer')
                target = OUT / f'breadcrumbs-{label}-{identity}.png'
                subprocess.run(['magick', str(raw), str(target)], env=env, check=True, timeout=10)
                report['captures'].append(str(target))

            def geometry(value):
                area = value['omni_rect']
                assert len(area) == 4, area
                ax, ay, aw, ah = area
                assert aw > 0 and ah > 0 and ax >= 0 and ay >= 0 and ax + aw <= W and ay + ah <= H, area
                rows = value['visible']
                assert rows and rows[-1]['path'] == value['path'], value
                controls = [row['rect'] for row in rows]
                if value['overflow']:
                    assert value['overflow_rect'] is not None, value
                    controls.append(value['overflow_rect'])
                for rect in controls:
                    x, y, width, height = rect
                    assert width > 0 and height > 0, rect
                    assert ax <= x and ay <= y and x + width <= ax + aw and y + height <= ay + ah, (area, rect)
                paths = [row['path'] for row in value['overflow'] + rows]
                expected = {str(path) for path in [Path(value['path']), *Path(value['path']).parents]}
                assert set(paths) == expected and len(paths) == len(set(paths)), (paths, sorted(expected))
                assert not value['editing'] and not value['filter'], value

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
            ready_path(listing, 'loaded-initial')
            send('breadcrumb-select-file')
            initial = ready_path(listing, 'initial')
            assert initial['selected_raw'] == b'example.txt'.hex(), initial
            assert initial['selection_raw'] == [b'example.txt'.hex()], initial
            assert initial['anchor_raw'] == b'example.txt'.hex(), initial
            geometry(initial)
            assert initial['overflow'], initial
            assert len(initial['tabs']) == 1, initial
            capture('deep-path')
            report['checks'].append('deep path retains every ancestor including above HOME, with bounded painted hit rectangles')

            send('breadcrumb-open')
            pointer_open = state('pointer-open')
            assert pointer_open['popup_open'] is True, pointer_open
            capture('ancestor-menu')
            send('escape')
            escaped = ready_path(listing, 'escaped')
            assert escaped['generation'] == initial['generation'], (initial, escaped)
            assert escaped['focus_browser'] is True, escaped
            geometry(escaped)
            for key in ['selected_raw', 'selection_raw', 'anchor_raw']:
                assert escaped[key] == initial[key], (key, initial, escaped)
            report['checks'].append('Escape returns focus to the browser and preserves the current path, generation and selection')

            send('breadcrumb-key-open')
            keyboard_open = state('keyboard-open')
            assert keyboard_open['popup_open'] is True, keyboard_open
            send('escape')
            keyboard_escaped = ready_path(listing, 'keyboard-open-escaped')
            assert keyboard_escaped['focus_browser'] is True, keyboard_escaped
            for key in ['path', 'generation', 'active', 'selected_raw', 'selection_raw', 'anchor_raw']:
                assert keyboard_escaped[key] == escaped[key], (key, escaped, keyboard_escaped)
            assert keyboard_escaped['tabs'] == escaped['tabs'], (escaped, keyboard_escaped)
            report['checks'].append('Alt+Down opens the ancestor menu without pointer input and Escape restores the same browser context')

            expected_pick = escaped['overflow'][-1]['path']
            assert Path(expected_pick).is_relative_to(home), 'test must navigate only its owned HOME tree'
            send('breadcrumb-open')
            send('breadcrumb-pick-last')
            picked = ready_path(expected_pick, 'picked-hidden-ancestor')
            assert picked['active'] == initial['active'] and len(picked['tabs']) == 1, picked
            assert picked['focus_browser'] is True, picked
            geometry(picked)
            capture('ancestor-located')
            report['checks'].append('toolkit keyboard navigation picks the exact last hidden ancestor above the background submenu')

            send('breadcrumb-reset')
            ready_path(listing, 'loaded-reset')
            send('breadcrumb-select-file')
            reset = ready_path(listing, 'reset')
            assert reset['selected_raw'] == b'example.txt'.hex(), reset
            geometry(reset)
            hidden_background_path = reset['overflow'][-1]['path']
            assert Path(hidden_background_path).is_relative_to(home), 'test background tab must stay in its owned HOME tree'
            send('breadcrumb-open')
            send('breadcrumb-pick-background')
            hidden_background = ready_path(listing, 'background-hidden-ancestor')
            assert hidden_background['active'] == reset['active'], (reset, hidden_background)
            assert hidden_background['generation'] == reset['generation'], (reset, hidden_background)
            assert len(hidden_background['tabs']) == len(reset['tabs']) + 1, (reset, hidden_background)
            assert hidden_background['tabs'][-1]['path'] == hidden_background_path, (hidden_background_path, hidden_background)
            assert hidden_background['focus_browser'] is True, hidden_background
            for key in ['selected_raw', 'selection_raw', 'anchor_raw']:
                assert hidden_background[key] == reset[key], (key, reset, hidden_background)
            geometry(hidden_background)
            report['checks'].append('hidden-ancestor submenu opens the exact path in a background tab while preserving browser context')

            reset = hidden_background
            visible_ancestors = [row for row in reset['visible'] if row['path'] != reset['path']]
            assert visible_ancestors, reset
            expected_background = visible_ancestors[0]['path']
            assert Path(expected_background).is_relative_to(home), 'test visible ancestor must stay in its owned HOME tree'
            send('breadcrumb-middle-visible')
            background = ready_path(listing, 'background-visible-ancestor')
            assert background['active'] == reset['active'], (reset, background)
            assert background['generation'] == reset['generation'], (reset, background)
            assert len(background['tabs']) == len(reset['tabs']) + 1, (reset, background)
            assert background['tabs'][-1]['path'] == expected_background, (expected_background, background)
            assert background['focus_browser'] is True, background
            for key in ['selected_raw', 'selection_raw', 'anchor_raw']:
                assert background[key] == reset[key], (key, reset, background)
            geometry(background)
            report['checks'].append('middle-click on a visible ancestor opens a background tab without changing the active path')

            send('breadcrumb-middle-overflow')
            ignored = ready_path(listing, 'middle-overflow-ignored')
            assert ignored['tabs'] == background['tabs'] and ignored['active'] == background['active'], (background, ignored)
            assert ignored['generation'] == background['generation'], (background, ignored)
            report['checks'].append('middle-click on the overflow control never runs its primary popup/navigation action')

            assert digest(fixture) == original_hash, 'fixture contents changed'
            report['fixture_intact'] = True
            send('close')
            process.wait(timeout=15)
            assert process.returncode == 0, f'app exit {process.returncode}'
            assert 'FILES_CLOSE_DRAINED' in text(), 'normal close did not report drained workers'
            report.update(functional_gate='PASS', normal_exit=True)
        except Exception as error:
            report['error'] = repr(error)
        finally:
            if process is not None and process.poll() is None:
                stop_tree(process, None)
            report['exit_code'] = process.returncode if process is not None else None
            saved_log.write_text(log.read_text(errors='replace') if log.exists() else '')
            report_path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(report_path), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
