#!/usr/bin/env python3
"""The files of crates/bencode-ui that the app reaches, and the rest.

Run from crates/bencode-ui: it reads what the app imports from ../../src,
prints what it keeps and drops, and with `--apply` deletes the dropped
files and their `mod` / `use` lines in the parents.

A file is kept when it defines an item the app imports, when a kept file
names it through `use crate::/super::/self::` or a `crate::` path, when it
is a child module a kept file uses by path, or when it adds an `impl` to a
kept type. It is a text scan, not the compiler: check with `cargo check`
and restore what it asks for.
"""
import re, os, glob, collections, sys, subprocess
files=glob.glob('src/**/*.rs',recursive=True); src={f:open(f).read() for f in files}
def modfiles(path):
    base='src/'+'/'.join(path)
    return [c for c in (base+'.rs',base+'/mod.rs') if c in src]
def defs_in(paths,name):
    base='src/'+'/'.join(paths)
    cands=[f for f in src if f==base+'.rs' or f.startswith(base+'/')]
    res={f for f in cands if re.search(r'\b(?:struct|enum|fn|trait|type|const|static|macro_rules!)\s*'+re.escape(name)+r'\b',src[f])}
    return res or set(modfiles(paths+[name])) or set(modfiles(paths))
def flatten(prefix,s):
    depth=0;cur='';parts=[]
    for ch in s:
        if ch=='{':depth+=1
        if ch=='}':depth-=1
        if ch==',' and depth==0: parts.append(cur);cur=''
        else: cur+=ch
    parts.append(cur); out=[]
    for p in parts:
        p=p.strip()
        if not p: continue
        if '{' in p:
            h,r=p.split('{',1); out+=flatten(prefix+[x for x in h.strip().rstrip(':').split('::') if x],r.rsplit('}',1)[0])
        else: out.append(prefix+[x for x in p.split(' as ')[0].strip().split('::') if x])
    return out
def modpath(f):
    p=f[4:-3].split('/')
    if p[-1] in ('mod','lib'): p=p[:-1]
    return p
def resolve(f,path):
    mp=modpath(f)
    while path and path[0] in ('crate','super','self'):
        h=path[0]; path=path[1:]
        if h=='crate': mp=[]
        elif h=='super': mp=mp[:-1]
    path=mp+path
    path=[p for p in path if p]
    if not path: return set()
    name=path[-1]
    if name in ('*','self'): return set(modfiles(path[:-1]))
    if len(path)==1: return set(modfiles(path)) or defs_in([],name)
    return defs_in(path[:-1],name)
def deps(f):
    s=src[f]; out=set()
    for m in re.finditer(r'\buse\s+((?:crate|super|self)::.*?);',s,re.S):
        for p in flatten([],m.group(1)): out|=resolve(f,p)
    for m in re.finditer(r'\b((?:crate|super)(?:::[A-Za-z0-9_]+)+)',s):
        out|=resolve(f,m.group(1).split('::'))
    d=os.path.dirname(f)
    body=re.sub(r'\bpub(?:\([^)]*\))?\s+use\s+[^;]*;','',s,flags=re.S)
    for m in re.finditer(r'^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_0-9]+)\s*;',s,re.M):
        n=m.group(1)
        if re.search(r'(?<![A-Za-z0-9_:])'+n+r'::',body):
            base=os.path.join(d if f.endswith(('mod.rs','lib.rs')) else f[:-3],n)
            for c in (base+'.rs',base+'/mod.rs'):
                if c in src: out.add(c)
    out.discard('src/lib.rs'); return out

