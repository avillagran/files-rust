#!/usr/bin/env python3
"""Accept rendered keyed-tab indicator motion in a private Files window.

The app samples its frozen toolkit clock. This driver checks actual accent
pixels, reversals and settled reduced-motion equality in full PNG frames.
It does not claim native input delivery or partial-damage screen capture.
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
MARKER = 'FILES_TAB_WIDGET_MOTION '


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pixels(path, rect, env):
    x, y, width, height = rect
    data = subprocess.check_output(
        ['magick', str(path), '-crop', f'{width}x{height}+{x}+{y}', '+repage',
         '-alpha', 'off', '-depth', '8', 'rgb:-'], env=env, timeout=15)
    assert len(data) == width * height * 3, (path, rect, len(data))
    return data


def inspect_motion(motion, shots, env):
    assert motion.get('ok') is True, motion
    captures = motion['captures']
    assert len(captures) == 8 and len({c['label'] for c in captures}) == 8
    accent = motion['accent_rgb']
    strip = motion['strip_rect']
    y = motion['underline_y']
    rows, crops = {}, {}
    for capture in captures:
        path = Path(capture['path'])
        assert path.is_absolute() and path.parent == shots and path.is_file(), path
        crop = pixels(path, strip, env)
        crops[capture['label']] = crop
        offset = (y - strip[1]) * strip[2] * 3
        row = crop[offset:offset + strip[2] * 3]
        hits = [i for i in range(strip[2])
                if all(abs(row[3 * i + c] - accent[c]) <= 2 for c in range(3))]
        assert hits, ('accent underline absent', capture)
        runs = []
        for hit in hits:
            if runs and hit == runs[-1][-1] + 1:
                runs[-1].append(hit)
            else:
                runs.append([hit])
        assert len(runs) == 1 and len(runs[0]) >= 20, ('multiple/residual underlines', capture, runs)
        rows[capture['label']] = [strip[0] + runs[0][0], strip[0] + runs[0][-1] + 1]

    def endpoint(label):
        capture = next(c for c in captures if c['label'] == label)
        x, _, width, _ = capture['target_rect']
        assert all(abs(a - b) <= 1 for a, b in zip(rows[label], [x, x + width])), (label, rows[label], capture)

    for label in ['first-settled', 'last-settled', 'reversed-settled', 'disabled-last']:
        endpoint(label)
    first, last = rows['first-settled'][0], rows['last-settled'][0]
    assert last > first + 40, rows
    assert abs(rows['forward-input'][0] - first) <= 1, rows
    assert first + 1 < rows['forward-64'][0] < last - 1, rows
    assert rows['forward-input'][0] < rows['forward-64'][0] < last, rows
    assert first + 1 < rows['reverse-48'][0] < last - 1, rows
    # A 16 ms frame elapses before retargeting. It may continue toward the
    # old target briefly, but must not jump to either endpoint.
    reverse, resumed = rows['reverse-48'][0], rows['reversed-input'][0]
    assert first < resumed <= reverse + 1, rows
    assert abs(resumed - reverse) < (last - first) / 3, rows
    assert resumed < rows['reversed-settled'][0], rows
    assert crops['last-settled'] == crops['reversed-settled'] == crops['disabled-last'], 'settled ON/OFF strip pixels differ'
    assert motion['stable_ids'] and motion['restored_active'] and motion['restored_policy'], motion
    return {'underline_bounds': rows, 'single_accent_run': True,
            'sampled_forward_order': True, 'intermediate_frames': True,
            'rapid_reversal': True, 'settled_on_equals_off': True,
            'disabled_target_in_input_frame': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    identity = str(time.time_ns())
    shots = OUT / f'tab-strip-motion-{identity}-shots'
    shots.mkdir()
    saved_log = OUT / f'tab-strip-motion-ui-{identity}.txt'
    target = OUT / f'tab-strip-motion-ui-{identity}.json'
    report = {'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED',
              'startup_gate': 'NOT_MEASURED', 'binary_path': str(binary),
              'method': __doc__.strip(), 'log_path': str(saved_log), 'captures': []}
    process = None
    fixture_root = Path.home() / '.cache/files-test'
    fixture_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='tab-strip-motion-', dir=fixture_root) as temporary:
        base = Path(temporary)
        log = base / 'app.log'
        raw = base / 'frame.ppm'
        try:
            home = base / 'home'
            for name in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'A', 'B', 'C']:
                (home / name).mkdir(parents=True)
            payloads = []
            for name in ['A', 'B', 'C']:
                path = home / name / 'one.txt'
                path.write_bytes(f'Owned tab-motion fixture {name}\n'.encode())
                payloads.append((path, digest(path)))
            for name in ['config', 'data', 'files-config']:
                (base / name).mkdir()
            devices = base / 'devices.json'
            devices.write_text('{"blockdevices": []}\n')
            shell = base / 'shell.toml'
            shell.write_text('[font]\nbase-size = 14\n')
            env = {**os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(base / 'config'),
                   'XDG_DATA_HOME': str(base / 'data'), 'FILES_CONFIG_DIR': str(base / 'files-config'),
                   'FILES_TEST_DEVICES_JSON': str(devices), 'OMARCHY_FLTK_USER_SHELL': str(shell),
                   'FILES_TEST_CAPTURE': str(raw), 'FILES_TEST_FIXED_SIZE': '1',
                   'FILES_TEST_ISOLATE_INPUT': '1', 'FILES_TEST_TOOLKIT_POPUP': '1',
                   'FILES_TEST_TAB_MOTION_SHOTS': str(shots), 'FILES_CLOSE_TRACE': '1',
                   'OMARCHY_FLTK_ANIM': 'on', 'TMPDIR': str(base),
                   'MAGICK_TEMPORARY_PATH': str(base)}
            for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                              ('files-html-helper', 'html_helper_sha256')]:
                report[key] = digest(binary if name == 'files' else binary.parent / name)
            with log.open('w') as stream:
                process = subprocess.Popen(
                    [str(binary), str(home / 'A'), '--size', '1000x650', '--scale', '1',
                     '--no-session', '--ui-smoke', '--exit-after', '60'],
                    env=env, stdout=subprocess.DEVNULL, stderr=stream, start_new_session=True)

            def text():
                return log.read_text(errors='replace')

            def wait(check, reason, timeout=20):
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
                offset = len(text())
                pending = raw.with_suffix('.pending')
                pending.write_text(action)
                pending.replace(raw.with_suffix('.popup-command'))
                marker = f'FILES_TOOLKIT_FIXTURE action={action}'
                wait(lambda: marker in text()[offset:].split('\n')[:-1], action)
                assert 'result=Err(' not in text()[offset:], text()[offset:]
                return offset

            def ready(count):
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    offset = send('tab-widget-state')
                    rows = [json.loads(line[len('FILES_TAB_WIDGET_STATE '):])
                            for line in text()[offset:].split('\n')[:-1]
                            if line.startswith('FILES_TAB_WIDGET_STATE ')]
                    if rows:
                        value = rows[-1]
                        if (value['loaded'] and value['settled'] and not value['refreshing']
                                and len(value['tabs']) == count
                                and value['widget']['visible']
                                and all(row['body_rect'] for row in value['widget']['rows'])):
                            report.setdefault('setup_states', []).append(value)
                            return value
                    time.sleep(.02)
                raise RuntimeError('settled tab geometry not ready')

            wait(lambda: 'FILES_UI_SMOKE_OK' in text(), 'smoke ready')
            ready(1)
            send('tab-context-seed')
            ready(3)
            send('tab-widget-motion')

            def motion_report():
                return next((line[len(MARKER):] for line in text().split('\n')[:-1]
                             if line.startswith(MARKER)), None)

            motion = json.loads(wait(motion_report, 'tab-strip motion report'))
            report['motion'] = motion
            report['pixel_checks'] = inspect_motion(motion, shots, env)
            assert all(digest(path) == checksum for path, checksum in payloads), 'fixture changed'
            report['fixture_intact'] = True
            send('close')
            process.wait(timeout=15)
            assert process.returncode == 0 and 'FILES_CLOSE_DRAINED' in text(), 'normal close/drain failed'
            report.update(functional_gate='PASS', normal_exit=True)
        except BaseException as error:
            report['error'] = repr(error)
        finally:
            if process is not None and process.poll() is None:
                stop_tree(process, None)
            report['captures'] = sorted(str(path) for path in shots.iterdir() if path.is_file())
            report['exit_code'] = process.returncode if process is not None else None
            saved_log.write_text(log.read_text(errors='replace') if log.exists() else '')
            target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'report': str(target), 'functional_gate': report['functional_gate'],
                      'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
