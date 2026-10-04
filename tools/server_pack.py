"""Install digest-pinned resource packs without recipe code or archive extraction.

The stable server.json is the publication point. Mutable data is never versioned.
Only an operator-selected local executable may validate an installation.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import time
import urllib.request
import urllib.parse

MAX_RECIPE = 1024 * 1024
MAX_FILE = 64 * 1024 * 1024
MAX_TOTAL = 512 * 1024 * 1024
MAX_FILES = 4096
MAX_VERSIONS = 32
NATIVE_SUFFIXES = {'.exe', '.dll', '.so', '.dylib', '.bat', '.cmd', '.ps1', '.sh', '.zip', '.tar', '.gz', '.sqlite3', '.db', '.pem', '.key'}


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r'[a-z0-9][a-z0-9_-]{0,63}', value):
        raise ValueError('expected a portable lowercase identifier')
    return value


def relative(value):
    if not isinstance(value, str) or len(value) > 240 or not value:
        raise ValueError('invalid relative file path')
    parts = value.split('/')
    for part in parts:
        if not re.fullmatch(r'[a-z0-9_][a-z0-9_.-]*', part) or part.endswith('.') or part in ('.', '..') or part.split('.')[0] in {'con', 'prn', 'aux', 'nul', *(f'com{i}' for i in range(1, 10)), *(f'lpt{i}' for i in range(1, 10))}:
            raise ValueError(f'unsafe portable path: {value}')
    return Path(*parts)


def no_symlinks(path):
    path = Path(path).absolute()
    for part in [path, *path.parents]:
        if part.is_symlink():
            raise ValueError(f'symlink is not allowed: {part}')
    return path


def confined(root, name):
    return no_symlinks(no_symlinks(root) / relative(name))


def open_regular(path):
    path = no_symlinks(path)
    if not stat.S_ISREG(path.lstat().st_mode): raise ValueError('local source must be a regular file')
    # Nonblocking open prevents a FIFO substitution between lstat and open from
    # hanging before fstat. NOFOLLOW also rejects a substituted final symlink.
    descriptor = os.open(path, os.O_RDONLY | getattr(os, 'O_BINARY', 0) | getattr(os, 'O_NONBLOCK', 0) | getattr(os, 'O_NOFOLLOW', 0))
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode): raise ValueError('local source must be a regular file')
        return os.fdopen(descriptor, 'rb')
    except BaseException:
        os.close(descriptor)
        raise


def read_json(path, maximum=MAX_RECIPE):
    with open_regular(path) as file:
        raw = file.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError(f'JSON exceeds {maximum} bytes')
    def duplicate(pairs):
        result = {}
        for key, value in pairs:
            if key in result: raise ValueError(f'duplicate JSON key: {key}')
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=duplicate, parse_constant=lambda value: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def atomic(path, value):
    path = no_symlinks(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.pack-write-', delete=False) as file:
        temporary = Path(file.name)
        try:
            file.write(json.dumps(value, indent=2, sort_keys=True, allow_nan=False).encode() + b'\n')
            file.flush(); os.fsync(file.fileno())
            file.close(); temporary.replace(path)
            if os.name != 'nt':
                fd = os.open(path.parent, os.O_RDONLY)
                try: os.fsync(fd)
                finally: os.close(fd)
        finally:
            temporary.unlink(missing_ok=True)


class FileLock:
    """Same byte-zero lock contract as server_supervisor; held across publication."""
    def __init__(self, path): self.path = path; self.file = None
    def __enter__(self):
        no_symlinks(self.path); self.path.parent.mkdir(parents=True, exist_ok=True)
        self.file = self.path.open('a+b')
        try:
            if os.name == 'nt':
                import msvcrt
                self.file.seek(0); self.file.write(b'0'); self.file.flush(); self.file.seek(0)
                msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            self.file.close(); self.file = None
            raise ValueError('pack is running or another operation is busy; stop its supervisor first') from error
        return self
    def __exit__(self, *args):
        if self.file:
            if os.name == 'nt':
                import msvcrt
                self.file.seek(0); msvcrt.locking(self.file.fileno(), msvcrt.LK_UNLCK, 1)
            self.file.close()


def keys(value, allowed, required=()):
    if not isinstance(value, dict) or set(value) - set(allowed) or set(required) - set(value):
        raise ValueError(f'expected fields {sorted(required)}; allowed fields {sorted(allowed)}')


def plan(recipe_path, source_root=None):
    recipe_path = Path(recipe_path)
    recipe = read_json(recipe_path)
    keys(recipe, {'format', 'id', 'version', 'resources', 'server', 'accounts_required'}, {'format', 'id', 'version', 'resources', 'server'})
    if recipe['format'] != 1: raise ValueError('recipe format must be 1')
    identifier(recipe['id'])
    version = recipe['version']
    if not isinstance(version, str) or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[a-z0-9.-]+)?', version): raise ValueError('recipe version must be an exact version')
    if type(recipe.get('accounts_required', False)) is not bool: raise ValueError('accounts_required must be boolean')
    keys(recipe['server'], {'ensure', 'grants', 'settings', 'content_limits', 'network_budgets', 'runtime_limits', 'http_origins', 'world_rotation'}, {'ensure', 'grants'})
    resources = recipe['resources']
    if not isinstance(resources, list) or not 1 <= len(resources) <= 128: raise ValueError('recipe requires 1..128 resources')
    ids, paths, total = set(), set(), 0
    for resource in resources:
        keys(resource, {'id', 'version', 'provenance', 'files'}, {'id', 'version', 'provenance', 'files'})
        rid = identifier(resource['id'])
        if rid in ids: raise ValueError('duplicate resource')
        ids.add(rid)
        if not isinstance(resource['version'], str) or not resource['version'] or len(resource['version']) > 64: raise ValueError('invalid exact resource version')
        if not isinstance(resource['provenance'], str) or not 1 <= len(resource['provenance']) <= 1024: raise ValueError('resource requires bounded provenance')
        if not isinstance(resource['files'], list): raise ValueError('files must be a list')
        for file in resource['files']:
            keys(file, {'path', 'source', 'sha256', 'bytes'}, {'path', 'source', 'sha256', 'bytes'})
            name = relative(file['path'])
            if name.suffix in NATIVE_SUFFIXES: raise ValueError('native executables, archives, credentials and stores cannot be recipe files')
            key = f'{rid}/{name.as_posix()}'
            if key in paths: raise ValueError('duplicate file path')
            paths.add(key)
            if not isinstance(file['sha256'], str) or not re.fullmatch('[0-9a-f]{64}', file['sha256']): raise ValueError('file requires a full SHA-256 digest')
            if type(file['bytes']) is not int or not 0 <= file['bytes'] <= MAX_FILE: raise ValueError('file size exceeds 64 MiB')
            total += file['bytes']
            source = file['source']
            if not isinstance(source, str): raise ValueError('source must be a string')
            if source.startswith('https://'):
                url = urllib.parse.urlsplit(source)
                if not url.hostname or url.username or url.password or url.fragment or len(source) > 2048: raise ValueError('invalid HTTPS source')
            else: relative(source)
        if f'{rid}/resource.json' not in paths: raise ValueError('each resource needs resource.json')
    if len(paths) > MAX_FILES or total > MAX_TOTAL: raise ValueError('pack exceeds 4096 files or 512 MiB')
    ensure = recipe['server']['ensure']
    if not isinstance(ensure, list) or not ensure or len(ensure) != len(set(ensure)) or any(rid not in ids for rid in ensure): raise ValueError('ensure must select unique pinned resources')
    grants = recipe['server']['grants']
    if not isinstance(grants, dict) or any(rid not in ids or not isinstance(values, list) or any(not isinstance(v, str) for v in values) for rid, values in grants.items()): raise ValueError('grants must refer to pinned resources')
    settings = recipe['server'].get('settings', {})
    if not isinstance(settings, dict) or any(rid not in ids or not isinstance(values, dict) for rid, values in settings.items()): raise ValueError('settings must refer to pinned resources')
    rotation = recipe['server'].get('world_rotation', [])
    if not isinstance(rotation, list) or any(not isinstance(rid, str) or rid not in ids for rid in rotation) or len(rotation) != len(set(rotation)): raise ValueError('world_rotation must select unique pinned resources')
    return {'version_id': hashlib.sha256(canonical(recipe)).hexdigest(), 'recipe': recipe, 'source_root': str(no_symlinks(source_root or recipe_path.parent)), 'files': len(paths), 'bytes': total, 'accounts_required': recipe.get('accounts_required', False), 'contract': 'Stopped code/config publication; mutable stores unchanged. Database migrations are not reversed by rollback.'}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs): raise ValueError('download redirects are not allowed; pin the final HTTPS URL')


def copy_file(spec, root, destination):
    source = spec['source']
    try:
        if source.startswith('https://'):
            # No ambient proxy credentials and no redirects; standard verified TLS.
            stream = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect()).open(source, timeout=15)
        else:
            stream = open_regular(confined(root, source))
    except OSError as error: raise ValueError(f'missing or unavailable source {source}: {error}') from error
    destination.parent.mkdir(parents=True, exist_ok=True)
    digest, written, deadline = hashlib.sha256(), 0, time.monotonic() + 60
    with stream, destination.open('wb') as out:
        while True:
            if time.monotonic() > deadline: raise ValueError('download exceeds 60 second deadline')
            block = getattr(stream, 'read1', stream.read)(min(65536, spec['bytes'] - written + 1))
            if not block: break
            written += len(block)
            if written > spec['bytes']: raise ValueError(f'source length exceeds pin: {spec["path"]}')
            digest.update(block); out.write(block)
        out.flush(); os.fsync(out.fileno())
    if written != spec['bytes'] or digest.hexdigest() != spec['sha256']: raise ValueError(f'source digest/length mismatch: {spec["path"]}')


def check_version(root, receipt):
    if not re.fullmatch('[0-9a-f]{64}', receipt.get('version_id', '')):
        raise ValueError('invalid version receipt identity')
    if hashlib.sha256(canonical(receipt['recipe'])).hexdigest() != receipt['version_id']:
        raise ValueError('receipt recipe digest mismatch')
    expected_config = dict(receipt['recipe']['server'], root=f'.pack/versions/{receipt["version_id"]}/resources', storage='data')
    if receipt.get('config') != expected_config:
        raise ValueError('receipt configuration mismatch')
    version_root = no_symlinks(root / '.pack' / 'versions' / receipt['version_id'])
    manifests = {}
    for resource in receipt['recipe']['resources']:
        folder = version_root / 'resources' / identifier(resource['id'])
        expected = set()
        for file in resource['files']:
            path = confined(folder, file['path']); expected.add(file['path'])
            if not path.is_file() or path.stat().st_size != file['bytes'] or hashlib.sha256(path.read_bytes()).hexdigest() != file['sha256']:
                raise ValueError(f'installed file digest/length mismatch: {path}')
        manifest = read_json(folder / 'resource.json')
        if manifest.get('id') != resource['id'] or manifest.get('version') != resource['version']: raise ValueError('manifest id/version does not match recipe pin')
        declarations = [p for field in ('files', 'client_scripts', 'shared_scripts', 'server_scripts') for p in manifest.get(field, [])]
        if any(p not in expected for p in declarations): raise ValueError('manifest declares an unpinned or missing file')
        actual = {p.relative_to(folder).as_posix() for p in folder.rglob('*') if p.is_file() or p.is_symlink()}
        if actual != expected: raise ValueError('installed resource contains unexpected files')
        manifests[resource['id']] = manifest
    for manifest in manifests.values():
        for dependency, version in manifest.get('dependencies', {}).items():
            if dependency not in manifests or manifests[dependency]['version'] != version: raise ValueError('missing or mismatched exact dependency')


def external_validator(executable):
    if not executable: raise ValueError('provide --server-executable pointing to a trusted built skate-server validator')
    executable = no_symlinks(executable)
    if not executable.is_file(): raise ValueError('server executable is missing; build skate-server first')
    def validate(path):
        with tempfile.TemporaryFile() as output:
            try:
                process = subprocess.run([str(executable), '--validate-resources', str(path)], stdout=output, stderr=subprocess.STDOUT, timeout=30, check=False)
            except (OSError, subprocess.TimeoutExpired) as error: raise ValueError(f'resource validator unavailable: {error}') from error
            if process.returncode:
                output.seek(0); message = output.read(4096).decode(errors='replace')
                raise ValueError(f'resource validator rejected installation: {message}')
    return validate


def history(root):
    root = no_symlinks(root)
    versions = root / '.pack' / 'versions'
    if not versions.exists(): return []
    result = []
    for path in sorted(versions.iterdir()):
        if re.fullmatch('[0-9a-f]{64}', path.name) and (path / 'receipt.json').exists():
            receipt = read_json(path / 'receipt.json')
            result.append(receipt)
    return result


def active_receipt(root):
    config = read_json(root / 'server.json')
    match = re.fullmatch(r'\.pack/versions/([0-9a-f]{64})/resources', config.get('root', ''))
    if not match: raise ValueError('server.json is not a managed pack configuration')
    receipt = read_json(root / '.pack' / 'versions' / match[1] / 'receipt.json')
    if receipt.get('version_id') != match[1] or receipt.get('config') != config: raise ValueError('active configuration differs from its receipt')
    return receipt


def verify(root):
    root = no_symlinks(root); receipt = active_receipt(root); check_version(root, receipt); return receipt


def _recover(root):
    journal = root / '.pack' / 'transaction.json'
    if not journal.exists(): return {'state': 'clean'}
    pending = read_json(journal)
    active = read_json(root / 'server.json') if (root / 'server.json').exists() else None
    if active == pending['next']:
        verify(root); state = 'committed'
    elif active == pending['previous']:
        state = 'rolled_back'
    else: raise ValueError('configuration changed during interrupted publication; inspect transaction.json before recovery')
    journal.unlink(); return {'state': state}


def recover(root):
    root = no_symlinks(root)
    with FileLock(root / '.pack' / 'run.lock'): return _recover(root)


def publish(root, receipt, validator, fault):
    check_version(root, receipt)
    descriptor, name = tempfile.mkstemp(prefix='.pack-validation-', suffix='.json', dir=root)
    os.close(descriptor)
    validation = Path(name)
    try:
        atomic(validation, receipt['config']); validator(validation)
    finally: validation.unlink(missing_ok=True)
    previous = read_json(root / 'server.json') if (root / 'server.json').exists() else None
    atomic(root / '.pack' / 'transaction.json', {'previous': previous, 'next': receipt['config']})
    fault('prepared')
    atomic(root / 'server.json', receipt['config'])
    fault('published')
    _recover(root)


def apply(recipe_path, root, source_root=None, server_executable=None, validator=None, fault=lambda point: None):
    proposal = plan(recipe_path, source_root); validator = validator or external_validator(server_executable)
    root = no_symlinks(root)
    with FileLock(root / '.pack' / 'run.lock'):
        _recover(root)
        if (root / 'server.json').exists(): active_receipt(root)
        versions = no_symlinks(root / '.pack' / 'versions'); versions.mkdir(exist_ok=True)
        version = versions / proposal['version_id']
        config = dict(proposal['recipe']['server'], root=f'.pack/versions/{proposal["version_id"]}/resources', storage='data')
        receipt = {key: value for key, value in proposal.items() if key != 'source_root'}
        receipt['config'] = config
        if not version.exists():
            if len(list(versions.iterdir())) >= MAX_VERSIONS: raise ValueError('32 retained versions reached; archive unused versions while stopped before installing more')
            with tempfile.TemporaryDirectory(prefix='.stage-', dir=root / '.pack') as stage:
                stage = Path(stage)
                for resource in proposal['recipe']['resources']:
                    for file in resource['files']:
                        copy_file(file, proposal['source_root'], stage / 'resources' / resource['id'] / file['path'])
                atomic(stage / 'receipt.json', receipt)
                stage.rename(version)
        else:
            if read_json(version / 'receipt.json') != receipt: raise ValueError('existing version receipt mismatch')
            # Even warm installs verify pinned inputs, catching stale local recipes.
            for resource in proposal['recipe']['resources']:
                for file in resource['files']:
                    if not file['source'].startswith('https://'):
                        source = confined(proposal['source_root'], file['source'])
                        if not source.is_file() or source.stat().st_size != file['bytes'] or hashlib.sha256(source.read_bytes()).hexdigest() != file['sha256']: raise ValueError('local source digest/length mismatch')
        publish(root, receipt, validator, fault)
        return receipt


def rollback(root, version_id, server_executable=None, validator=None):
    if not re.fullmatch('[0-9a-f]{64}', version_id): raise ValueError('rollback needs a full retained version id')
    root = no_symlinks(root); validator = validator or external_validator(server_executable)
    with FileLock(root / '.pack' / 'run.lock'):
        _recover(root)
        receipt = read_json(root / '.pack' / 'versions' / version_id / 'receipt.json')
        if receipt.get('version_id') != version_id: raise ValueError('receipt version mismatch')
        publish(root, receipt, validator, lambda _: None)
        return receipt


def init_accounts(root, executable, username):
    root = no_symlinks(root); executable = no_symlinks(executable)
    if not executable.is_file(): raise ValueError('account executable is missing; build skate-accounts first')
    with FileLock(root / '.pack' / 'run.lock'):
        authority = no_symlinks(root / 'data' / 'accounts')
        if authority.exists() or (root / 'accounts').exists() or (root / 'accounts.json').exists(): raise ValueError('account bootstrap already exists; use authenticated administration')
        authority.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        subprocess.run([str(executable), 'init', str(authority), username], check=True)
        atomic(root / 'accounts.json', {'database': 'data/accounts/accounts.sqlite3', 'bind': '127.0.0.1:31443', 'certificate': 'data/accounts/certificate.pem', 'key': 'data/accounts/private-key.pem'})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('plan', 'apply'):
        cmd = sub.add_parser(name); cmd.add_argument('recipe', type=Path); cmd.add_argument('--source-root', type=Path)
        if name == 'apply': cmd.add_argument('--root', required=True, type=Path); cmd.add_argument('--server-executable', required=True, type=Path)
    for name in ('history', 'verify', 'recover'):
        sub.add_parser(name).add_argument('root', type=Path)
    cmd = sub.add_parser('rollback'); cmd.add_argument('root', type=Path); cmd.add_argument('version_id'); cmd.add_argument('--server-executable', required=True, type=Path)
    cmd = sub.add_parser('init-accounts'); cmd.add_argument('root', type=Path); cmd.add_argument('username'); cmd.add_argument('--account-executable', required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'plan': result = plan(args.recipe, args.source_root)
        elif args.command == 'apply': result = apply(args.recipe, args.root, args.source_root, args.server_executable)
        elif args.command == 'rollback': result = rollback(args.root, args.version_id, args.server_executable)
        elif args.command == 'init-accounts': result = init_accounts(args.root, args.account_executable, args.username)
        else: result = globals()[args.command](args.root)
        print(json.dumps(result, indent=2))
    except (ValueError, OSError, subprocess.SubprocessError) as error: parser.error(str(error))

if __name__ == '__main__': main()
