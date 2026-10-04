"""Standalone server packaging checks using synthetic executable data only."""
import hashlib
import json
from pathlib import Path
import posixpath
import re
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'tools/package_server.py'
PACKAGE = 'skate-server-windows-x64'
DOCUMENTATION = (
    'README.md', 'CONTEXT.md', 'docs/LINUX.md',
    'docs/multiplayer/README.md',
    'docs/multiplayer/accounts-and-administration.md',
    'docs/multiplayer/backend-services.md', 'docs/multiplayer/browser-interfaces.md',
    'docs/multiplayer/large-messages.md', 'docs/multiplayer/platform-extension.md',
    'docs/multiplayer/platform-extension-evidence.md',
    'docs/multiplayer/production-validation.md',
    'docs/multiplayer/resource-compatibility.md', 'docs/multiplayer/resource-validation.md',
    'docs/multiplayer/resource-worlds.md', 'docs/multiplayer/resources.md',
    'docs/multiplayer/shared-entities.md', 'docs/multiplayer/verified-competitions.md',
    'docs/multiplayer/voice.md', 'crates/skate-mods/managed-host/IPC.md',
    'sdk/ANIMATION.md', 'sdk/DEFORMATION.md', 'sdk/ENGINE_API.md',
    'sdk/GENERAL_API.md', 'sdk/RESOURCES.md',
    'sdk/resources.d.ts', 'sdk/resources.lua', 'sdk/skate.lua',
)


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
                f'{PACKAGE}/LUA-NOTICES.txt', f'{PACKAGE}/PLATFORM-NOTICES.txt',
                *{f'{PACKAGE}/{name}' for name in DOCUMENTATION},
                *{f'{PACKAGE}/resources/{name}' for name in (
                    'README.md', 'server.json', 'skate-rules/resource.json',
                    'skate-rules/shared.lua', 'landing-challenge/resource.json',
                    'landing-challenge/shared.lua', 'landing-challenge/client.lua',
                    'landing-challenge/server.lua', 'landing-challenge/ui/title.txt',
                    'platform-examples.json', 'persistent-progression/resource.json',
                    'persistent-progression/server.lua', 'persistent-progression/README.md',
                    'js-rules/resource.json', 'js-rules/shared.js',
                    'lua-rule-adapter/resource.json', 'lua-rule-adapter/shared.lua',
                    'cross-language-demo/resource.json', 'cross-language-demo/client.js',
                    'cross-language-demo/server.js', 'cross-language-demo/README.md',
                    'shared-objects/resource.json',
                    'shared-objects/server.lua',
                    'shared-objects/README.md',
                    'inventory-ui/resource.json',
                    'inventory-ui/server.lua',
                    'inventory-ui/client.lua',
                    'inventory-ui/index.html',
                    'inventory-ui/inventory.css',
                    'inventory-ui/inventory.js',
                    'inventory-ui/README.md',
                    'presentation-demo/resource.json',
                    'presentation-demo/client.lua',
                    'presentation-demo/server.lua',
                    'presentation-demo/clips.json',
                    'presentation-demo/mascot.glb',
                    'presentation-demo/hat.glb',
                    'presentation-demo/README.md',
                    'verified-course/resource.json', 'verified-course/client.lua', 'verified-course/server.lua',
                    'verified-course/README.md', 'voice-room/README.md',
                    'voice-room/resource.json', 'voice-room/client.lua', 'voice-room/server.lua',
                    'community-park/resource.json', 'community-park/park.skate',
                    'community-park/placements.json', 'community-park/markers.json', 'community-park/README.md',
                    'community-park/park-low.skate', 'community-park/placements-low.json',
                    'community-park/client.lua', 'community-park/server.lua',
                    'managed-language-demo/resource.json',
                    'managed-language-demo/client.cs',
                    'managed-language-demo/server.cs',
                    'managed-language-demo/README.md',
                )},
            })
            self.assertEqual(archive.read(f'{PACKAGE}/skate-server.exe'), windows_executable())
            readme = archive.read(f'{PACKAGE}/README.txt').decode('utf-8')
            self.assertIn('--test-world', readme)
            self.assertIn('--connect', readme)
            self.assertIn('UDP', readme)
            self.assertIn('--resources', readme)
            self.assertIn('--accounts', readme)
            self.assertIn('--account-config', readme)
            self.assertIn('skate-account.exe init', readme)
            self.assertNotIn('No account authentication, encryption, or matchmaking is provided', readme)
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

    def test_bundled_examples_have_local_setup_documentation(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(self.output / f'{PACKAGE}.zip') as archive:
            names = set(archive.namelist())
            for name in names:
                if '/resources/' not in name or not name.endswith('README.md'):
                    continue
                content = re.sub(r'```.*?```|`[^`]*`', '', archive.read(name).decode(), flags=re.S)
                for target in re.findall(r'(?<!!)\[[^\]]*\]\(([^\n)]+)\)', content):
                    target = target.split('#', 1)[0]
                    if not target or '://' in target:
                        continue
                    destination = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
                    self.assertIn(destination, names, f'{name} needs {target}')
            self.assertIn('SKATE_DOTNET_ROOT', archive.read(f'{PACKAGE}/sdk/RESOURCES.md').decode())
            self.assertIn('engine.animation', archive.read(f'{PACKAGE}/sdk/ANIMATION.md').decode())
            self.assertIn('--accounts', archive.read(f'{PACKAGE}/docs/multiplayer/accounts-and-administration.md').decode())

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

    def test_optional_account_tool_and_managed_worker_use_explicit_allowlist(self):
        account = self.root / 'skate-account.exe'
        account.write_bytes(windows_executable())
        worker = self.root / 'managed worker'
        worker.mkdir()
        names = ('Skate.ResourceHost.dll', 'Skate.ResourceHost.deps.json',
                 'Skate.ResourceHost.runtimeconfig.json', 'Microsoft.CodeAnalysis.dll',
                 'Microsoft.CodeAnalysis.CSharp.dll', 'LICENSE.txt', 'ThirdPartyNotices.txt')
        for name in names:
            (worker / name).write_text('synthetic packaging fixture', encoding='utf-8')
        (worker / 'credentials.json').write_text('must not ship', encoding='utf-8')
        result = self.package('--account-executable', str(account), '--managed-host', str(worker))
        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(self.output / f'{PACKAGE}.zip') as archive:
            self.assertEqual(archive.read(f'{PACKAGE}/skate-account.exe'), windows_executable())
            self.assertEqual({name.removeprefix(f'{PACKAGE}/managed-host/') for name in archive.namelist()
                              if name.startswith(f'{PACKAGE}/managed-host/')}, set(names))
            metadata = json.loads(archive.read(f'{PACKAGE}/server-build.json'))
            self.assertTrue(metadata['account_setup_tool'])
            self.assertTrue(metadata['managed_host'])
        previous = (self.output / f'{PACKAGE}.zip').read_bytes()
        (worker / 'LICENSE.txt').unlink()
        result = self.package('--account-executable', str(account), '--managed-host', str(worker))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.output / f'{PACKAGE}.zip').read_bytes(), previous)


if __name__ == '__main__':
    unittest.main()
