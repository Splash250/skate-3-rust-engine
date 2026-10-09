"""Prepare a local interior without relaxing downloaded-resource import budgets.

Requires an independently authored shell in the source model's coordinate frame.
The output directory must not exist: publication is atomic and never replaces an
installed generation. Geometry clustering preserves material/UV/normal seams;
its maximum position displacement is recorded for visual acceptance.
"""
import argparse
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile

import numpy as np
from PIL import Image

BUDGET = 16 * 1024 * 1024


def read_glb(raw):
    if len(raw) < 20 or struct.unpack_from('<4sII', raw) != (b'glTF', 2, len(raw)):
        raise ValueError('invalid GLB header')
    chunks = {}
    offset = 12
    while offset < len(raw):
        size, kind = struct.unpack_from('<II', raw, offset)
        offset += 8
        if offset + size > len(raw) or kind in chunks:
            raise ValueError('invalid GLB chunk')
        chunks[kind] = raw[offset:offset+size]
        offset += size
    return json.loads(chunks[0x4e4f534a]), chunks.get(0x004e4942, b'')


def write_glb(doc, binary):
    doc = copy.deepcopy(doc)
    doc['buffers'] = [{'byteLength': len(binary)}]
    js = json.dumps(doc, separators=(',', ':'), allow_nan=False).encode()
    js += b' ' * (-len(js) % 4)
    binary += b'\0' * (-len(binary) % 4)
    return (struct.pack('<4sII', b'glTF', 2, 28+len(js)+len(binary)) +
            struct.pack('<II', len(js), 0x4e4f534a) + js +
            struct.pack('<II', len(binary), 0x004e4942) + binary)


def _shell(path, origin_y):
    raw = path.read_bytes()
    if len(raw) > 4 * 1024 * 1024:
        raise ValueError('shell exceeds 4 MiB')
    shell = json.loads(raw)
    if set(shell) != {'version', 'positions', 'triangles'} or shell['version'] != 1:
        raise ValueError('invalid shell schema')
    pos = np.asarray(shell['positions'], dtype=np.float64)
    idx = np.asarray(shell['triangles'])
    if (pos.ndim != 2 or pos.shape[1] != 3 or not 0 < len(pos) <= 65536 or
            idx.ndim != 2 or idx.shape[1] != 3 or not 0 < len(idx) <= 32768 or
            idx.dtype.kind not in 'iu' or not np.isfinite(pos).all() or
            idx.min() < 0 or idx.max() >= len(pos)):
        raise ValueError('invalid shell geometry')
    pos[:, 1] -= origin_y
    area = np.cross(pos[idx[:, 1]]-pos[idx[:, 0]], pos[idx[:, 2]]-pos[idx[:, 0]])
    if np.any(np.sum(area*area, axis=1) <= 1e-20):
        raise ValueError('degenerate shell triangles')
    shell['positions'] = pos.tolist()
    return shell


