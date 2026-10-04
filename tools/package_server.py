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
            source_modified: bool = False) -> Path:
    require_windows_x64(executable)
    readme = ROOT / 'tools/server-package/README.txt'
    license_file = ROOT / 'LICENSE'
    # Read required inputs before touching an existing deliverable.
    readme_bytes = readme.read_bytes()
    license_bytes = license_file.read_bytes()
    bundled = {'LUA-NOTICES.txt': ROOT / 'tools/server-package/LUA-NOTICES.txt'}
    bundled.update({f'resources/{name}': ROOT / 'resources' / name
                    for name in RESOURCE_FILES})
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
    args = parser.parse_args()
    try:
        archive = package(args.executable, args.output_directory, args.revision,
                          args.source_modified)
    except (OSError, ValueError) as error:
        parser.exit(1, f'Server packaging failed: {error}\n')
    print(f'Server package: {archive}')
    print(f'SHA256: {archive}.sha256')


if __name__ == '__main__':
    main()
