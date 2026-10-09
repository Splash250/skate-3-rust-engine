import io
import json
from pathlib import Path
import tempfile
import unittest
from PIL import Image
from tools.prepare_interior import prepare, read_glb, write_glb


class InteriorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        image = io.BytesIO()
        Image.new('RGBA', (4096, 32), (180, 70, 30, 255)).save(image, format='PNG')
        payload = image.getvalue()
        doc = {'asset': {'version': '2.0', 'copyright': 'Synthetic test author'},
               'buffers': [{'byteLength': len(payload)}],
               'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': len(payload)}],
               'images': [{'bufferView': 0, 'mimeType': 'image/png'}],
               'textures': [{'source': 0}], 'materials': [{
                   'pbrMetallicRoughness': {'baseColorFactor': [1, 1, 1, 0.8], 'baseColorTexture': {'index': 0}},
                   'extensions': {'KHR_materials_unlit': {}}}],
               'extensionsUsed': ['KHR_materials_unlit']}
        self.source = self.root / 'source.glb'
        self.source.write_bytes(write_glb(doc, payload))
        self.shell = self.root / 'shell.json'
        self.shell.write_text(json.dumps({'version': 1, 'positions': [[0,17.52,0],[0,17.52,2],[2,17.52,0]], 'triangles': [[0,1,2]]}))

    def run_prepare(self):
        return prepare(self.source, self.root / 'output', shell=self.shell, origin_y=17.52)

    def test_embedded_images_fit_default_resource_budget(self):
        report = self.run_prepare()
        self.assertLessEqual(report['decoded_texture_bytes'], 16 * 1024 * 1024)
        doc, blob = read_glb((self.root / 'output/apartment.glb').read_bytes())
        view = doc['bufferViews'][doc['images'][0]['bufferView']]
        im = Image.open(io.BytesIO(blob[view['byteOffset']:view['byteOffset']+view['byteLength']]))
        self.assertLessEqual(max(im.size), 2048)
        self.assertEqual(im.getpixel((0,0)), (180,70,30,255))

    def test_baked_material_has_supported_equivalent(self):
        self.run_prepare()
        doc, _ = read_glb((self.root / 'output/apartment.glb').read_bytes())
        self.assertNotIn('extensionsUsed', doc)
        mat = doc['materials'][0]
        self.assertNotIn('extensions', mat)
        self.assertEqual(mat['emissiveTexture'], {'index': 0})
        self.assertEqual(mat['emissiveFactor'], [1,1,1])
        self.assertEqual(mat['pbrMetallicRoughness']['baseColorFactor'], [0,0,0,0.8])

    def test_source_is_unchanged(self):
        before = self.source.read_bytes()
        self.run_prepare()
        self.assertEqual(before, self.source.read_bytes())
        self.assertIn('Synthetic test author', (self.root / 'output/ATTRIBUTION.txt').read_text())

    def test_shell_floor_and_doorway_match_visual_transform(self):
        self.run_prepare()
        shell = json.loads((self.root / 'output/collision.json').read_text())
        self.assertEqual(shell['positions'][0], [0,0,0])
        self.assertEqual(shell['triangles'], [[0,1,2]])

    def test_missing_shell_cannot_publish_cosmetic_collision(self):
        with self.assertRaises(ValueError):
            prepare(self.source, self.root / 'output')
        self.assertFalse((self.root / 'output').exists())

    def add_triangle(self, normalized_uv=False):
        import struct
        doc, binary = read_glb(self.source.read_bytes())
        binary += b'\0' * (-len(binary) % 4)
        arrays = [(struct.pack('<9f', 0,17.52,0, 0,17.52,2, 2,17.52,0), 'VEC3', 5126, 3),
                  (struct.pack('<9f', *([0,1,0]*3)), 'VEC3', 5126, 3),
                  (struct.pack('<3I', 0,1,2), 'SCALAR', 5125, 3)]
        doc['accessors'] = []
        for data, kind, component, count in arrays:
            view = len(doc['bufferViews'])
            doc['bufferViews'].append({'buffer':0,'byteOffset':len(binary),'byteLength':len(data)})
            doc['accessors'].append({'bufferView':view,'type':kind,'componentType':component,'count':count})
            binary += data
        doc['meshes'] = [{'primitives':[{'attributes':{'POSITION':0,'NORMAL':1},'indices':2,'material':0}]}]
        if normalized_uv:
            data = struct.pack('<6H', 0,0, 65535,0, 0,65535)
            view = len(doc['bufferViews'])
            doc['bufferViews'].append({'buffer':0,'byteOffset':len(binary),'byteLength':len(data)})
            doc['accessors'].append({'bufferView':view,'type':'VEC2','componentType':5123,'count':3,'normalized':True})
            doc['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_0'] = 3
            binary += data
        self.source.write_bytes(write_glb(doc, binary))

    def test_triangle_indices_keep_all_corners_after_reindexing(self):
        self.add_triangle()
        self.run_prepare()
        runtime, _ = read_glb((self.root / 'output/apartment.glb').read_bytes())
        primitive = runtime['meshes'][0]['primitives'][0]
        self.assertEqual(runtime['accessors'][primitive['indices']]['count'], 3)

    def test_normalized_integer_uvs_keep_texture_coordinates(self):
        import struct
        self.add_triangle(normalized_uv=True)
        self.run_prepare()
        runtime, binary = read_glb((self.root / 'output/apartment.glb').read_bytes())
        primitive = runtime['meshes'][0]['primitives'][0]
        accessor = runtime['accessors'][primitive['attributes']['TEXCOORD_0']]
        view = runtime['bufferViews'][accessor['bufferView']]
        uv = struct.unpack_from('<6f', binary, view.get('byteOffset', 0))
        self.assertEqual(sorted(zip(uv[::2], uv[1::2])), [(0,0),(0,1),(1,0)])
