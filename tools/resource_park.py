"""Place static park objects and export a redistributable SKATE08 resource.

Standard-library only. Coordinates use Y-up metres. The editor JSON is the
save/load format; native grind polylines are exported independently of meshes.
"""
from __future__ import annotations
import argparse
import json
import math
from pathlib import Path
import struct

MAX_OBJECTS = 2048


def vector(value, length=3):
    if not isinstance(value, (list, tuple)) or len(value) != length:
        raise ValueError(f"expected {length} coordinates")
    result = [float(v) for v in value]
    if any(not math.isfinite(v) or abs(v) > 99999 for v in result):
        raise ValueError("coordinates must be finite and within 99999 metres")
    return result


def validate(scene):
    if not isinstance(scene, dict):
        raise ValueError("scene must be an object")
    if scene.get("format") != 1 or not isinstance(scene.get("name"), str) or not scene["name"]:
        raise ValueError("expected format 1 and a park name")
    vector(scene["spawn"])
    if not math.isfinite(float(scene.get("heading", 0))):
        raise ValueError("heading must be finite")
    objects = scene.get("objects")
    if not isinstance(objects, list) or len(objects) > MAX_OBJECTS:
        raise ValueError("park object budget exceeded")
    ids = set()
    for item in objects:
        if not isinstance(item, dict):
            raise ValueError("park objects must be objects")
        key = item.get("id", "")
        if not isinstance(key, str) or not key or len(key) > 64 or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789_-" for c in key) or key in ids:
            raise ValueError("object IDs must be unique portable identifiers")
        ids.add(key)
        if item.get("kind") not in ("box", "ramp", "rail", "marker"):
            raise ValueError(f"{key}: supported kinds are box, ramp, rail, marker")
        vector(item["position"])
        size = vector(item.get("size", [1, 1, 1]))
        if any(x <= 0 or x > 1000 for x in size):
            raise ValueError(f"{key}: sizes must be positive and at most 1000 metres")
        if any(x < 0 or x > 1 for x in vector(item.get("color", [.55, .6, .65]))):
            raise ValueError(f"{key}: colors must be in 0..1")
        rotation = item.get("rotation", 0)
        if not isinstance(rotation, (int, float)) or not math.isfinite(rotation) or abs(rotation) > 36000:
            raise ValueError(f"{key}: rotation must be finite degrees within 36000")
        if item["kind"] == "marker":
            if item.get("marker_type", "interaction") not in ("interaction", "checkpoint", "spawn"):
                raise ValueError(f"{key}: invalid marker type")
            if type(item.get("order", 0)) is not int or not 0 <= item.get("order", 0) <= 2048:
                raise ValueError(f"{key}: checkpoint order must be 0..2048")
            if not isinstance(item.get("label", key), str) or len(item.get("label", key)) > 128:
                raise ValueError(f"{key}: label must be at most 128 characters")
        if item["kind"] == "rail":
            points = item.get("points", [])
            if not isinstance(points, list) or not 2 <= len(points) <= 1024:
                raise ValueError(f"{key}: rails require 2..1024 local points")
            for point in points:
                vector(point)
            pairs = list(zip(points, points[1:]))
            if item.get("closed"): pairs.append((points[-1], points[0]))
            if any(sum((float(x)-float(y))**2 for x,y in zip(a,b)) < 1e-10 for a, b in pairs):
                raise ValueError(f"{key}: rail segments must have length")
    return scene


def load(path):
    if path.stat().st_size > 4 * 1024 * 1024:
        raise ValueError("placement file exceeds 4 MiB")
    return validate(json.loads(path.read_text()))


def save(path, scene):
    validate(scene)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(scene, indent=2) + "\n")
    temporary.replace(path)


def make_scene(name):
    return {"format": 1, "name": name, "spawn": [-8, 1, 0], "heading": 0,
            "objects": [{"id": "floor", "kind": "box", "position": [0, -.25, 0],
                         "size": [50, .5, 50], "color": [.3, .34, .4]}]}


def transform(item, point, scale=False):
    """Yaw in degrees around +Y; shared by meshes and native rail metadata."""
    x, y, z = vector(point)
    if scale:
        size = vector(item.get("size", [1, 1, 1]))
        x, y, z = x * size[0], y * size[1], z * size[2]
    angle = math.radians(item.get("rotation", 0))
    c, s = math.cos(angle), math.sin(angle)
    p = item["position"]
    return vector([p[0] + x*c + z*s, p[1] + y, p[2] - x*s + z*c])


def rail_points(item):
    return [transform(item, point, scale=True) for point in item["points"]]


