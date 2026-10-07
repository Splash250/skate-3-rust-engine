import json
import re
import unittest
from pathlib import Path

from tools import resource_park


ROOT = Path(__file__).resolve().parents[1]
WORLD = ROOT / "resources" / "boardwalk-borough"
REQUIRED_MARKERS = {
    "plaza_spawn": "spawn",
    "pizza_counter": "interaction",
    "drop_01": "checkpoint",
    "drop_02": "checkpoint",
    "drop_03": "checkpoint",
    "apt_entry": "interaction",
    "apt_exit": "interaction",
}


class BoardwalkBoroughTests(unittest.TestCase):
    def test_boardwalk_marker_manifest_matches_placements(self):
        for name in ("placements.json", "markers.json", "resource.json"):
            self.assertTrue((WORLD / name).is_file(), f"missing {name}")

        placements = json.loads((WORLD / "placements.json").read_text())
        markers = json.loads((WORLD / "markers.json").read_text())
        manifest = json.loads((WORLD / "resource.json").read_text())
        objects = {item["id"]: item for item in placements["objects"]}
        self.assertEqual(len(objects), len(placements["objects"]))
        self.assertTrue(all(re.fullmatch(r"[a-z0-9_-]{1,64}", key) for key in objects))

        marker_map = {item["id"]: item for item in markers}
        self.assertEqual(len(marker_map), len(markers))
        self.assertEqual(set(REQUIRED_MARKERS), set(marker_map) & set(REQUIRED_MARKERS))
        for marker_id, marker_type in REQUIRED_MARKERS.items():
            self.assertEqual(objects[marker_id]["kind"], "marker")
            self.assertEqual(objects[marker_id]["marker_type"], marker_type)
            self.assertEqual(marker_map[marker_id]["position"], objects[marker_id]["position"])
            self.assertTrue(marker_map[marker_id]["label"])

        self.assertEqual(manifest["world"], {
            "map": "park.skate",
            "required": True,
            "lods": [{"map": "park-low.skate", "distance": 200}],
        })
        for name in ("park.skate", "park-low.skate", "placements.json", "placements-low.json", "markers.json"):
            self.assertIn(name, manifest["files"])
        scripts = set(manifest.get("server_scripts", [])) | set(manifest.get("client_scripts", []))
        self.assertFalse(scripts & set(manifest["files"]), "script paths must not be duplicated in files")

    def test_boardwalk_lod_and_base_export_are_valid(self):
        for name in ("placements.json", "placements-low.json", "park.skate", "park-low.skate"):
            self.assertTrue((WORLD / name).is_file(), f"missing {name}")
        base = json.loads((WORLD / "placements.json").read_text())
        low = json.loads((WORLD / "placements-low.json").read_text())
        base_bytes = (WORLD / "park.skate").read_bytes()
        low_bytes = (WORLD / "park-low.skate").read_bytes()

        self.assertTrue(any(item["kind"] != "marker" for item in base["objects"]))
        self.assertTrue(all(item["kind"] != "rail" for item in low["objects"]))
        self.assertEqual(base_bytes, resource_park.encode(base))
        self.assertEqual(low_bytes, resource_park.encode(low, render_only=True))
        self.assertTrue(base_bytes.startswith(b"SKATE08\0"))
        self.assertTrue(low_bytes.startswith(b"SKATE08\0"))
        self.assertGreater(len(base_bytes), len(low_bytes))


if __name__ == "__main__":
    unittest.main()
