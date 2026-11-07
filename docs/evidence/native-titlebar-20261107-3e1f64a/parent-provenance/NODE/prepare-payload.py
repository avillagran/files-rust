import pathlib,os,subprocess,json,hashlib,tarfile,posixpath,stat,sys
r=pathlib.Path(sys.argv[1]);records=json.loads((r/'manifest.json').read_text());db={}
for path in pathlib.Path('/var/lib/pacman/sync').glob('*.db'):
 with tarfile.open(path,'r:*') as tf:
  for m in tf.getmembers():
   if not m.name.endswith('/desc'):continue
   fields=tf.extractfile(m).read().decode().split('\n\n');values={}
   for item in fields:
    lines=item.strip().splitlines()
    if lines and lines[0].startswith('%'):values[lines[0].strip('%')]='\n'.join(lines[1:])
   if values.get('NAME') in {i['name'] for i in records}:db[values['NAME']]=values
files=[];root=r/'payload';links=[]
for row in records:
 p=r/'packages'/row['archive'];expected=db[row['name']];assert expected['VERSION']==row['version'] and expected['FILENAME']==row['archive'];assert row['sha256']==expected['SHA256SUM']==hashlib.sha256(p.read_bytes()).hexdigest();row['repository_sha256_verified']=True
 with tarfile.open(p,'r:*') as tf:
  for m in tf.getmembers():
   parts=pathlib.PurePosixPath(m.name).parts
   if not parts or parts[0].startswith('.'):continue
   assert not pathlib.PurePosixPath(m.name).is_absolute() and '..' not in parts
   dest=root/pathlib.Path(*parts)
   assert dest.parent.resolve().is_relative_to(root.resolve())
   dest.parent.mkdir(parents=True,exist_ok=True)
   if m.isdir():dest.mkdir(exist_ok=True);continue
   if m.issym():
    linkname=m.linkname
    if linkname.startswith('/'):
     absolute=pathlib.PurePosixPath(linkname);assert absolute.parts[1]=='usr' and '..' not in absolute.parts
     linkname=posixpath.relpath(linkname.lstrip('/'),posixpath.dirname(m.name))
    target=posixpath.normpath(posixpath.join(posixpath.dirname(m.name),linkname));assert target!='..' and not target.startswith('../')
    if dest.is_symlink():assert os.readlink(dest)==linkname
    else:os.symlink(linkname,dest)
    links.append(str(dest.relative_to(root)));continue
   assert m.isfile() or m.islnk(),(row['name'],m.name,m.type)
   size=m.size
   if m.islnk():
    target=pathlib.PurePosixPath(m.linkname);assert not target.is_absolute() and '..' not in target.parts
    link=tf.getmember(m.linkname);assert link.isfile();size=link.size
   data=tf.extractfile(m).read();assert len(data)==size and size<64*1024*1024
   if dest.exists():assert dest.is_file() and not dest.is_symlink() and dest.read_bytes()==data
   else:
    fd=os.open(dest,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o755 if m.mode&0o111 else 0o644)
    with os.fdopen(fd,'wb') as out:out.write(data)
   files.append({'path':str(dest.relative_to(root)),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
for name in links:assert (root/name).resolve().is_relative_to(root.resolve())
(r/'verified-packages.json').write_text(json.dumps(records,indent=2)+'\n');(r/'files.json').write_text(json.dumps(files,indent=2)+'\n')
env=dict(os.environ,LD_LIBRARY_PATH=str(root/'usr/lib'))
result=subprocess.run(['ldd',str(root/'usr/bin/kwin_wayland')],env=env,capture_output=True,text=True)
(r/'ldd.txt').write_text(result.stdout+result.stderr)
print(json.dumps({'root':str(r),'packages':len(records),'files':len(files),'symlinks':len(links),'missing':[l.strip() for l in result.stdout.splitlines() if 'not found' in l]}),flush=True)