def triangles(item):
    """World-space triangles with author winding and independent visual rails."""
    p = vector(item["position"])
    size = vector(item.get("size", [1, 1, 1]))
    if item["kind"] == "rail":
        points = rail_points(item)
        pairs = list(zip(points, points[1:]))
        if item.get("closed"):
            pairs.append((points[-1], points[0]))
        for a, b in pairs:
            direction = [b[i]-a[i] for i in range(3)]
            length = math.sqrt(sum(x*x for x in direction))
            if length < 1e-5:
                raise ValueError("zero-length rail")
            direction = [x/length for x in direction]
            ref = [0, 1, 0] if abs(direction[1]) < .9 else [1, 0, 0]
            right = [direction[1]*ref[2]-direction[2]*ref[1], direction[2]*ref[0]-direction[0]*ref[2], direction[0]*ref[1]-direction[1]*ref[0]]
            scale = .06 / math.sqrt(sum(x*x for x in right))
            right = [x*scale for x in right]
            up = [direction[1]*right[2]-direction[2]*right[1], direction[2]*right[0]-direction[0]*right[2], direction[0]*right[1]-direction[1]*right[0]]
            corners = [[end[i]+r*right[i]+u*up[i] for i in range(3)] for end in (a, b) for r, u in ((-1,-1),(1,-1),(1,1),(-1,1))]
            yield from faces(corners, [(0,3,2,1),(4,5,6,7),(0,1,5,4),(3,7,6,2),(0,4,7,3),(1,2,6,5)])
        return
    if item["kind"] == "marker":
        r = min(size[0], 1) * .3
        corners = [[p[0]+x*r, p[1]+y*r, p[2]+z*r] for x,y,z in [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)]]
        for face in [(0,2,4),(4,2,1),(1,2,5),(5,2,0),(0,4,3),(4,1,3),(1,5,3),(5,0,3)]:
            yield [corners[i] for i in face]
        return
    x,y,z = [v/2 for v in size]
    if item["kind"] == "ramp":
        points = [(-x,-y,-z),(x,-y,-z),(x,-y,z),(-x,-y,z),(-x,y,z),(x,y,z)]
        quads = [(0,3,2,1),(0,1,5,4),(3,4,5,2)]
        sides = [(0,4,3),(1,2,5)]
    else:
        points = [(-x,-y,-z),(x,-y,-z),(x,-y,z),(-x,-y,z),(-x,y,-z),(x,y,-z),(x,y,z),(-x,y,z)]
        quads = [(0,3,2,1),(4,5,6,7),(0,1,5,4),(3,7,6,2),(0,4,7,3),(1,2,6,5)]
        sides = []
    corners = [transform(item, v) for v in points]
    yield from faces(corners, quads)
    for face in sides:
        yield outward(corners, face)


def outward(points, face):
    tri=[points[i] for i in face]
    center=[sum(p[i] for p in points)/len(points) for i in range(3)]
    a,b,c=tri;ab=[b[i]-a[i] for i in range(3)];ac=[c[i]-a[i] for i in range(3)]
    cross=[ab[1]*ac[2]-ab[2]*ac[1],ab[2]*ac[0]-ab[0]*ac[2],ab[0]*ac[1]-ab[1]*ac[0]]
    if sum(cross[i]*(a[i]-center[i]) for i in range(3))<0:tri=[a,c,b]
    return tri


def faces(points, quads):
    for a,b,c,d in quads:
        yield outward(points,(a,b,c))
        yield outward(points,(a,c,d))


def encode(scene, render_only=False):
    validate(scene)
    objects = scene["objects"]
    render = [(index+1, tri) for index,item in enumerate(objects) for tri in triangles(item)]
    collision = [] if render_only else [(index+1, tri) for index,item in enumerate(objects) if item["kind"] != "marker" for tri in triangles(item)]
    rails = [] if render_only else [item for item in objects if item["kind"] == "rail"]
    if not collision and not render_only:
        raise ValueError("park needs at least one collidable object")
    out = bytearray(b"SKATE08\0")
    def u(*v): out.extend(struct.pack("<"+"I"*len(v), *v))
    def f(*v): out.extend(struct.pack("<"+"f"*len(v), *v))
    def string(value):
        data = value.encode(); u(len(data)); out.extend(data)
    u(0x12345678); string(scene["name"]); f(*scene["spawn"],scene.get("heading",0))
    f(.09,.34,.72,.58,.78,.98,.18,.25,.34,0,12,.62,17,0)
    f(.045,.10,.26,1,.32,.10,.05,.035,.06,.007,.015,.045,.045,.085,.17,.008,.014,.032)
    f(1,.92,.78,.42,.56,.92,1.25,.18,.32,.11,1,1,1)
    u(len(objects),0,len(render)*3,len(render)*3,len(collision),len(rails),0,0,0)
    for item in objects:
        string(item["id"]);u(1);f(.7,.05,*item.get("color",[.55,.6,.65]),.75,0)
        u(0,0);f(1);u(0,0,0,0);f(.5);u(3,1,0)
    for material, tri in render:
        a,b,c = tri; ab = [b[i]-a[i] for i in range(3)]; ac = [c[i]-a[i] for i in range(3)]
        n = [ab[1]*ac[2]-ab[2]*ac[1],ab[2]*ac[0]-ab[0]*ac[2],ab[0]*ac[1]-ab[1]*ac[0]]
        length = math.sqrt(sum(x*x for x in n))
        if length < 1e-8: raise ValueError("degenerate geometry")
        n = [v/length for v in n]
        for point in tri: f(*point,*n,0,0,0,0);u(material)
    for i in range(len(render)*3):u(i)
    for material,tri in collision:
        for point in tri:f(*point)
        u(1,material)
    for item in rails:
        string(item["id"]);u(int(item.get("closed",False)));u(len(item["points"]))
        for point in rail_points(item):f(*point)
    return bytes(out)


