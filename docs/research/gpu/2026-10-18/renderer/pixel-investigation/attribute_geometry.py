#!/usr/bin/env python3
"""Read-only, independent CPU projection/ray attribution for frozen NAS PNG failures."""
from pathlib import Path
import json
import math
import struct

ROOT = Path('/tmp/catwalk-wgpu-worktree')
BUNDLE = Path('/tmp/catwalk-renderer-bundle-20261018-final')
OUT = Path('/tmp/catwalk-pixel-investigation')

def sub(a, b): return tuple(x-y for x,y in zip(a,b))
def div(a, s): return tuple(x/s for x in a)
def dot(a, b): return a[0]*b[0]+a[1]*b[1]+a[2]*b[2]
def cross(a,b): return (a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
def length(a): return math.hypot(math.hypot(a[0],a[1]),a[2])
def unit(a): return div(a,length(a))
def f32(v): return struct.unpack('f',struct.pack('f',v))[0]
def edge(a,b,p): return (p[0]-a[0])*(b[1]-a[1])-(p[1]-a[1])*(b[0]-a[0])
TOWARD=unit((1.,-1.,1.)); RIGHT=unit(cross((0.,0.,1.),TOWARD)); UP=unit(cross(TOWARD,RIGHT)); LIGHT=unit((-.35,-.6,1.))

def normal(a,b,c): return unit(cross(unit(sub(b,a)),unit(sub(c,a))))
def shade(n):
    facing=dot(n,TOWARD)
    if facing<0 or (facing==0 and dot(n,LIGHT)<0): n=tuple(-x for x in n)
    intensity=.35+.65*max(dot(n,LIGHT),0)
    return [math.floor(c*intensity+.5) for c in (100,160,220)]

def gear():
    positions=[]; faces=[]; source_faces=[]
    for line in (BUNDLE/'inputs/gear.obj').read_text().splitlines():
        words=line.split()
        if not words: continue
        if words[0]=='v':positions.append(tuple(map(float,words[1:4])))
        if words[0]=='f':
            corners=[int(word.split('/')[0])-1 for word in words[1:]]
            for i in range(1,len(corners)-1):
                faces.append((corners[0],corners[i],corners[i+1]));source_faces.append(corners)
    return [positions],faces,source_faces

def synthetic():
    namespace={'__name__':'geometry_attribution_import'}
    exec(compile((ROOT/'tools/generate-gpu-workload.py').read_text(),'generate-gpu-workload.py','exec'),namespace)
    witness=json.loads((BUNDLE/'inputs/synthetic-repeated.step.json').read_text())
    positions,triangles=namespace['source_mesh'](witness['segments'])
    faces=[tuple(index-1 for index in face) for face in triangles]
    meshes=[]
    for instance in witness['placements']:
        m=instance['transform_row_major']
        meshes.append([tuple(sum(m[r][j]*p[j] for j in range(3))+m[r][3] for r in range(3)) for p in positions])
    return meshes,faces,None

def analyze(name, mesh_data, targets):
    meshes,faces,source_faces=mesh_data
    coordinate_scale=max(abs(v) for mesh in meshes for p in mesh for v in p)
    scaled=[[div(p,coordinate_scale) for p in mesh] for mesh in meshes]
    lower=[min(p[i] for mesh in scaled for p in mesh) for i in range(3)]
    upper=[max(p[i] for mesh in scaled for p in mesh) for i in range(3)]
    center=tuple(lower[i]+(upper[i]-lower[i])*.5 for i in range(3));extent=max(b-a for a,b in zip(lower,upper))
    normalized=[[div(sub(p,center),extent) for p in mesh] for mesh in scaled]
    camera_points=[[(dot(p,RIGHT),-dot(p,UP),dot(p,TOWARD)) for p in mesh] for mesh in normalized]
    lower=[min(p[i] for mesh in camera_points for p in mesh) for i in range(3)]
    upper=[max(p[i] for mesh in camera_points for p in mesh) for i in range(3)]
    camera_center=tuple(lower[i]+(upper[i]-lower[i])*.5 for i in range(3))
    pixels=min((512-2*512*.075)/(upper[i]-lower[i]) for i in range(2))
    projected=[[((p[0]-camera_center[0])*pixels+256,(p[1]-camera_center[1])*pixels+256,p[2]) for p in mesh] for mesh in camera_points]
    points=[]
    for samples,x,y in targets:
        for sy in range(samples):
            for sx in range(samples):
                points.append({'pixel':[x,y],'samples':samples,'subsample':[sx,sy],'sample_xy':[x+(sx+.5)/samples,y+(sy+.5)/samples],'candidates':[]})
    for instance,(world,mesh) in enumerate(zip(scaled,projected)):
        for face_index,indices in enumerate(faces):
            a,b,c=(mesh[i] for i in indices)
            minx,maxx=min(a[0],b[0],c[0]),max(a[0],b[0],c[0]); miny,maxy=min(a[1],b[1],c[1]),max(a[1],b[1],c[1])
            area=edge(a,b,c)
            if abs(area)<1e-12:continue
            for point in points:
                p=point['sample_xy']
                if not minx-.02<=p[0]<=maxx+.02 or not miny-.02<=p[1]<=maxy+.02:continue
                weights=[edge(b,c,p)/area,edge(c,a,p)/area,edge(a,b,p)/area]
                covered=min(weights)>=-1e-10
                edges=[(b,c),(c,a),(a,b)]
                distances=[abs(edge(u,v,p))/math.hypot(v[0]-u[0],v[1]-u[1]) if u[:2]!=v[:2] else math.inf for u,v in edges]
                if not covered and not (min(distances)<.02 and min(weights)>-.02):continue
                depth=sum(w*v[2] for w,v in zip(weights,(a,b,c)))
                color=shade(normal(*(world[i] for i in indices)))
                nearest=min(range(3),key=lambda i:distances[i])
                item={'instance':instance,'triangle_fan_index':face_index,'position_indices':list(indices),'cpu_covered':covered,'cpu_depth_f32':f32(depth),'depth_f64':depth,'color_rgb':color,'barycentric_weights':weights,'nearest_edge_distance_pixels':distances[nearest],'nearest_edge_xy':[list(edges[nearest][0][:2]),list(edges[nearest][1][:2])]}
                if source_faces is not None:item['obj_face_position_indices']=source_faces[face_index]
                point['candidates'].append(item)
    for point in points:
        point['candidates'].sort(key=lambda item:(item['cpu_depth_f32'],-item['instance'],-item['triangle_fan_index']),reverse=True)
        point['cpu_winner']=next((item for item in point['candidates'] if item['cpu_covered']),None)
        point['candidates']=point['candidates'][:10]
        winner=point['cpu_winner']
        print(name,point['pixel'],'s',point['samples'],point['subsample'],'winner',None if winner is None else (winner['instance'],winner['triangle_fan_index'],winner['color_rgb'],winner['nearest_edge_distance_pixels']),flush=True)
    report={'input':name,'method':'Independent Python projection and f64 barycentric evaluation using source positions; OBJ polygons fan-triangulated (same planar faces, not exact importer triangulation). Actual GPU primitive coverage is not read back.','points':points,'fit':{'coordinate_scale':coordinate_scale,'center':center,'extent':extent,'projection_pixels':pixels,'camera_center':camera_center}}
    (OUT/f'{name}-geometry-attribution.json').write_text(json.dumps(report,indent=2)+'\n')

analyze('gear',gear(),[(1,406,313),(2,462,229),(2,184,247),(2,327,247)])
analyze('synthetic-repeated',synthetic(),[(1,90,302),(1,421,302),(2,122,317)])
