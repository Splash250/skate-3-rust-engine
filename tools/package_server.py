"""Package the dedicated Windows server, without game assets or installer tools."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
PACKAGE = 'skate-server-windows-x64'
RESOURCE_FILES = (
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
    'verified-course/README.md',
    'voice-room/resource.json',
    'voice-room/client.lua',
    'voice-room/server.lua',
    'voice-room/README.md',
    'community-park/resource.json',
    'community-park/park.skate',
    'community-park/park-low.skate',
    'community-park/placements.json',
    'community-park/placements-low.json',
    'community-park/markers.json',
    'community-park/client.lua',
    'community-park/server.lua',
    'community-park/README.md',
    'managed-language-demo/resource.json',
    'managed-language-demo/client.cs',
    'managed-language-demo/server.cs',
    'managed-language-demo/README.md',
)
MANAGED_FILES = (
    'Skate.ResourceHost.dll', 'Skate.ResourceHost.deps.json',
    'Skate.ResourceHost.runtimeconfig.json', 'Microsoft.CodeAnalysis.dll',
    'Microsoft.CodeAnalysis.CSharp.dll', 'LICENSE.txt', 'ThirdPartyNotices.txt',
)
DOCUMENTATION_FILES = (
    'README.md', 'CONTEXT.md', 'docs/LINUX.md',
    'docs/multiplayer/README.md',
    'docs/multiplayer/accounts-and-administration.md',
    'docs/multiplayer/backend-services.md', 'docs/multiplayer/browser-interfaces.md',
    'docs/multiplayer/large-messages.md', 'docs/multiplayer/platform-extension.md',
    'docs/multiplayer/platform-extension-evidence.md',
    'docs/multiplayer/resource-compatibility.md', 'docs/multiplayer/resource-validation.md',
    'docs/multiplayer/resource-worlds.md', 'docs/multiplayer/resources.md',
    'docs/multiplayer/shared-entities.md', 'docs/multiplayer/verified-competitions.md',
    'docs/multiplayer/voice.md', 'crates/skate-mods/managed-host/IPC.md',
    'sdk/ANIMATION.md', 'sdk/DEFORMATION.md', 'sdk/ENGINE_API.md',
    'sdk/GENERAL_API.md', 'sdk/RESOURCES.md',
    'sdk/resources.d.ts', 'sdk/resources.lua', 'sdk/skate.lua',
)


def require_windows_x64(executable: Path) -> None:
    """Reject accidental Linux, 32-bit, DLL, or truncated build outputs."""
    with executable.open('rb') as stream:
        header = stream.read(64)
        if len(header) == 64 and header[:2] == b'MZ':
            stream.seek(struct.unpack_from('<I', header, 0x3C)[0])
            pe = stream.read(26)
            if len(pe) == 26 and pe[:4] == b'PE\0\0':
                machine = struct.unpack_from('<H', pe, 4)[0]
                characteristics = struct.unpack_from('<H', pe, 22)[0]
                magic = struct.unpack_from('<H', pe, 24)[0]
                if (machine == 0x8664 and magic == 0x020B
                        and characteristics & 0x0002 and not characteristics & 0x2000):
                    return
    raise ValueError(f'Expected a Windows x64 executable: {executable}')


def package(executable: Path, output_directory: Path, revision: str,
            source_modified: bool = False, account_executable: Path | None = None,
            managed_host: Path | None = None) -> Path:
    require_windows_x64(executable)
    readme = ROOT / 'tools/server-package/README.txt'
    license_file = ROOT / 'LICENSE'
    # Read required inputs before touching an existing deliverable.
    readme_bytes = readme.read_bytes()
    license_bytes = license_file.read_bytes()
    bundled = {name: ROOT / 'tools/server-package' / name
               for name in ('LUA-NOTICES.txt', 'PLATFORM-NOTICES.txt')}
    bundled.update({f'resources/{name}': ROOT / 'resources' / name
                    for name in RESOURCE_FILES})
    bundled.update({name: ROOT / name for name in DOCUMENTATION_FILES})
    if account_executable is not None:
        require_windows_x64(account_executable)
        bundled['skate-account.exe'] = account_executable
    if managed_host is not None:
        bundled.update({f'managed-host/{name}': managed_host / name
                        for name in MANAGED_FILES})
    bundled_bytes = {}
    for name, path in bundled.items():
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'Expected a regular package input: {path}')
        bundled_bytes[name] = path.read_bytes()
    with executable.open('rb') as stream:
        executable_hash = hashlib.file_digest(stream, 'sha256').hexdigest()
    metadata = {
        'schema': 1,
        'product': 'skate-server',
        'target': 'windows-x64',
        'revision': revision,
        'source_modified': source_modified,
        'executable_sha256': executable_hash,
        'account_setup_tool': account_executable is not None,
        'managed_host': managed_host is not None,
    }
    output_directory.mkdir(parents=True, exist_ok=True)
    archive_name = f'{PACKAGE}.zip'
    # Only these explicit inputs are shipped: never recurse over target/assets.
    with tempfile.TemporaryDirectory(prefix='.server-package-', dir=output_directory) as temp:
        staged_archive = Path(temp) / archive_name
        with zipfile.ZipFile(staged_archive, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            archive.write(executable, f'{PACKAGE}/skate-server.exe')
            archive.writestr(f'{PACKAGE}/README.txt', readme_bytes)
            archive.writestr(f'{PACKAGE}/LICENSE', license_bytes)
            for name, contents in bundled_bytes.items():
                archive.writestr(f'{PACKAGE}/{name}', contents)
            archive.writestr(f'{PACKAGE}/server-build.json', json.dumps(metadata, indent=2) + '\n')
        with staged_archive.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        staged_checksum = Path(temp) / f'{archive_name}.sha256'
        staged_checksum.write_text(f'{digest}  {archive_name}\n', encoding='ascii', newline='\n')
        staged_archive.replace(output_directory / archive_name)
        staged_checksum.replace(output_directory / staged_checksum.name)
    return output_directory / archive_name


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target')
    parser.add_argument('--revision', required=True, help='Git revision used for the build')
    parser.add_argument('--source-modified', action='store_true',
                        help='Mark builds from a checkout with local modifications')
    parser.add_argument('--account-executable', type=Path,
                        help='Fresh Windows x64 skate-account setup tool')
    parser.add_argument('--managed-host', type=Path,
                        help='Published trusted managed worker with .NET license notices')
    args = parser.parse_args()
    try:
        archive = package(args.executable, args.output_directory, args.revision,
                          args.source_modified, args.account_executable, args.managed_host)
    except (OSError, ValueError) as error:
        parser.exit(1, f'Server packaging failed: {error}\n')
    print(f'Server package: {archive}')
    print(f'SHA256: {archive}.sha256')


if __name__ == '__main__':
    main()
