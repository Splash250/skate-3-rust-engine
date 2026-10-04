"""Standalone server packaging checks using synthetic executable data only."""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'tools/package_server.py'
PACKAGE = 'skate-server-windows-x64'


def windows_executable(machine=0x8664):
    data = bytearray(512)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3C, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    struct.pack_into('<H', data, 0x84, machine)
    struct.pack_into('<H', data, 0x96, 0x0022)  # Executable, large-address aware.
    struct.pack_into('<H', data, 0x98, 0x020B)  # PE32+.
    return bytes(data)


class ServerPackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='skate server package ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.executable = self.root / 'build/skate-server.exe'
        self.executable.parent.mkdir()
        self.executable.write_bytes(windows_executable())
        self.output = self.root / 'output with spaces'

    def package(self, *extra):
        self.assertTrue(SCRIPT.is_file(), 'standalone server packager must exist')
        return subprocess.run(
            [sys.executable, str(SCRIPT), '--executable', str(self.executable),
             '--output-directory', str(self.output), '--revision', 'a' * 40, *extra],
            capture_output=True, text=True, check=False,
        )

    def test_zip_is_self_contained_and_excludes_adjacent_private_files(self):
        for name in ('steam_api64.dll', 'private.skate', 'skate3rust.exe', 'release.json'):
            (self.executable.parent / name).write_bytes(b'must not ship')
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        archive_path = self.output / f'{PACKAGE}.zip'
        with zipfile.ZipFile(archive_path) as archive:
            self.assertEqual(set(archive.namelist()), {
                f'{PACKAGE}/skate-server.exe', f'{PACKAGE}/README.txt',
                f'{PACKAGE}/LICENSE', f'{PACKAGE}/server-build.json',
                f'{PACKAGE}/LUA-NOTICES.txt',
                *{f'{PACKAGE}/resources/{name}' for name in (
                    'README.md', 'server.json', 'skate-rules/resource.json',
                    'skate-rules/shared.lua', 'landing-challenge/resource.json',
                    'landing-challenge/shared.lua', 'landing-challenge/client.lua',
                    'landing-challenge/server.lua', 'landing-challenge/ui/title.txt',
                )},
            })
            self.assertEqual(archive.read(f'{PACKAGE}/skate-server.exe'), windows_executable())
            readme = archive.read(f'{PACKAGE}/README.txt').decode('utf-8')
            self.assertIn('--test-world', readme)
            self.assertIn('--connect', readme)
            self.assertIn('UDP', readme)
            self.assertIn('--resources', readme)
            self.assertIn('Lua.org', archive.read(f'{PACKAGE}/LUA-NOTICES.txt').decode())
            manifest = json.loads(archive.read(f'{PACKAGE}/resources/landing-challenge/resource.json'))
            self.assertEqual(manifest['server_scripts'], ['server.lua'])
            self.assertEqual(manifest['client_scripts'], ['client.lua'])
            self.assertEqual(archive.read(f'{PACKAGE}/LICENSE'), (ROOT / 'LICENSE').read_bytes())
            metadata = json.loads(archive.read(f'{PACKAGE}/server-build.json'))
            self.assertEqual(metadata['target'], 'windows-x64')
            self.assertEqual(metadata['revision'], 'a' * 40)
            self.assertFalse(metadata['source_modified'])
            self.assertEqual(metadata['executable_sha256'],
                             hashlib.sha256(windows_executable()).hexdigest())
        digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
        self.assertEqual((self.output / f'{PACKAGE}.zip.sha256').read_text('ascii'),
                         f'{digest}  {PACKAGE}.zip\n')

    def test_wrong_target_and_missing_executable_do_not_publish(self):
        dll = bytearray(windows_executable())
        struct.pack_into('<H', dll, 0x96, 0x2022)
        for invalid in (b'\x7fELF' + bytes(508), windows_executable(0x014C), b'MZ',
                        windows_executable()[:150], dll):
            with self.subTest(header=invalid[:4], size=len(invalid)):
                self.executable.write_bytes(invalid)
                result = self.package()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Windows x64 executable', result.stderr)
                self.assertFalse((self.output / f'{PACKAGE}.zip').exists())
        self.executable.unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.output / f'{PACKAGE}.zip').exists())

    def test_invalid_rebuild_preserves_previous_package_and_checksum(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        previous = {path.name: path.read_bytes() for path in self.output.iterdir()}
        self.executable.write_bytes(b'not an executable')
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual({path.name: path.read_bytes() for path in self.output.iterdir()}, previous)

    def test_local_modifications_are_marked_in_build_metadata(self):
        result = self.package('--source-modified')
        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(self.output / f'{PACKAGE}.zip') as archive:
            metadata = json.loads(archive.read(f'{PACKAGE}/server-build.json'))
        self.assertTrue(metadata['source_modified'])


if __name__ == '__main__':
    unittest.main()
