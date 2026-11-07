#!/usr/bin/env python3
"""Retain read-only compositor state around an authorized titlebar replacement."""
import argparse, hashlib, json, os, pathlib, re, subprocess
p=argparse.ArgumentParser();p.add_argument('--source-main',required=True,type=pathlib.Path);p.add_argument('--output',required=True,type=pathlib.Path);args=p.parse_args()
def command(*parts):
 return subprocess.check_output(['hyprctl','-i',instance,*parts],text=True)
instances=json.loads(subprocess.check_output(['hyprctl','-j','instances'],text=True));assert len(instances)==1
entry=instances[0];instance=entry['instance'];pid=entry['pid'];proc=pathlib.Path('/proc')/str(pid)
fields=(proc/'stat').read_text().rsplit(')',1)[1].split();assert fields[0]!='Z';start=int(fields[19])
keys=sorted(set(re.findall(r'"(plugin:hyprbars:[^"]+)"',args.source_main.read_text()))-{'plugin:hyprbars:hyprbars-button'})
options={}
for key in keys:
 result=subprocess.run(['hyprctl','-i',instance,'-j','getoption',key],capture_output=True,text=True)
 assert result.returncode==0 or 'no such option' in result.stdout
 options[key]=result.stdout.strip()
clients=json.loads(command('-j','clients'))
windows=[{key:c.get(key) for key in ('address','pid','at','size','floating','mapped','hidden','workspace','fullscreen','fullscreenClient')} for c in clients]
active=json.loads(command('-j','activewindow')).get('address')
plugin=pathlib.Path('/usr/lib/omarchy-hyprland-titlebars/titlebars.so');st=plugin.stat();maps=[]
for line in (proc/'maps').read_text().splitlines():
 if 'titlebars.so' in line:
  f=line.split();maps.append({'inode':int(f[4]),'device':f[3],'path':' '.join(f[5:]),'matches_installed_inode':int(f[4])==st.st_ino})
value={'pid':pid,'start_ticks':start,'instance':instance,'installed_sha256':hashlib.sha256(plugin.read_bytes()).hexdigest(),'installed_inode':st.st_ino,'plugin_maps':maps,'options':options,'active_address':active,'windows':sorted(windows,key=lambda w:w['address']),'config_errors':command('-j','configerrors').strip(),'plugins':command('-j','plugin','list').strip()}
fd=os.open(args.output,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,'w') as f:json.dump(value,f,indent=2);f.write('\n')
print(json.dumps({'output':str(args.output),'pid':pid,'start_ticks':start,'window_count':len(windows),'installed_sha256':value['installed_sha256']}))
