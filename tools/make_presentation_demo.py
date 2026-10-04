#!/usr/bin/env python3
"""Generate original articulated robot and hat GLBs for the public resource.

Only public bone names/parent relationships establish compatibility. All mesh
vertices, colors and identity bind frames below are original procedural geometry.
No retail vertices, textures, bind poses, animation samples or private files are
read by this generator. The hat's authored cylinder axis is local Z; its resource
attachment supplies the offset and rotation for the target bone's live basis.
"""
import json
import math
from pathlib import Path
import struct

BONES = [
    ('TRAJECTORY', None), ('HIPS', 'TRAJECTORY'), ('SPINE', 'HIPS'),
    ('SPINE1', 'SPINE'), ('SPINE2', 'SPINE1'), ('SPINE3', 'SPINE2'),
    ('NECK', 'SPINE3'), ('NECK1', 'NECK'), ('HEAD', 'NECK1'),
    ('RIGHTSHOULDER', 'SPINE3'), ('RIGHTARM', 'RIGHTSHOULDER'),
    ('RIGHTFOREARM', 'RIGHTARM'), ('RIGHTHAND', 'RIGHTFOREARM'),
    ('LEFTSHOULDER', 'SPINE3'), ('LEFTARM', 'LEFTSHOULDER'),
    ('LEFTFOREARM', 'LEFTARM'), ('LEFTHAND', 'LEFTFOREARM'),
    ('RIGHTUPLEG', 'HIPS'), ('RIGHTLEG', 'RIGHTUPLEG'),
    ('RIGHTFOOT', 'RIGHTLEG'), ('RIGHTTOEBASE', 'RIGHTFOOT'),
    ('LEFTUPLEG', 'HIPS'), ('LEFTLEG', 'LEFTUPLEG'),
    ('LEFTFOOT', 'LEFTLEG'), ('LEFTTOEBASE', 'LEFTFOOT'),
]
BLUE = (.08, .55, .85, 1.)
CYAN = (.22, .86, .94, 1.)
DARK = (.025, .055, .10, 1.)
GOLD = (1., .56, .08, 1.)
WHITE = (.9, .96, 1., 1.)


