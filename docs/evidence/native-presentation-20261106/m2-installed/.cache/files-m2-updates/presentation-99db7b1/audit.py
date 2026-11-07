import hashlib,json,pathlib,sys
h=pathlib.Path.home(); base=h/'.cache/files-m2-updates/presentation-99db7b1'; receipt=base/'source/docs/evidence/0.0.4/installed-checkpoint-1794010906514383974.json'; r=json.loads(receipt.read_text()); backup=pathlib.Path(r['backup']); expected={'config.toml':'36383052a1f04ccb6fe5f122e2e67f03fb87aeaf9fe93e9581ff4f31e6d8fe9e','session.bin':'bf44903de9bbd4add969a9409b03b6a12edc865d7ff293fc16146ab80c0ea4f9','layouts/32d393d1817c5e72.toml':'1cbf77da02414f7c97e6a895b9a65fefaae10c41c91bdf869111a56df40d0a21','layouts/35201e16dc621286.toml':'c17250785358f5cb71cc8624ecfef3fc85d20ba3f386a987d0d8bc9224bb1772','layouts/0d922b96b215a0a5.toml':'e0f3277c2f15261378b7aa49496828016358163df7236397f825f1b00a0932e7'}
def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
audit={'files':[],'config':[]}
for f in r['files']:
 a=digest(backup/f['name']); b=digest(h/'.local/bin'/f['name']); audit['files'].append({'name':f['name'],'backup_sha256':a,'installed_sha256':b,'backup_matches':a==f['old_sha256'],'installed_matches':b==f['release_sha256']})
for name,sha in expected.items():
 actual=digest(h/'.config/files'/name); audit['config'].append({'name':name,'sha256':actual,'unchanged':actual==sha})
audit['pass']=all(x['backup_matches'] and x['installed_matches'] for x in audit['files']) and all(x['unchanged'] for x in audit['config']); out=base/('audit-'+sys.argv[1]+'.json'); out.write_text(json.dumps(audit,indent=2)+'\n'); print(out, 'PASS' if audit['pass'] else 'FAIL'); sys.exit(0 if audit['pass'] else 1)
