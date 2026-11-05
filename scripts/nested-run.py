#!/usr/bin/env python3
"""Run a Files check inside a private nested Hyprland, then stop it.

For machines without Xvfb (M2): the nested compositor starts on a hidden
special workspace of the current session (preview-copy-nested-check.py's
harness), the check gets the NESTED WAYLAND_DISPLAY only, and the user's
windows are never touched.

    scripts/nested-run.py scripts/toolkit-sidebar-ui-check.py [args...]
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('nested', ROOT / 'scripts/preview-copy-nested-check.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    report = {}
    pids = []
    base = Path.home() / '.cache/files-test'
    base.mkdir(parents=True, exist_ok=True)
    code = 1
    with tempfile.TemporaryDirectory(prefix='nested-run-', dir=base) as tmp:
        try:
            socket, signature, pids = harness.start_nested(Path(tmp), report)
            if socket == report['parent_socket']:
                raise SystemExit('refusing the session socket')
            env = {**os.environ, 'WAYLAND_DISPLAY': socket, 'HYPRLAND_INSTANCE_SIGNATURE': signature}
            env.pop('DISPLAY', None)
            code = subprocess.run([sys.executable, *sys.argv[1:]], env=env).returncode
        finally:
            for signal in (15, 9):
                alive = []
                for pid in pids or []:
                    try:
                        os.kill(int(pid), signal)
                        alive.append(pid)
                    except (OSError, ValueError):
                        pass
                deadline = time.monotonic() + 10
                while alive and time.monotonic() < deadline:
                    alive = [p for p in alive if Path(f'/proc/{p}').exists()]
                    time.sleep(.2)
                if not alive:
                    break
            print(json.dumps({'nested': report.get('nested'), 'stopped': not any(
                Path(f'/proc/{p}').exists() for p in pids or [])}))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