def prepare(source: Path, output: Path, *, shell=None, origin_y=0.0) -> dict:
    source, output = Path(source), Path(output)
    if shell is None:
        raise ValueError('an independently authored collision --shell is required')
    if not np.isfinite(origin_y) or output.exists():
        raise ValueError('finite origin and a new output directory required')
    collision = _shell(Path(shell), origin_y)
    raw = source.read_bytes()
    original, binary = read_glb(raw)
    if original.get('animations') or original.get('skins'):
        raise ValueError('only static interior scenes are supported')
    for node in original.get('nodes', []):
        if any(k in node for k in ('matrix', 'translation', 'rotation', 'scale', 'children')):
            raise ValueError('bake node transforms before preparing the interior')
    if any(b.get('uri') for b in original.get('buffers', [])):
        raise ValueError('source must embed buffers')
    doc = copy.deepcopy(original)
    doc.pop('extensionsUsed', None)
    doc.pop('extensionsRequired', None)
    for mat in doc.get('materials', []):
        extensions = mat.pop('extensions', {})
        if set(extensions) - {'KHR_materials_unlit'}:
            raise ValueError('unsupported source material extension')
        if 'KHR_materials_unlit' in extensions:
            pbr = mat.setdefault('pbrMetallicRoughness', {})
            factor = pbr.get('baseColorFactor', [1, 1, 1, 1])
            mat['emissiveFactor'] = factor[:3]
            if 'baseColorTexture' in pbr:
                mat['emissiveTexture'] = copy.deepcopy(pbr['baseColorTexture'])
            pbr['baseColorFactor'] = [0, 0, 0, factor[3]]
            pbr['metallicFactor'] = 0
            pbr['roughnessFactor'] = 1
    def reject_extensions(value):
        if isinstance(value, dict):
            if 'extensions' in value:
                raise ValueError('unsupported source extension')
            for v in value.values():
                reject_extensions(v)
        elif isinstance(value, list):
            for v in value:
                reject_extensions(v)
    reject_extensions(doc)
    images = []
    for image in original.get('images', []):
        if 'uri' in image:
            raise ValueError('source must embed images')
        view = original['bufferViews'][image['bufferView']]
        offset = view.get('byteOffset', 0)
        im = Image.open(io.BytesIO(binary[offset:offset+view['byteLength']])).convert('RGBA')
        im.thumbnail((2048, 2048), Image.Resampling.LANCZOS)
        images.append(im)
    references = [t['source'] for t in original.get('textures', [])]
    def texture_bytes():
        return max(sum(im.width*im.height*4 for im in images),
                   sum(images[i].width*images[i].height*4 for i in references))
    while texture_bytes() > BUDGET:
        # Reduce the largest allocation first, counting repeated texture sources.
        index = max(range(len(images)), key=lambda i: images[i].width*images[i].height*max(1, references.count(i)))
        im = images[index]
        if im.width == im.height == 1:
            raise ValueError('texture references exceed budget')
        images[index] = im.resize((max(1, im.width//2), max(1, im.height//2)), Image.Resampling.LANCZOS)
    def array(index):
        a = original['accessors'][index]
        if 'sparse' in a:
            raise ValueError('sparse source accessors unsupported')
        view = original['bufferViews'][a['bufferView']]
        width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
        dtype = np.dtype({5126: '<f4', 5125: '<u4', 5123: '<u2', 5121: 'u1'}[a['componentType']])
        offset = view.get('byteOffset', 0) + a.get('byteOffset', 0)
        values = np.ndarray((a['count'], width), dtype=dtype, buffer=binary, offset=offset,
                          strides=(view.get('byteStride', width*dtype.itemsize), dtype.itemsize)).copy()
        if a.get('normalized', False):
            if a['componentType'] not in (5121, 5123) or a['type'] == 'SCALAR':
                raise ValueError('unsupported normalized accessor')
            values = values.astype(np.float32) / np.iinfo(dtype).max
        return values
    primitives = []
    for mesh in original.get('meshes', []):
        for primitive in mesh['primitives']:
            if primitive.get('mode', 4) != 4 or primitive.get('targets'):
                raise ValueError('static triangles required')
            attrs = {k: array(v) for k, v in primitive['attributes'].items()}
            if 'NORMAL' not in attrs or set(attrs) - {'POSITION', 'NORMAL', 'TEXCOORD_0'}:
                raise ValueError('baked position, normal and UV attributes required')
            attrs['POSITION'][:, 1] -= origin_y
            indices = array(primitive['indices']).astype(np.uint32).reshape(-1, 3)
            if indices.max() >= len(attrs['POSITION']) or not all(np.isfinite(a).all() for a in attrs.values()):
                raise ValueError('invalid geometry')
            primitives.append((primitive, attrs, indices))
    # Quantized seam-aware vertex clustering, retaining a source vertex for each
    # cluster. No sampled/deleted faces: only faces collapsed by a cluster vanish.
    reduced = []
    used_step = 0
    max_displacement = 0.0
    for step in [0, .0025, .005, .01, .02]:
        reduced = []
        expanded = 0
        max_displacement = 0.0
        for primitive, attrs, indices in primitives:
            packed = np.concatenate([attrs[k] for k in sorted(attrs)], axis=1)
            if step:
                baked = 'KHR_materials_unlit' in original['materials'][primitive['material']].get('extensions', {})
                keys = np.concatenate([np.rint(attrs[k] / (step if k == 'POSITION' else .02 if k == 'NORMAL' else 1/1024)) for k in sorted(attrs) if not (baked and k == 'NORMAL')], axis=1)
            else:
                keys = packed
            _, first, inverse = np.unique(keys, axis=0, return_index=True, return_inverse=True)
            remapped = inverse[indices]
            remapped = remapped[(remapped[:,0] != remapped[:,1]) & (remapped[:,1] != remapped[:,2]) & (remapped[:,0] != remapped[:,2])]
            selected = {k: v[first] for k, v in attrs.items()}
            area = np.cross(selected['POSITION'][remapped[:,1]]-selected['POSITION'][remapped[:,0]], selected['POSITION'][remapped[:,2]]-selected['POSITION'][remapped[:,0]])
            remapped = remapped[np.sum(area*area, axis=1) > 1e-20]
            max_displacement = max(max_displacement, float(np.linalg.norm(attrs['POSITION']-selected['POSITION'][inverse], axis=1).max()))
            # Compact unused vertices after collapsed faces.
            used, inv = np.unique(remapped, return_inverse=True)
            selected = {k: v[used] for k, v in selected.items()}
            remapped = inv.reshape(-1, 3).astype(np.uint32)
            expanded += len(used)*(sum(a.shape[1]*4 for a in selected.values())+28) + remapped.size*4
            reduced.append((primitive, selected, remapped))
        # Include repeated boundary vertices introduced by bounded index chunks.
        expanded = sum(len(np.unique(indices[start:start+87381])) * (sum(a.shape[1]*4 for a in attrs.values())+28) + indices[start:start+87381].size*4
                       for _, attrs, indices in reduced for start in range(0, len(indices), 87381))
        if expanded <= BUDGET:
            used_step = step
            break
    else:
        raise ValueError('geometry cannot fit the existing 16 MiB budget within 2cm clustering')
    doc['bufferViews'], doc['accessors'] = [], []
    blob = bytearray()
    def add_view(data, target=None):
        blob.extend(b'\0' * (-len(blob) % 4))
        v = {'buffer': 0, 'byteOffset': len(blob), 'byteLength': len(data)}
        if target:
            v['target'] = target
        index = len(doc['bufferViews'])
        doc['bufferViews'].append(v)
        blob.extend(data)
        return index
    for i, im in enumerate(images):
        encoded = io.BytesIO()
        mime = original['images'][i]['mimeType']
        if mime == 'image/jpeg':
            im.convert('RGB').save(encoded, format='JPEG', quality=95, subsampling=0)
        else:
            im.save(encoded, format='PNG')
        doc['images'][i] = {'bufferView': add_view(encoded.getvalue()), 'mimeType': mime}
    def add_accessor(data, kind):
        data = np.asarray(data, dtype='<u4' if kind == 'SCALAR' else '<f4')
        a = {'bufferView': add_view(data.tobytes(), 34963 if kind == 'SCALAR' else 34962),
             'componentType': 5125 if kind == 'SCALAR' else 5126, 'count': len(data), 'type': kind}
        if kind == 'VEC3':
            a.update(min=data.min(axis=0).tolist(), max=data.max(axis=0).tolist())
        index = len(doc['accessors'])
        doc['accessors'].append(a)
        return index
    # Split large index accessors under the existing 262144-element cap, while
    # compacting attributes per chunk so reader-use charging stays honest.
    cursor = 0
    for mesh in doc.get('meshes', []):
        out = []
        for _ in mesh['primitives']:
            primitive, attrs, indices = reduced[cursor]
            cursor += 1
            for start in range(0, len(indices), 87381):
                chunk = indices[start:start+87381]
                used, inv = np.unique(chunk, return_inverse=True)
                p = copy.deepcopy(primitive)
                p['attributes'] = {k: add_accessor(a[used], f'VEC{a.shape[1]}') for k, a in attrs.items()}
                p['indices'] = add_accessor(inv.astype(np.uint32).reshape(-1), 'SCALAR')
                out.append(p)
        mesh['primitives'] = out
    expanded = sum(doc['accessors'][p['attributes']['POSITION']]['count'] *
                   (sum({'VEC2': 8, 'VEC3': 12}[doc['accessors'][a]['type']] for a in p['attributes'].values())+28)
                   + doc['accessors'][p['indices']]['count']*4 for m in doc.get('meshes', []) for p in m['primitives'])
    result = write_glb(doc, bytes(blob))
    if len(result) > BUDGET or expanded > BUDGET:
        raise ValueError(f'encoded {len(result)} or expanded {expanded} model exceeds 16 MiB after chunking (step {used_step})')
    report = {'source_sha256': hashlib.sha256(raw).hexdigest(), 'runtime_sha256': hashlib.sha256(result).hexdigest(),
              'decoded_texture_bytes': texture_bytes(), 'expanded_geometry_bytes': expanded,
              'encoded_bytes': len(result), 'origin_y': origin_y, 'cluster_step_m': used_step,
              'maximum_vertex_displacement_m': max_displacement,
              'triangles': sum(a.size//3 for _, _, a in reduced),
              'collision_sha256': hashlib.sha256(json.dumps(collision, separators=(',', ':'), allow_nan=False).encode()).hexdigest()}
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.interior-', dir=output.parent))
    try:
        (staging/'apartment.glb').write_bytes(result)
        (staging/'collision.json').write_text(json.dumps(collision, separators=(',', ':'), allow_nan=False))
        (staging/'preparation.json').write_text(json.dumps(report, indent=2)+'\n')
        attribution = original.get('asset', {}).get('copyright', 'See source attribution')
        source_note = source.parent/'README.txt'
        (staging/'ATTRIBUTION.txt').write_text(attribution+'\n'+(source_note.read_text() if source_note.exists() else ''))
        os.rename(staging, output)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--shell', type=Path, required=True)
    parser.add_argument('--origin-y', type=float, default=0)
    args = parser.parse_args()
    print(json.dumps(prepare(args.source, args.output, shell=args.shell, origin_y=args.origin_y), indent=2))
