"""Pin and validate the complete Boardwalk Borough resource closure."""
import hashlib
import json
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[1]
RESOURCES=ROOT/'resources'
RECIPE=RESOURCES/'packs'/'boardwalk-borough.recipe.json'

class BoardwalkPackTests(unittest.TestCase):
    def test_boardwalk_recipe_pins_every_manifest_file_and_uses_one_required_world(self):
        recipe=json.loads(RECIPE.read_text())
        self.assertTrue(recipe['accounts_required'])
        pinned={item['id']:item for item in recipe['resources']}
        expected={'boardwalk-borough','platform-profiles','rp-economy','voice-room','phone-calls',
            'interaction-policy','inventory-ui','admin-dashboard','phone','rp-properties','rp-pizza'}
        self.assertEqual(set(pinned),expected)
        self.assertEqual(len(recipe['server']['ensure']),len(set(recipe['server']['ensure'])))
        self.assertEqual(set(recipe['server']['ensure']),expected)
        required_worlds=[]
        for rid,item in pinned.items():
            manifest=json.loads((RESOURCES/rid/'resource.json').read_text())
            if manifest.get('world',{}).get('required'):
                required_worlds.append(rid)
            required=set(manifest.get('files',[])+manifest.get('server_scripts',[])+manifest.get('client_scripts',[])+manifest.get('shared_scripts',[]))
            self.assertTrue(item['provenance'].strip(),rid)
            specs={f['path']:f for f in item['files']}
            self.assertEqual(set(specs),required|{'resource.json'},f'{rid} file pin closure')
            for path,spec in specs.items():
                source=RESOURCES/rid/path
                data=source.read_bytes()
                self.assertEqual(spec['source'],f'{rid}/{path}')
                self.assertEqual(spec['bytes'],len(data),f'{rid}/{path} byte count')
                self.assertEqual(spec['sha256'],hashlib.sha256(data).hexdigest(),f'{rid}/{path} digest')
            for dependency in manifest.get('dependencies',{}):
                self.assertIn(dependency,pinned,f'{rid} dependency {dependency} is absent')
        self.assertEqual(required_worlds,['boardwalk-borough'])
        self.assertEqual(recipe['server'].get('world_rotation'),['boardwalk-borough'])

if __name__=='__main__':unittest.main()