def app_uses():
    files = subprocess.check_output(
        ["grep", "-rlE", "bencode_ui", "../../src", "--include=*.rs"]).decode().split()
    def flatten(prefix, s):
        depth = 0; cur = ""; parts = []
        for ch in s:
            if ch == "{": depth += 1
            if ch == "}": depth -= 1
            if ch == "," and depth == 0: parts.append(cur); cur = ""
            else: cur += ch
        parts.append(cur); out = []
        for p in parts:
            p = p.strip()
            if not p: continue
            if "{" in p:
                head, rest = p.split("{", 1)
                out += flatten(prefix + head.strip().rstrip(":").strip() + "::" if head.strip() else prefix,
                               rest.rsplit("}", 1)[0])
            else:
                out.append(prefix + p.split(" as ")[0].strip())
        return out
    items = set()
    for f in files:
        s = open(f).read()
        for m in re.finditer(r"use\s+bencode_ui::(.*?);", s, re.S): items |= set(flatten("", m.group(1)))
        for m in re.finditer(r"(?<!use )bencode_ui::([A-Za-z_][A-Za-z0-9_:]*)", s): items.add(m.group(1))
    return [i.split("::") for i in sorted(items)]
used=app_uses()
start={'src/lib.rs','src/assets.rs'}
for u in used:
    u=[x for x in u if x]
    if u[-1] in ('new','text'): u=u[:-1]
    if not u or u==['Assets'] or u==['init']: continue
    if len(u)==1: start|=set(modfiles(u)); continue
    start|=defs_in(u[:-1],u[-1])
seen=set();stack=list(start)
while stack:
    f=stack.pop()
    if f in seen: continue
    seen.add(f); stack+=[d for d in deps(f) if d not in seen]
# Files that add impls to a kept type, and what they need, until nothing new.
def types_in(f):
    return set(re.findall(r'\b(?:struct|enum|trait)\s+([A-Z][A-Za-z0-9_]*)',src[f]))
impls={f:set(re.findall(r'\bimpl(?:\s*<[^>{]*>)?\s+(?:[A-Za-z0-9_:<>, ]+\s+for\s+)?(?:[a-z_]+::)*([A-Z][A-Za-z0-9_]*)',src[f])) for f in src}
while True:
    kept_types=set().union(*(types_in(f) for f in seen))
    new={f for f in src if f not in seen and impls[f] & kept_types}
    if not new: break
    stack=list(new)
    while stack:
        f=stack.pop()
        if f in seen: continue
        seen.add(f); stack+=[d for d in deps(f) if d not in seen]
keep=set(seen)
for f in list(seen):
    p=os.path.dirname(f)
    while p!='src':
        for c in (p+'.rs',p+'/mod.rs'):
            if c in src: keep.add(c)
        p=os.path.dirname(p)
drop=sorted(f for f in src if f not in keep)
print('keep',len(keep),sum(src[f].count('\n') for f in keep),'drop',len(drop),sum(src[f].count('\n') for f in drop))
if '--apply' not in sys.argv: sys.exit()
def parent_of(f):
    d=os.path.dirname(f); name=os.path.basename(f)[:-3]
    if name=='mod': name=os.path.basename(d); d=os.path.dirname(d)
    for c in (d+'.rs', d+'/mod.rs', 'src/lib.rs' if d=='src' else None):
        if c and os.path.exists(c): return c,name
    return None,name
removed_mods=collections.defaultdict(set)
for f in drop:
    p,name=parent_of(f)
    if p: removed_mods[p].add(name)
for f in drop:
    os.remove(f)
# remove empty dirs
for d,_,_ in sorted(os.walk('src'),reverse=True):
    if not os.listdir(d): os.rmdir(d)
for p,names in removed_mods.items():
    if not os.path.exists(p): continue
    s=open(p).read()
    # a dir module whose mod.rs survives: only remove if the module file itself is gone
    base=os.path.dirname(p) if p.endswith(('mod.rs','lib.rs')) else p[:-3]
    names={n for n in names if not (os.path.exists(os.path.join(base,n+'.rs')) or os.path.exists(os.path.join(base,n,'mod.rs')))}
    for n in names:
        # attributes + mod decl
        s=re.sub(r'((?:^[ \t]*#\[[^\]]*\]\s*\n)*)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+'+n+r'\s*;\s*\n','',s,flags=re.M)
        # use statements starting with n:: or self::n::
        s=re.sub(r'((?:^[ \t]*#\[[^\]]*\]\s*\n)*)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+(?:self::)?'+n+r'::[^;]*;\s*\n','',s,flags=re.M|re.S)
    open(p,'w').write(s)
print('applied')
