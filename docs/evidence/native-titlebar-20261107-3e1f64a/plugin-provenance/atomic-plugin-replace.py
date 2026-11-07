#!/usr/bin/env python3
"""Replace one verified plugin inode, preserving its verified prior artifact."""
import argparse, hashlib, json, os, pathlib, stat
p=argparse.ArgumentParser();p.add_argument('--candidate',required=True,type=pathlib.Path);p.add_argument('--candidate-sha256',required=True);p.add_argument('--expected-installed-sha256',required=True);p.add_argument('--backup',required=True,type=pathlib.Path);args=p.parse_args()
assert os.geteuid()==0, 'Run the reviewed atomic replacement with sudo -n'
def open_directory(path):
 path=pathlib.Path(path);assert path.is_absolute() and '..' not in path.parts
 fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC)
 try:
  for part in path.parts[1:]:
   nxt=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd);os.close(fd);fd=nxt
  return fd
 except BaseException:os.close(fd);raise
def read(directory,name):
 fd=os.open(name,os.O_RDONLY|os.O_NONBLOCK|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=directory)
 with os.fdopen(fd,'rb') as f:
  st=os.fstat(f.fileno());assert stat.S_ISREG(st.st_mode) and st.st_nlink==1 and st.st_size<16*1024*1024
  data=f.read(16*1024*1024+1);assert len(data)==st.st_size
 return st,data
sha=lambda b:hashlib.sha256(b).hexdigest()
lib=open_directory('/usr/lib/omarchy-hyprland-titlebars');source_dir=open_directory(args.candidate.parent);backup_dir=open_directory(args.backup.parent)
try:
 old_st,old=read(lib,'titlebars.so');new_st,new=read(source_dir,args.candidate.name)
 assert sha(old)==args.expected_installed_sha256 and sha(new)==args.candidate_sha256
 assert old_st.st_uid==0 and new[:4]==b'\x7fELF' and old[18:20]==new[18:20]
 fd=os.open(args.backup.name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o644,dir_fd=backup_dir)
 with os.fdopen(fd,'wb') as f:f.write(old);f.flush();os.fchmod(f.fileno(),0o644);os.fsync(f.fileno())
 assert sha(read(backup_dir,args.backup.name)[1])==args.expected_installed_sha256
 temporary='.titlebars-stage-'+str(os.getpid())+'-'+os.urandom(8).hex()+'.so'
 fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o700,dir_fd=lib)
 with os.fdopen(fd,'wb') as f:
  f.write(new);f.flush();os.fchown(f.fileno(),old_st.st_uid,old_st.st_gid);os.fchmod(f.fileno(),stat.S_IMODE(old_st.st_mode));os.fsync(f.fileno())
 assert sha(read(lib,temporary)[1])==args.candidate_sha256
 assert sha(read(lib,'titlebars.so')[1])==args.expected_installed_sha256
 os.replace(temporary,'titlebars.so',src_dir_fd=lib,dst_dir_fd=lib);os.fsync(lib)
 installed_st,installed=read(lib,'titlebars.so');assert sha(installed)==args.candidate_sha256 and installed_st.st_ino!=old_st.st_ino
 print(json.dumps({'backup':str(args.backup),'backup_sha256':sha(old),'installed_sha256':sha(installed),'old_inode':old_st.st_ino,'new_inode':installed_st.st_ino,'atomic_replace':True}))
finally:os.close(lib);os.close(source_dir);os.close(backup_dir)
