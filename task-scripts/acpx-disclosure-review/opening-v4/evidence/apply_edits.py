import re,sys
root="/Volumes/Mini-SSD/offload/repos/grok-build/"
src=open(root+"task-scripts/acpx-disclosure-review/opening-v4/grok-build/task-scripts/acpx-disclosure-returns/acpx-opening-ledger-delta.edits.md").read()
src=src.split("## Coordinator follow-ups")[0]
parts=re.split(r'^## Edit (\d+):.*\n',src,flags=re.M)
edits=[]
for i in range(1,len(parts),2):
    n=int(parts[i]); body=parts[i+1]
    m=re.match(r'Path: (\S+)\nOLD:\n~~~rust\n(.*?)\n~~~\nNEW:\n~~~rust\n(.*?)\n~~~\s*$',body,flags=re.S)
    assert m,n
    edits.append((n,m.group(1),m.group(2),m.group(3)))
assert len(edits)==20
texts={}
for n,p,o,w in edits:
    t=texts.setdefault(p,open(root+p).read())
    c=t.count(o)
    print(n,p,c)
    assert c==1
    texts[p]=t.replace(o,w)
if "--write" in sys.argv:
    for p,t in texts.items(): open(root+p,"w").write(t)
