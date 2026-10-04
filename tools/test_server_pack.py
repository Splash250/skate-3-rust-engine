"""Reproducible installation uses disposable sources, stores and validators."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from tools import server_pack as pack
from tools import server_supervisor as supervisor


class PackTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.install = self.base / 'install'
        self.source = self.base / 'source'
        self.source.mkdir()
        self.manifest = {'format': 1, 'api': 1, 'id': 'test', 'version': '1.0.0', 'language': 'lua', 'server_scripts': ['server.lua']}
        self.recipe = self.base / 'recipe.json'
        self.write_recipe('1.0.0', b'return {}')

    def write_recipe(self, version, code):
        self.manifest['version'] = version
        (self.source / 'resource.json').write_text(json.dumps(self.manifest))
        (self.source / 'server.lua').write_bytes(code)
        files = [{'path': name, 'source': 'source/' + name, 'sha256': hashlib.sha256((self.source / name).read_bytes()).hexdigest(), 'bytes': (self.source / name).stat().st_size} for name in ['resource.json', 'server.lua']]
        self.recipe.write_text(json.dumps({'format': 1, 'id': 'example', 'version': version, 'resources': [{'id': 'test', 'version': version, 'provenance': 'Synthetic tests', 'files': files}], 'server': {'ensure': ['test'], 'grants': {}}, 'accounts_required': False}))

    def apply(self, **kwargs):
        return pack.apply(self.recipe, self.install, validator=lambda p: json.loads(p.read_text()), **kwargs)

    def test_cold_warm_upgrade_rollback_preserves_stable_configuration_and_data(self):
        first = self.apply()
        self.assertEqual(self.apply()['version_id'], first['version_id'])
        (self.install / 'data').mkdir()
        (self.install / 'data' / 'private').write_text('keep')
        self.write_recipe('1.1.0', b'return {on_load=function() end}')
        second = self.apply()
        self.assertNotEqual(first['version_id'], second['version_id'])
        pack.rollback(self.install, first['version_id'], validator=lambda p: None)
        self.assertEqual(json.loads((self.install / 'server.json').read_text()), first['config'])
        self.assertEqual((self.install / 'data' / 'private').read_text(), 'keep')
        self.assertEqual(len(pack.history(self.install)), 2)
        self.assertEqual(pack.verify(self.install)['version_id'], first['version_id'])

    def test_digest_corruption_and_validation_failure_leave_active_unchanged(self):
        self.apply()
        previous = (self.install / 'server.json').read_bytes()
        (self.source / 'server.lua').write_text('tampered')
        with self.assertRaises(ValueError): self.apply()
        self.assertEqual((self.install / 'server.json').read_bytes(), previous)
        self.write_recipe('1.1.0', b'return {}')
        with self.assertRaisesRegex(ValueError, 'validator'):
            pack.apply(self.recipe, self.install, validator=lambda p: (_ for _ in ()).throw(ValueError('validator failed')))
        self.assertEqual((self.install / 'server.json').read_bytes(), previous)

    def test_interrupted_publication_recovers_without_losing_previous_version(self):
        original = self.apply()
        self.write_recipe('1.1.0', b'return {}')
        def fail(point):
            if point == 'prepared': raise RuntimeError('interrupted')
        with self.assertRaises(RuntimeError): self.apply(fault=fail)
        self.assertEqual(pack.recover(self.install)['state'], 'rolled_back')
        self.assertEqual(pack.verify(self.install)['version_id'], original['version_id'])
        def fail_after(point):
            if point == 'published': raise RuntimeError('interrupted')
        with self.assertRaises(RuntimeError): self.apply(fault=fail_after)
        self.assertEqual(pack.recover(self.install)['state'], 'committed')
        self.assertNotEqual(pack.verify(self.install)['version_id'], original['version_id'])

    def test_rejects_paths_symlinks_unpinned_native_files_and_unknown_instructions(self):
        recipe = json.loads(self.recipe.read_text())
        for name in ['../escape', '/escape', 'nul.txt', 'A.lua', 'foo\\bar', '.secret', 'worker.exe', 'library.so']:
            bad = json.loads(json.dumps(recipe)); bad['resources'][0]['files'][0]['path'] = name
            self.recipe.write_text(json.dumps(bad))
            with self.subTest(name=name), self.assertRaises(ValueError): pack.plan(self.recipe)
        self.recipe.write_text(json.dumps(recipe))
        (self.source / 'server.lua').unlink()
        (self.source / 'server.lua').symlink_to(self.source / 'resource.json')
        with self.assertRaises(ValueError): self.apply()
        recipe['shell'] = 'anything'
        self.recipe.write_text(json.dumps(recipe))
        with self.assertRaises(ValueError): pack.plan(self.recipe)

    def test_corrupt_retained_version_and_busy_lock_reject_rollback_or_apply(self):
        first = self.apply()
        with pack.FileLock(self.install / '.pack' / 'run.lock'):
            with self.assertRaisesRegex(ValueError, 'running|busy'): self.apply()
        self.write_recipe('1.1.0', b'return {}')
        self.apply()
        target = self.install / first['config']['root'] / 'test' / 'server.lua'
        target.write_text('corrupted')
        with self.assertRaises(ValueError): pack.rollback(self.install, first['version_id'], validator=lambda p: None)

    def test_missing_validator_and_source_prerequisites_are_actionable(self):
        with self.assertRaisesRegex(ValueError, 'server.*executable|validator'):
            pack.apply(self.recipe, self.install)
        (self.source / 'server.lua').unlink()
        with self.assertRaisesRegex(ValueError, 'source|missing'): self.apply()

    def test_unmanaged_configuration_and_tampered_receipts_are_preserved(self):
        self.install.mkdir()
        (self.install / 'server.json').write_text('{"unrelated":"keep"}')
        with self.assertRaises(ValueError): self.apply()
        self.assertEqual(json.loads((self.install / 'server.json').read_text()), {"unrelated":"keep"})
        (self.install / 'server.json').unlink()
        first = self.apply()
        receipt_path = self.install / '.pack' / 'versions' / first['version_id'] / 'receipt.json'
        receipt = json.loads(receipt_path.read_text()); receipt['recipe']['version'] = '9.9.9'
        receipt_path.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, 'digest'): pack.verify(self.install)

    def test_manifest_version_and_dependencies_must_match_pins(self):
        recipe = json.loads(self.recipe.read_text()); recipe['resources'][0]['version'] = '8.0.0'
        self.recipe.write_text(json.dumps(recipe))
        with self.assertRaisesRegex(ValueError, 'version'): self.apply()

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'POSIX special-file check')
    def test_local_fifo_source_is_rejected_without_blocking_installer(self):
        (self.source / 'server.lua').unlink()
        os.mkfifo(self.source / 'server.lua')
        script = "from tools import server_pack as p; import sys; p.apply(sys.argv[1],sys.argv[2],validator=lambda path:None)"
        process = subprocess.Popen([sys.executable, '-c', script, str(self.recipe), str(self.install)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            try: _, stderr = process.communicate(timeout=2)
            except subprocess.TimeoutExpired: self.fail('nonregular recipe source blocked installation')
            self.assertNotEqual(process.returncode, 0)
            self.assertIn(b'regular', stderr)
            self.assertFalse((self.install / 'server.json').exists())
        finally:
            if process.poll() is None: process.kill()
            process.communicate()

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'POSIX special-file substitution check')
    def test_local_source_replaced_by_fifo_between_check_and_open_is_rejected(self):
        script = '''from tools import server_pack as p
from pathlib import Path
import os,sys
original=os.open
source=Path(sys.argv[1])
def substitute(path,flags,*args,**kwargs):
    if Path(path)==source:
        source.unlink(); os.mkfifo(source)
    return original(path,flags,*args,**kwargs)
p.os.open=substitute
p.copy_file({'source':'server.lua','path':'server.lua','bytes':9,'sha256':'0'*64},source.parent,source.parent/'result')
'''
        process = subprocess.Popen([sys.executable, '-c', script, str(self.source / 'server.lua')], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            try: _, stderr = process.communicate(timeout=2)
            except subprocess.TimeoutExpired: self.fail('FIFO substitution blocked source open')
            self.assertNotEqual(process.returncode, 0)
            self.assertIn(b'regular', stderr)
        finally:
            if process.poll() is None: process.kill()
            process.communicate()

    def test_settings_and_world_rotation_cannot_reference_unpinned_resources(self):
        original = json.loads(self.recipe.read_text())
        for key, value in [('settings', {'external': {'duration': 10}}), ('world_rotation', ['external'])]:
            recipe = json.loads(json.dumps(original)); recipe['server'][key] = value
            self.recipe.write_text(json.dumps(recipe))
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'pinned'):
                pack.plan(self.recipe)

    def test_account_bootstrap_is_included_in_default_stopped_store_snapshot(self):
        # The actual account initializer creates a private new directory, TLS
        # files and SQLite database; model only that subprocess boundary here.
        executable = self.base / 'trusted-account-tool'; executable.touch()
        def initialize(arguments, **kwargs):
            directory = Path(arguments[2]); directory.mkdir()
            (directory / 'certificate.pem').write_text('synthetic certificate')
            (directory / 'private-key.pem').write_text('synthetic key')
            with sqlite3.connect(directory / 'accounts.sqlite3') as db:
                db.execute('CREATE TABLE fixture(account TEXT)')
                db.execute("INSERT INTO fixture VALUES('persistent-identity')")
        with mock.patch.object(pack.subprocess, 'run', side_effect=initialize):
            pack.init_accounts(self.install, executable, 'administrator')
        config = json.loads((self.install / 'accounts.json').read_text())
        database = self.install / config['database']
        self.assertTrue(database.is_relative_to(self.install / 'data'), 'default backup excludes initialized account database')
        supervisor.snapshot(self.install / 'data', self.base / 'backups', 'initialized')
        saved = self.base / 'backups' / 'initialized' / 'data' / database.relative_to(self.install / 'data')
        with sqlite3.connect(saved) as db:
            self.assertEqual(db.execute('SELECT account FROM fixture').fetchone()[0], 'persistent-identity')

    def test_manifest_cannot_load_unpinned_script_after_installation(self):
        self.manifest['shared_scripts'] = ['unbundled.lua']
        self.write_recipe('1.0.0', b'return {}')
        with self.assertRaisesRegex(ValueError, 'unpinned|missing'): self.apply()
        self.assertFalse((self.install / 'server.json').exists())

if __name__ == '__main__': unittest.main()
