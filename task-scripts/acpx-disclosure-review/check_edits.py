import re,sys,pathlib
repo=pathlib.Path(sys.argv[1]); text=open(sys.argv[2]).read()
parts=re.split(r'(?m)^## ',text)[1:]
spans={}
for p in parts:
    title=p.splitlines()[0]
    m=re.search(r'(?m)^Path:\s*`?([^`\n]+)`?',p)
    olds=re.findall(r'(?ms)^OLD:\s*\n~~~[a-z]*\n(.*?)\n~~~',p)
    news=re.findall(r'(?ms)^NEW:\s*\n~~~[a-z]*\n(.*?)\n~~~',p)
    if not m: print('NO PATH',title); continue
    path=m.group(1).strip(); src=(repo/path).read_text()
    for o in olds:
        n=src.count(o); i=src.find(o)
        line=src[:i].count('\n')+1 if i>=0 else -1
        spans.setdefault(path,[]).append((i,i+len(o),title))
        print(f'{n} match  L{line:<5} {path.split("/")[-1]:14} {title[:70]}')
    if len(olds)!=len(news): print('  OLD/NEW count mismatch',len(olds),len(news))
for path,s in spans.items():
    s=sorted(x for x in s if x[0]>=0)
    for a,b in zip(s,s[1:]):
        if b[0]<a[1]: print('OVERLAP',path,a[2][:40],'|',b[2][:40])
