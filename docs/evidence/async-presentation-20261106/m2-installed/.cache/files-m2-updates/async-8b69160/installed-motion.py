import importlib.util, json, os, pathlib, signal, subprocess, tempfile, time
root = pathlib.Path.home()/'.cache/files-m2-updates/async-8b69160/source'
spec = importlib.util.spec_from_file_location('harness', root/'scripts/preview-copy-nested-check.py')
h = importlib.util.module_from_spec(spec); spec.loader.exec_module(h)
base = pathlib.Path(tempfile.mkdtemp(prefix='regression-compositor-', dir=pathlib.Path.home()/'.cache/files-test'))
report = {'fixture': str(base)}
socket, signature, pids = h.start_nested(base, report)
env = dict(os.environ, WAYLAND_DISPLAY=socket, HYPRLAND_INSTANCE_SIGNATURE=signature)
env.pop('DISPLAY', None)
try:
    for script in ('tab-strip-motion-ui-check',):
        r = subprocess.run(['python3', str(root/'scripts'/f'{script}.py'), '--binary', str(pathlib.Path.home()/'.local/bin/files')], env=env, capture_output=True, text=True)
        print(script, 'exit=', r.returncode, r.stdout, r.stderr, flush=True)
        if r.returncode: break
finally:
    for pid in pids:
        try: os.kill(int(pid), signal.SIGTERM)
        except ProcessLookupError: pass
    print('compositor', json.dumps(report), flush=True)
