import os,json,hashlib,subprocess,time,importlib.util,stat
from pathlib import Path
root=Path.home()/'.cache/files-m2-updates/sheets-7ec4ed5';source=root/'source';release=source/'target/release'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
manifest=json.loads((root/'source-manifest-7ec4ed5.json').read_text())
for item in manifest['files']:
 p=source/item['path'];assert not p.is_symlink() and sha(p)==item['sha256'] and stat.S_IMODE(p.stat().st_mode)==int(item['git_mode'],8)&0o777
record=json.loads((root/'build-result.json').read_text());(root/'build-result.json').rename(root/'build-result-cached-negative-checks.json')
assert subprocess.check_output(['pkg-config','--variable=libdir','pango'],text=True).strip()=='/usr/lib'
env=dict(os.environ);env.update(TMPDIR=str(root/'private-build/tmp'),FILES_CONFIG_DIR=str(root/'private-build/config'),XDG_DATA_HOME=str(root/'private-build/data'),CARGO_TARGET_DIR=str(source/'target'),PYTHONDONTWRITEBYTECODE='1',CMAKE_LIBRARY_PATH='/usr/lib')
cache=source/'target/release/build/fltk-sys-c3a49caeb1e019b6/out/build/CMakeCache.txt'
cache.rename(cache.with_name('CMakeCache.failed-library-checks.txt'))
start=time.monotonic()
with (root/'build-fresh-library-checks.log').open('x') as out:result=subprocess.run(['cargo','build','--release','--locked','--offline'],cwd=source,env=env,stdout=out,stderr=subprocess.STDOUT)
record.update(build_returncode=result.returncode,elapsed_seconds=time.monotonic()-start,library_discovery='CMAKE_LIBRARY_PATH=/usr/lib, matching native pkg-config pango libdir',prior_failure='build-result-cached-negative-checks.json')
for item in manifest['files']:assert sha(source/item['path'])==item['sha256']
if result.returncode==0:
 for name in ['files','files-helper','files-html-helper','files-html-helper.LICENSES','files-html-helper.build.json']:record[name+'_sha256']=sha(release/name)
 for name in ['files','files-helper','files-html-helper']:assert 'ARM aarch64' in subprocess.check_output(['file',str(release/name)],text=True)
 stamp=json.loads((release/'files-html-helper.build.json').read_text());spec=importlib.util.spec_from_file_location('recipe',source/'scripts/html-engine-probe.py');recipe=importlib.util.module_from_spec(spec);spec.loader.exec_module(recipe)
 assert stamp['adapter_sha256']==recipe.adapter_fingerprint() and stamp['binary_sha256']==sha(release/'files-html-helper')
 record.update(html_stamp=stamp,reused_native_html_recipe_verified=True,stage='built-not-installed-or-native-accepted')
else:record['stage']='build-failed-evidence-preserved'
(root/'build-result.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record));raise SystemExit(result.returncode)
