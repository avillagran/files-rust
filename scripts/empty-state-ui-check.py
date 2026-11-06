#!/usr/bin/env python3
"""Capture pane-owned empty/filter states with private production-handler fixtures.

Run under Xvfb or scripts/nested-run.py. State assertions and real framebuffers
cover owner isolation; Painter recorder tests separately verify message bounds
and counts. This is not native OS input or resource/startup acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

from memcheck import stop_tree

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'docs/evidence/0.0.4'
STATE = 'FILES_HIDDEN_STATE '


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ready(state):
    watch = state['listing_watch']
    return (watch['settled'] and not watch['pending']
            and all(p['loaded'] for p in state['dual']['panels']))


class Case:
    def __init__(self, base, name, binary, source, other, animations, report, identity):
        self.base = base / name
        self.base.mkdir()
        home = self.base / 'home'
        for folder in ['Downloads', 'Documents', 'Pictures', 'Music', 'Videos']:
            (home / folder).mkdir(parents=True)
        for folder in ['config', 'data', 'files-config']:
            (self.base / folder).mkdir()
        devices = self.base / 'devices.json'
        devices.write_text('{"blockdevices": []}\n')
        shell = self.base / 'shell.toml'
        shell.write_text('[font]\nbase-size = 14\n')
        self.env = {
            **os.environ, 'HOME': str(home), 'XDG_CONFIG_HOME': str(self.base / 'config'),
            'XDG_DATA_HOME': str(self.base / 'data'), 'FILES_CONFIG_DIR': str(self.base / 'files-config'),
            'FILES_TEST_DEVICES_JSON': str(devices), 'OMARCHY_FLTK_USER_SHELL': str(shell),
            'TMPDIR': str(self.base), 'MAGICK_TEMPORARY_PATH': str(self.base),
            'FILES_TEST_HIDDEN_CONTEXT': '1', 'FILES_TEST_ISOLATE_INPUT': '1',
            'FILES_TEST_FIXED_SIZE': '1', 'FILES_CLOSE_TRACE': '1',
            'FILES_TEST_DUAL_B': str(other), 'OMARCHY_FLTK_ANIM': animations,
        }
        self.raw = self.base / 'frame.ppm'
        self.env['FILES_TEST_CAPTURE'] = str(self.raw)
        self.log = OUT / f'empty-state-{name}-{identity}.txt'
        self.name, self.report, self.identity, self.seq = name, report, identity, 0
        report['logs'].append(str(self.log))
        with self.log.open('w') as stream:
            self.process = subprocess.Popen(
                [str(binary), str(source), '--no-session', '--ui-smoke', '--ui-scene',
                 'hidden-context', '--size', '1000x650', '--scale', '1', '--exit-after', '90'],
                env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                stderr=stream, start_new_session=True)

    def text(self):
        return self.log.read_text(errors='replace')

    def wait(self, check, reason, allow_exit=False):
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            value = check()
            if value:
                return value
            if self.process.poll() is not None and not allow_exit:
                raise RuntimeError(f'{self.name}: {reason}: exit {self.process.returncode}')
            time.sleep(.02)
        raise RuntimeError(f'{self.name}: {reason}: timed out')

    def state(self, after=0, predicate=ready):
        def latest():
            value = None
            for line in self.text()[after:].split('\n')[:-1]:
                if line.startswith(STATE):
                    row = json.loads(line[len(STATE):])
                    if row['seq'] == self.seq:
                        value = row
            return value if value is not None and predicate(value) else None
        return self.wait(latest, f'painted ready state seq={self.seq}')

    def command(self, action):
        self.state()  # Do not submit navigation while initial/path watch refresh is pending.
        before = len(self.text())
        pending = self.raw.with_suffix('.pending')
        pending.write_text(action)
        pending.replace(self.raw.with_suffix('.command'))
        pattern = re.compile(r'FILES_SPACE_COMMAND seq=(\d+) action=' + re.escape(action))

        def acknowledged():
            for line in self.text()[before:].split('\n')[:-1]:
                if line.startswith('FILES_SPACE_COMMAND_ERROR '):
                    raise RuntimeError(line)
                match = pattern.fullmatch(line)
                if match:
                    return int(match[1])
            return None
        self.seq = self.wait(acknowledged, action, action == 'callback')
        return before

    def act(self, action):
        state = self.state(self.command(action))
        self.report['states'].append({'case': self.name, 'action': action, 'state': state})
        return state

    def capture(self, label):
        state = self.state()
        self.raw.unlink(missing_ok=True)
        self.raw.with_suffix('.request').touch()
        width, height = state['width'], state['height']

        def complete():
            if not self.raw.exists():
                return False
            parts = self.raw.read_bytes().split(b'\n', 3)
            return (len(parts) == 4 and parts[1] == f'{width} {height}'.encode()
                    and len(parts[3]) == width * height * 3)
        self.wait(complete, 'complete framebuffer')
        target = OUT / f'empty-state-{label}-{self.identity}.png'
        subprocess.run(['magick', str(self.raw), str(target)], env=self.env, check=True, timeout=15)
        self.report['captures'].append({'path': str(target), 'dimensions': [width, height]})

    def close(self):
        self.command('callback')
        self.wait(lambda: self.process.poll() is not None, 'normal close', True)
        assert self.process.returncode == 0 and 'FILES_CLOSE_DRAINED' in self.text()
        self.report['normal_exits'].append(self.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/files')
    parser.add_argument('--animations', choices=['on', 'off'], default='on')
    args = parser.parse_args()
    binary, identity = args.binary.resolve(), str(time.time_ns())
    OUT.mkdir(parents=True, exist_ok=True)
    report = {'functional_gate': 'FAIL', 'resource_gate': 'NOT_MEASURED', 'startup_gate': 'NOT_MEASURED',
              'method': __doc__.strip(), 'binary_path': str(binary), 'animations': args.animations,
              'states': [], 'captures': [], 'logs': [], 'checks': [], 'normal_exits': []}
    for name, key in [('files', 'binary_sha256'), ('files-helper', 'helper_sha256'),
                      ('files-html-helper', 'html_helper_sha256')]:
        report[key] = digest(binary if name == 'files' else binary.parent / name)
    base_root = Path.home() / '.cache/files-test'
    base_root.mkdir(parents=True, exist_ok=True)
    case = None
    temporary = tempfile.TemporaryDirectory(prefix='empty-state-', dir=base_root)
    try:
        base = Path(temporary.name)
        populated, empty = base / 'A', base / 'B'
        populated.mkdir(); empty.mkdir()
        payload = populated / 'one.txt'
        payload.write_bytes(b'Owned empty-state sibling fixture\n')
        original = digest(payload)
        raw = b'one.txt'.hex()

        case = Case(base, 'populated-left', binary, populated, empty, args.animations, report, identity)
        for view in ['list', 'grid']:
            s = case.act(view)
            assert s['order'] == [raw]
            generation = s['generation']
            s = case.act('filter-query:no-such-owned-name')
            assert not s['order'] and s['query'] == 'no-such-owned-name'
            assert s['generation'] == generation
            case.capture(f'{view}-filter-no-results')
            s = case.act('filter-clear-hit')
            assert s['order'] == [raw] and not s['query'] and s['generation'] == generation
            case.act('filter-key:escape')
        case.act('list'); case.act('select-name:one.txt'); case.act('dual-on')
        case.act('dual-switch'); s = case.act('dual-b-location')
        assert s['dual']['enabled'] and s['dual']['right_active'] and not s['order']
        other = next(p for p in s['dual']['panels'] if not p['active'])
        assert other['order'] == [raw] and other['selection'] == [raw]
        s = case.act('fixture-short')
        assert [s['width'], s['height']] == [800, 440]
        case.capture('right-empty-narrow')
        s = case.act('fixture-minimum')
        assert [s['width'], s['height']] == [440, 440]
        case.capture('right-empty-minimum')
        s = case.act('dual-switch')
        assert s['order'] == [raw] and s['selection'] == [raw]
        case.close(); case = None

        case = Case(base, 'empty-left', binary, empty, populated, args.animations, report, identity)
        for view in ['list', 'grid', 'columns']:
            s = case.act(view)
            assert not s['order'] and not s['selection'] and not s['error']
            case.capture(f'{view}-empty-folder')
        case.act('list'); case.act('dual-on'); case.act('dual-switch'); case.act('dual-b-location')
        case.act('select-name:one.txt'); s = case.act('dual-switch')
        assert s['dual']['enabled'] and not s['dual']['right_active'] and not s['order']
        other = next(p for p in s['dual']['panels'] if not p['active'])
        assert other['order'] == [raw] and other['selection'] == [raw]
        s = case.act('fixture-short')
        assert [s['width'], s['height']] == [800, 440]
        case.capture('left-empty-narrow')
        s = case.act('fixture-minimum')
        assert [s['width'], s['height']] == [440, 440]
        case.capture('left-empty-minimum')
        s = case.act('dual-switch')
        assert s['order'] == [raw] and s['selection'] == [raw]
        case.close(); case = None
        assert digest(payload) == original and not list(empty.iterdir())
        report.update(functional_gate='PASS', fixture_intact=True)
        report['checks'] = ['List/Grid no-match filters clear to the same raw listing without reload',
                            'Empty List/Grid/Columns framebuffers with no selection or listing error',
                            'Either active empty pane preserves the populated sibling raw listing and selection at 800x440 and 440x440',
                            'Both private app cases close normally with callback drain']
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        if case is not None and case.process.poll() is None:
            stop_tree(case.process, None)
        temporary.cleanup()
        target = OUT / f'empty-state-ui-{identity}.json'
        target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'report': str(target), 'functional_gate': report['functional_gate'],
                          'error': report.get('error')}))
    return 0 if report['functional_gate'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