def export(scene, root, resource_id, lod_scene=None, lod_distance=200):
    validate(scene)
    if not resource_id or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789_-" for c in resource_id):
        raise ValueError("resource ID must be portable lowercase ASCII")
    manifest_path = root / "resource.json"
    previous = json.loads(manifest_path.read_text()) if manifest_path.exists() else None
    if previous is not None and previous.get("id") != resource_id:
        raise ValueError("existing resource ID differs; export to another directory")
    payload = encode(scene)
    far_payload = None
    if lod_scene is not None:
        if not isinstance(lod_distance, int) or not 10 <= lod_distance <= 10000:
            raise ValueError("LOD distance must be an integer from 10 to 10000 metres")
        far_payload = encode(lod_scene,render_only=True)
    root.mkdir(parents=True, exist_ok=True)
    (root/"park.skate").write_bytes(payload)
    markers = [{"id":o["id"],"position":o["position"],"radius":o.get("size",[1])[0],"type":o.get("marker_type","interaction"),"order":o.get("order",0),"label":o.get("label",o["id"])} for o in scene["objects"] if o["kind"]=="marker"]
    (root/"markers.json").write_text(json.dumps(markers,indent=2)+"\n")
    save(root/"placements.json",scene)
    manifest = {"format":1,"api":1,"id":resource_id,"version":"1.0.0","language":"lua",
                "files":["park.skate","markers.json","placements.json"],"world":{"map":"park.skate","required":True}}
    if previous is not None:
        generated = {"park.skate", "markers.json", "placements.json", "park-low.skate", "placements-low.json"}
        extras = [name for name in previous.get("files", []) if name not in generated]
        old_world = previous.get("world", {})
        manifest = dict(previous, files=extras + manifest["files"], world=manifest["world"])
        if far_payload is None and old_world.get("lods"):
            manifest["world"]["lods"] = old_world["lods"]
            manifest["files"].extend(name for name in previous.get("files", []) if name in {"park-low.skate", "placements-low.json"})
    if far_payload is not None:
        (root/"park-low.skate").write_bytes(far_payload)
        save(root/"placements-low.json",lod_scene)
        manifest["files"].extend(["park-low.skate","placements-low.json"])
        manifest["world"]["lods"]=[{"map":"park-low.skate","distance":lod_distance}]
    (root/"resource.json").write_text(json.dumps(manifest,indent=2)+"\n")
    return len(payload)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest="command",required=True)
    new=sub.add_parser("new");new.add_argument("scene",type=Path);new.add_argument("--name",default="Community park")
    listing=sub.add_parser("list");listing.add_argument("scene",type=Path)
    for command in ("place","update"):
        p=sub.add_parser(command);p.add_argument("scene",type=Path);p.add_argument("--id",required=True)
        p.add_argument("--kind",choices=["box","ramp","rail","marker"]);p.add_argument("--position",nargs=3,type=float)
        p.add_argument("--rotation",type=float);p.add_argument("--marker-type",choices=["interaction","checkpoint","spawn"]);p.add_argument("--order",type=int);p.add_argument("--label");p.add_argument("--size",nargs=3,type=float);p.add_argument("--color",nargs=3,type=float)
        p.add_argument("--point",action="append",nargs=3,type=float);p.add_argument("--closed",action="store_true",default=None)
    remove=sub.add_parser("remove");remove.add_argument("scene",type=Path);remove.add_argument("--id",required=True)
    out=sub.add_parser("export");out.add_argument("scene",type=Path);out.add_argument("--root",type=Path,required=True);out.add_argument("--resource-id",required=True)
    out.add_argument("--lod-scene",type=Path);out.add_argument("--lod-distance",type=int,default=200)
    args=parser.parse_args()
    try:
        if args.command=="new":save(args.scene,make_scene(args.name));return
        scene=load(args.scene)
        if args.command=="list":print(json.dumps(scene,indent=2));return
        if args.command=="export":print(f"Exported {export(scene,args.root,args.resource_id,load(args.lod_scene) if args.lod_scene else None,args.lod_distance)} base bytes to {args.root}");return
        found=next((o for o in scene["objects"] if o["id"]==args.id),None)
        if args.command=="remove":
            if found is None:raise ValueError("unknown object ID")
            scene["objects"].remove(found)
        else:
            if (args.command=="place") == (found is not None):raise ValueError("place requires a new ID; update requires an existing ID")
            item=dict(found or {"id":args.id,"kind":"box","position":[0,0,0]})
            for field in ("kind","position","size","color","closed","rotation","marker_type","order","label"):
                if getattr(args,field) is not None:item[field]=getattr(args,field)
            if args.point is not None:item["points"]=args.point
            if found is not None:scene["objects"].remove(found)
            scene["objects"].append(item)
        save(args.scene,scene)
    except (ValueError,OSError,KeyError,TypeError) as error:parser.error(str(error))

if __name__=="__main__":main()
