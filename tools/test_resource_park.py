import json
from pathlib import Path
import tempfile
import unittest
from tools import resource_park as park

class ParkTests(unittest.TestCase):
    def test_save_load_and_export_keep_native_rail_and_marker_separate(self):
        scene=park.make_scene("Original community park")
        scene["objects"].extend([
            {"id":"ramp","kind":"ramp","position":[0,1,4],"size":[4,2,6]},
            {"id":"rail","kind":"rail","position":[5,1,0],"points":[[0,0,-4],[0,0,4]]},
            {"id":"finish","kind":"marker","position":[8,1,0]},
        ])
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);park.save(root/"scene.json",scene)
            self.assertEqual(park.load(root/"scene.json"),scene)
            size=park.export(scene,root/"resource","community-park")
            self.assertGreater(size,2000)
            raw=(root/"resource/park.skate").read_bytes()
            self.assertTrue(raw.startswith(b"SKATE08\0"))
            manifest=json.loads((root/"resource/resource.json").read_text())
            self.assertEqual(manifest["world"],{"map":"park.skate","required":True})
            self.assertEqual(json.loads((root/"resource/markers.json").read_text())[0]["id"],"finish")
            self.assertEqual(raw,park.encode(scene))

    def test_playable_floor_and_ramp_winding_faces_up(self):
        floor=park.make_scene("Floor")["objects"][0]
        triangles=list(park.triangles(floor))
        top=[t for t in triangles if all(p[1]==0 for p in t)]
        self.assertEqual(len(top),2)
        ramp={"kind":"ramp","position":[0,1,0],"size":[4,2,6]}
        for a,b,c in top+list(park.triangles(ramp))[2:4]:
            ab=[b[i]-a[i] for i in range(3)];ac=[c[i]-a[i] for i in range(3)]
            self.assertGreater(ab[2]*ac[0]-ab[0]*ac[2],0)

    def test_authored_far_lod_exports_a_separate_public_scene(self):
        scene=park.make_scene("Base");far=park.make_scene("Far")
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            park.export(scene,root,"community-park",lod_scene=far,lod_distance=200)
            manifest=json.loads((root/"resource.json").read_text())
            self.assertEqual(manifest["world"]["lods"],[{"map":"park-low.skate","distance":200}])
            self.assertIn("park-low.skate",manifest["files"])
            self.assertEqual((root/"park-low.skate").read_bytes(),park.encode(far,render_only=True))

    def test_rotation_and_rail_scale_use_the_same_world_transform(self):
        item = {"kind": "rail", "position": [10, 2, 3], "rotation": 90,
                "size": [2, 1, 3], "points": [[0, 0, 0], [0, 0, 2]]}
        points = park.rail_points(item)
        self.assertEqual(points[0], [10.0, 2.0, 3.0])
        self.assertAlmostEqual(points[1][0], 16)
        self.assertAlmostEqual(points[1][2], 3)
        vertices = [p for tri in park.triangles(item) for p in tri]
        self.assertLess(max(p[2] for p in vertices) - min(p[2] for p in vertices), .2)

    def test_export_preserves_scripts_and_emits_authoritative_marker_metadata(self):
        scene = park.make_scene("Editable")
        scene["objects"].append({"id":"finish", "kind":"marker", "position":[1, 2, 3],
                                 "marker_type":"checkpoint", "order":2, "label":"Finish"})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "resource.json").write_text(json.dumps({"format":1,"api":1,"id":"editable","version":"1.2.3", "language":"lua", "server_scripts":["server.lua"], "files":["extra.txt"], "capabilities":["resource.events"]}))
            (root / "server.lua").write_text('return {}')
            (root / "extra.txt").write_text('public')
            park.export(scene, root, "editable")
            manifest = json.loads((root / "resource.json").read_text())
            self.assertEqual(manifest["server_scripts"], ["server.lua"])
            self.assertEqual(manifest["version"], "1.2.3")
            self.assertIn("extra.txt", manifest["files"])
            marker = json.loads((root / "markers.json").read_text())[0]
            self.assertEqual(marker["type"], "checkpoint")
            self.assertEqual(marker["order"], 2)
            self.assertEqual(marker["label"], "Finish")

    def test_invalid_ids_duplicate_points_nonfinite_and_budgets_fail(self):
        for bad in ["../escape","UpperCase","nul/room"]:
            scene=park.make_scene("Invalid");scene["objects"][0]["id"]=bad
            with self.assertRaises(ValueError):park.validate(scene)
        scene=park.make_scene("Invalid");scene["objects"][0]["position"][0]=float("nan")
        with self.assertRaises(ValueError):park.validate(scene)
        scene=park.make_scene("Invalid");scene["objects"].append(dict(scene["objects"][0]))
        with self.assertRaises(ValueError):park.validate(scene)
        scene=park.make_scene("Invalid");scene["objects"].append({"id":"rail","kind":"rail","position":[0,0,0],"points":[[0,0,0],[0,0,0]]})
        with self.assertRaises(ValueError):park.validate(scene)

if __name__=="__main__":unittest.main()
