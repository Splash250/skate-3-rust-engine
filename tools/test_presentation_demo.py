"""Public fixture checks: geometry must exercise a complete articulated skin."""
import json
import struct
import tempfile
import unittest
from pathlib import Path

from tools.make_presentation_demo import make_glb


def decode(path):
    data = path.read_bytes()
    length = struct.unpack_from('<I', data, 12)[0]
    document = json.loads(data[20:20+length])
    binary = data[28+length:]

    def accessor(index):
        entry = document['accessors'][index]
        view = document['bufferViews'][entry['bufferView']]
        size = {'SCALAR':1, 'VEC3':3, 'VEC4':4, 'MAT4':16}[entry['type']]
        kind = {5126:'f', 5123:'H'}[entry['componentType']]
        offset = view.get('byteOffset', 0) + entry.get('byteOffset', 0)
        values = struct.unpack_from('<'+kind*(entry['count']*size), binary, offset)
        return [values[i:i+size] for i in range(0, len(values), size)]

    return document, accessor


class PresentationDemoTests(unittest.TestCase):
    def test_mascot_weights_cover_torso_both_arms_and_legs(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'mascot.glb'
            make_glb(path, True)
            document, accessor = decode(path)
            attributes = document['meshes'][0]['primitives'][0]['attributes']
            joints = accessor(attributes['JOINTS_0'])
            weights = accessor(attributes['WEIGHTS_0'])
            names = [document['nodes'][j]['name'] for j in document['skins'][0]['joints']]
            used = {names[j] for js, ws in zip(joints, weights) for j, w in zip(js, ws) if w > 0}
            self.assertTrue({'HEAD','HIPS','SPINE3','RIGHTFOREARM','LEFTFOREARM','RIGHTLEG','LEFTLEG','RIGHTFOOT','LEFTFOOT'} <= used)
            self.assertGreater(sum(sum(w > 0 for w in ws) > 1 for ws in weights), 100,
                               'connecting segments must follow both joint endpoints')
            self.assertTrue(all(abs(sum(w)-1) < 1e-6 for w in weights))
            self.assertGreater(len(accessor(attributes['POSITION'])), 1000)

    def test_hat_has_brim_crown_and_band_along_authored_z_axis(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'hat.glb'
            make_glb(path, False)
            document, accessor = decode(path)
            attributes = document['meshes'][0]['primitives'][0]['attributes']
            positions = accessor(attributes['POSITION'])
            low, high = min(p[2] for p in positions), max(p[2] for p in positions)
            self.assertGreater(high-low, .20)
            self.assertGreater(max(abs(p[0]) for p in positions), .25)
            self.assertGreaterEqual(len(set(accessor(attributes['COLOR_0']))), 3)

    def test_head_visor_is_horizontal_with_live_local_x_up(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'mascot.glb'
            make_glb(path, True)
            document, accessor = decode(path)
            attributes = document['meshes'][0]['primitives'][0]['attributes']
            names = [document['nodes'][j]['name'] for j in document['skins'][0]['joints']]
            head = names.index('HEAD')
            visor = [point for point, color, joint in zip(accessor(attributes['POSITION']),
                      accessor(attributes['COLOR_0']), accessor(attributes['JOINTS_0']))
                      if joint[0] == head and color[0] < .05]
            self.assertGreater(len(visor), 100)
            self.assertLess(max(p[0] for p in visor)-min(p[0] for p in visor), .2)
            self.assertGreater(max(p[2] for p in visor)-min(p[2] for p in visor), .4)


if __name__ == '__main__':
    unittest.main()