def make_glb(path: Path, skinned: bool) -> None:
    positions, normals, colors, joints, weights = [], [], [], [], []
    names = [name for name, _ in BONES]

    def vertex(point, normal, color, blend):
        positions.extend(point)
        normals.extend(normal)
        colors.extend(color)
        if skinned:
            joints.extend([names.index(name) for name, _ in blend] + [0]*(4-len(blend)))
            weights.extend([weight for _, weight in blend] + [0.]*(4-len(blend)))

    def sphere(radius, color, blend=(), center=(0., 0., 0.), segments=10, rings=6):
        def point(row, column):
            phi = math.pi*row/rings
            theta = 2*math.pi*column/segments
            direction = (math.sin(phi)*math.cos(theta), math.sin(phi)*math.sin(theta), math.cos(phi))
            position = tuple(center[i] + radius[i]*direction[i] for i in range(3))
            normal = [direction[i]/radius[i] for i in range(3)]
            length = math.sqrt(sum(v*v for v in normal))
            return position, tuple(v/length for v in normal)
        for row in range(rings):
            for column in range(segments):
                a,b,c,d = point(row,column),point(row+1,column),point(row+1,column+1),point(row,column+1)
                triangles = ([] if row == 0 else [(a,b,d)]) + ([] if row == rings-1 else [(b,c,d)])
                for triangle in triangles:
                    for position, normal in triangle:
                        vertex(position, normal, color, blend)

    def cylinder(radius, bottom, top, color, segments=24):
        for i in range(segments):
            angle, next_angle = 2*math.pi*i/segments, 2*math.pi*(i+1)/segments
            a=(radius*math.cos(angle),radius*math.sin(angle))
            b=(radius*math.cos(next_angle),radius*math.sin(next_angle))
            side = [((*a,bottom),(*a,0.)),((*b,bottom),(*b,0.)),((*a,top),(*a,0.)),
                    ((*a,top),(*a,0.)),((*b,bottom),(*b,0.)),((*b,top),(*b,0.))]
            for p,n in side: vertex(p,tuple(v/radius for v in n),color,())
            for p in [(0.,0.,top),(*a,top),(*b,top)]: vertex(p,(0.,0.,1.),color,())
            for p in [(0.,0.,bottom),(*b,bottom),(*a,bottom)]: vertex(p,(0.,0.,-1.),color,())

    if skinned:
        # Identity inverse binds deliberately author each rounded component in
        # its joint's coordinates. Multi-joint spheres interpolate their centers
        # between the live named endpoints, without copying a proprietary rest pose.
        for name, radius, color in [
            ('HIPS',(.20,.17,.17),BLUE), ('SPINE2',(.23,.16,.20),BLUE),
            ('SPINE3',(.25,.17,.20),CYAN), ('HEAD',(.25,.215,.235),WHITE),
            ('RIGHTSHOULDER',(.13,.13,.13),GOLD), ('LEFTSHOULDER',(.13,.13,.13),GOLD),
            ('RIGHTFOREARM',(.095,.095,.095),DARK), ('LEFTFOREARM',(.095,.095,.095),DARK),
            ('RIGHTHAND',(.10,.10,.12),GOLD), ('LEFTHAND',(.10,.10,.12),GOLD),
            ('RIGHTUPLEG',(.115,.115,.115),DARK), ('LEFTUPLEG',(.115,.115,.115),DARK),
            ('RIGHTLEG',(.105,.105,.105),GOLD), ('LEFTLEG',(.105,.105,.105),GOLD),
            ('RIGHTFOOT',(.12,.20,.10),DARK), ('LEFTFOOT',(.12,.20,.10),DARK),
        ]:
            sphere(radius,color,[(name,1.)],segments=16 if name=='HEAD' else 10)
        for first, second, radius, color in [
            ('HIPS','SPINE3',.17,BLUE),
            ('RIGHTARM','RIGHTFOREARM',.075,CYAN), ('LEFTARM','LEFTFOREARM',.075,CYAN),
            ('RIGHTFOREARM','RIGHTHAND',.065,BLUE), ('LEFTFOREARM','LEFTHAND',.065,BLUE),
            ('RIGHTUPLEG','RIGHTLEG',.09,BLUE), ('LEFTUPLEG','LEFTLEG',.09,BLUE),
            ('RIGHTLEG','RIGHTFOOT',.08,CYAN), ('LEFTLEG','LEFTFOOT',.08,CYAN),
        ]:
            for t in [.2,.4,.6,.8]:
                sphere((radius,)*3,color,[(first,1-t),(second,t)])
        # Live HEAD local +X is up, confirmed with native attachment-axis
        # captures. Keep the band horizontal and eyes side by side along Z.
        sphere((.085,.222,.241),DARK,[('HEAD',1.)],center=(.02,0.,0.),segments=16)
        for z in [-.085,.085]:
            sphere((.055,.038,.042),CYAN,[('HEAD',1.)],center=(.035,-.212,z),segments=8)
    else:
        cylinder(.31,0.,.035,GOLD)
        cylinder(.21,.035,.245,GOLD)
        cylinder(.216,.045,.10,DARK)
        cylinder(.214,.245,.27,WHITE)

    blob = bytearray()
    views, accessors = [], []
    def accessor(values, fmt, component, kind, count, **extra):
        while len(blob) % 4: blob.append(0)
        offset = len(blob)
        blob.extend(struct.pack('<'+fmt*len(values),*values))
        views.append({'buffer':0,'byteOffset':offset,'byteLength':len(blob)-offset})
        accessors.append({'bufferView':len(views)-1,'componentType':component,'count':count,'type':kind,**extra})
        return len(accessors)-1
    count = len(positions)//3
    attributes = {
        'POSITION':accessor(positions,'f',5126,'VEC3',count,min=[min(positions[i::3]) for i in range(3)],max=[max(positions[i::3]) for i in range(3)]),
        'NORMAL':accessor(normals,'f',5126,'VEC3',count),
        'COLOR_0':accessor(colors,'f',5126,'VEC4',count),
    }
    if skinned:
        nodes = [{'name':name} for name in names]
        for i,(_,parent) in enumerate(BONES):
            if parent is not None: nodes[names.index(parent)].setdefault('children',[]).append(i)
        attributes['JOINTS_0'] = accessor(joints,'H',5123,'VEC4',count)
        attributes['WEIGHTS_0'] = accessor(weights,'f',5126,'VEC4',count)
        identity = [1.,0.,0.,0., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.]
        inverse = accessor(identity*len(names),'f',5126,'MAT4',len(names))
        skins = [{'joints':list(range(len(names))),'skeleton':0,'inverseBindMatrices':inverse}]
        nodes.append({'name':'Original articulated resource robot','mesh':0,'skin':0})
        roots = [0,len(names)]
    else:
        nodes,roots,skins = [{'name':'Original resource top hat','mesh':0}],[0],[]
    document = {'asset':{'version':'2.0','generator':'Original skate resource procedural robot'},'scene':0,
        'scenes':[{'nodes':roots}],'nodes':nodes,'meshes':[{'primitives':[{'attributes':attributes,'material':0}]}],
        'materials':[{'pbrMetallicRoughness':{'baseColorFactor':[1,1,1,1],'metallicFactor':.12,'roughnessFactor':.55},'doubleSided':True}],
        'buffers':[{'byteLength':len(blob)}],'bufferViews':views,'accessors':accessors}
    if skins: document['skins']=skins
    encoded=json.dumps(document,separators=(',',':')).encode()
    encoded+=b' '*(-len(encoded)%4);blob+=b'\0'*(-len(blob)%4)
    path.write_bytes(struct.pack('<III',0x46546C67,2,12+8+len(encoded)+8+len(blob))+struct.pack('<II',len(encoded),0x4E4F534A)+encoded+struct.pack('<II',len(blob),0x004E4942)+blob)


if __name__=='__main__':
    destination=Path(__file__).resolve().parents[1]/'resources'/'presentation-demo'
    make_glb(destination/'mascot.glb',True)
    make_glb(destination/'hat.glb',False)
