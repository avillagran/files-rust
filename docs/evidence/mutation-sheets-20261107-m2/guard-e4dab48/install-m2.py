"""Install an ARM candidate after private native acceptance, retaining backups."""
import hashlib,json,os,stat,subprocess,sys,time
from pathlib import Path
root=Path.home()/'.cache/files-m2-updates/sheets-e4dab48'
release=root/'source/target/release'
report_path=Path(sys.argv[1]);report=json.loads(report_path.read_text())
assert report['functional_gate']=='PASS'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert sha(release/'files')==report['binary_sha256']
assert sha(release/'files-helper')==report['helper_sha256']
assert sha(release/'files-html-helper')==report['html_helper_sha256']
build=json.loads((root/'build-result.json').read_text());assert build['build_returncode']==0 and build['reused_native_html_recipe_verified']
names=['files','files-helper','files-html-helper','files-html-helper.LICENSES','files-html-helper.build.json']
stamp=json.loads((release/names[4]).read_text());assert stamp['binary_sha256']==sha(release/names[2])
assert stamp['adapter_sha256']==build['html_stamp']['adapter_sha256']
for name in names[:3]:
 data=(release/name).read_bytes()[:20];assert data[:4]==b'\x7fELF' and int.from_bytes(data[18:20],'little')==183
bin=Path.home()/'.local/bin';assert bin.is_dir() and not bin.is_symlink()
transaction=root/('installation-'+str(time.time_ns()));transaction.mkdir(mode=0o700)
backup=transaction/'backup';backup.mkdir(mode=0o700)
records=[];staged={};replaced=[]
def copy_exclusive(source,destination,mode):
 fd=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode)
 with os.fdopen(fd,'wb') as out,source.open('rb') as inp:
  while chunk:=inp.read(1024*1024):out.write(chunk)
  out.flush();os.fchmod(out.fileno(),mode);os.fsync(out.fileno())
try:
 for name in names:
  dest=bin/name;info=dest.lstat();assert stat.S_ISREG(info.st_mode) and info.st_uid==os.getuid()
  old=sha(dest);copy_exclusive(dest,backup/name,stat.S_IMODE(info.st_mode));assert sha(backup/name)==old
  new=sha(release/name);assert new==build[name+'_sha256']
  records.append(dict(name=name,old_sha256=old,backup_sha256=old,release_sha256=new))
 (transaction/'before.json').write_text(json.dumps(records,indent=2)+'\n')
 for record in records:
  name=record['name'];temporary=bin/('.files-stage-'+transaction.name+'-'+name)
  copy_exclusive(release/name,temporary,0o755 if name in names[:3] else 0o644)
  assert sha(temporary)==record['release_sha256'];staged[name]=temporary
 for record in records:
  name=record['name'];assert sha(bin/name)==record['old_sha256'],'Concurrent installation'
  os.replace(staged[name],bin/name);replaced.append(name)
 for record in records:
  record['installed_sha256']=sha(bin/record['name']);assert record['installed_sha256']==record['release_sha256']
 fd=os.open(bin,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
except BaseException:
 for name in reversed(replaced):
  rollback=bin/('.files-rollback-'+transaction.name+'-'+name)
  copy_exclusive(backup/name,rollback,stat.S_IMODE((backup/name).stat().st_mode));os.replace(rollback,bin/name)
 (transaction/'failed.json').write_text(json.dumps(dict(replaced=replaced,records=records),indent=2)+'\n')
 raise
result=dict(source_checkpoint=build['source_checkpoint'],functional_gate='PASS',validation_report=str(report_path),backup=str(backup),files=records,scope='M2 private functional acceptance; all five backed up and individually atomically replaced; existing processes untouched',resource_gate='NOT_MEASURED',startup_gate='NOT_MEASURED',full_regression_gate='NOT_RUN')
(transaction/'installed.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
