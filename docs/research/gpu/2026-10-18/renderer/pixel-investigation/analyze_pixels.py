#!/usr/bin/env python3
"""Read frozen NAS images; record changed pixels using the unchanged policy mask."""
from pathlib import Path
from PIL import Image,ImageChops,ImageFilter
import hashlib
import json

BUNDLE=Path('/tmp/catwalk-renderer-bundle-20261018-final')
OUT=Path('/tmp/catwalk-pixel-investigation')
POLICY=BUNDLE/'policy.json'
policy=json.loads(POLICY.read_text())
report=json.loads((BUNDLE/'results/comparisons.json').read_text())
records=[];summaries=[]
for case in report['comparisons']:
    if case['candidate_renderer']!='gpu':continue
    comparison=case['comparison']
    cpu=Image.open(comparison['reference']).convert('RGBA')
    gpu=Image.open(comparison['candidate']).convert('RGBA')
    transparent=case['background']=='transparent'
    def foreground(image):
        threshold=policy['foreground_rgb_background_difference_threshold']
        return Image.frombytes('L',image.size,bytes(255 if (p[3]>0 if transparent else p[3]!=255 or max(abs(p[i]-[245,247,250][i]) for i in range(3))>threshold) else 0 for p in image.getdata()))
    interior=ImageChops.darker(foreground(cpu),foreground(gpu)).filter(ImageFilter.MinFilter(3))
    changed=[]
    for index,(a,b,mask) in enumerate(zip(cpu.getdata(),gpu.getdata(),interior.getdata())):
        if a==b:continue
        x,y=index%cpu.width,index//cpu.width
        neighbors=[]
        for yy in range(max(0,y-1),min(cpu.height,y+2)):
            for xx in range(max(0,x-1),min(cpu.width,x+2)):
                neighbors.append({'xy':[xx,yy],'cpu':list(cpu.getpixel((xx,yy))),'gpu':list(gpu.getpixel((xx,yy)))})
        changed.append({'xy':[x,y],'cpu':list(a),'gpu':list(b),'max_abs_error':max(abs(v-w) for v,w in zip(a,b)),'policy_common_interior':bool(mask),'neighbors':neighbors})
    changed.sort(key=lambda item:(item['policy_common_interior'],item['max_abs_error']),reverse=True)
    internal=[item for item in changed if item['policy_common_interior']]
    violations=[item for item in internal if item['max_abs_error']>policy['interior_max_abs_channel_error']]
    assert len(internal)==comparison['interior_rgba_errors']['changed_pixels']
    summary={'input':case['input'],'samples':case['samples'],'background':case['background'],'failed_criteria':[name for name,passed in comparison['criteria'].items() if not passed],'changed_pixels':len(changed),'common_interior_changed_pixels':len(internal),'common_interior_pixels_exceeding_max3':len(violations),'worst_common_interior':{k:v for k,v in internal[0].items() if k!='neighbors'}}
    summaries.append(summary)
    records.append({'input':case['input'],'samples':case['samples'],'background':case['background'],'reference':comparison['reference'],'candidate':comparison['candidate'],'changed':changed})
    if case['background']=='opaque':
        x,y=internal[0]['xy'];box=(max(0,x-9),max(0,y-9),min(cpu.width,x+10),min(cpu.height,y+10))
        crops=[cpu.crop(box).convert('RGB'),gpu.crop(box).convert('RGB'),ImageChops.difference(cpu,gpu).convert('RGB').crop(box).point(lambda value:min(255,value*8))]
        canvas=Image.new('RGB',(1140,380))
        for column,crop in enumerate(crops):canvas.paste(crop.resize((380,380),Image.Resampling.NEAREST),(380*column,0))
        canvas.save(OUT/f"{case['input']}-s{case['samples']}-crop-cpu-gpu-diff.png")
    print(json.dumps(summary),flush=True)
(OUT/'changed-pixels.json').write_text(json.dumps(records,indent=2)+'\n')
(OUT/'pixel-summary.json').write_text(json.dumps({'policy_sha256':hashlib.sha256(POLICY.read_bytes()).hexdigest(),'policy_unchanged':True,'original_comparisons_sha256':hashlib.sha256((BUNDLE/'results/comparisons.json').read_bytes()).hexdigest(),'pairs':summaries},indent=2)+'\n')
