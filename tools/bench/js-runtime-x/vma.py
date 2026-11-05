#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
import os,re,sys,collections
tot=collections.Counter()
for p in os.listdir('/proc'):
    if not p.isdigit(): continue
    try:
        if open(f'/proc/{p}/comm').read().strip()!='Discord': continue
        cur=None;vma={}
        for line in open(f'/proc/{p}/smaps'):
            h=re.match(r'^[0-9a-f]+-[0-9a-f]+ (\S+) \S+ \S+ \S+\s*(.*)$',line)
            if h: cur=(os.path.basename(h.group(2).strip()) or 'anon',h.group(1)); continue
            f=line.split(':')[0]
            if f in('Pss','Pss_Anon','Pss_File','Shared_Clean','Shared_Dirty','Private_Clean','Private_Dirty'):
                tot[(cur,f)]+=int(line.split()[1])
    except OSError: pass
rows=collections.defaultdict(dict)
for (c,f),v in tot.items(): rows[c][f]=v
for c,d in sorted(rows.items(), key=lambda x:-x[1].get('Pss',0))[:12]:
    print(c, {k:v//1024 for k,v in d.items() if v>=1024})
