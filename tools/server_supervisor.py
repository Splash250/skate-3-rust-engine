#!/usr/bin/env python3
"""Local, bounded dedicated-server supervision and stopped-store recovery.

No shell commands, downloads, credentials or recipe execution. Configuration and
control directories are operator-owned. Keep them outside source control.
"""
from __future__ import annotations
import argparse
import contextlib
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import threading
import time
import uuid

MAX_FILES = 4096
MAX_BYTES = 2 * 1024**3


def atomic_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temp = path.with_name(path.name + '.tmp')
    with open(temp, 'w', encoding='utf-8') as stream:
        if os.name != 'nt': os.fchmod(stream.fileno(), 0o600)
        json.dump(value, stream, separators=(',', ':'))
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)
    if os.name != 'nt':
        descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try: os.fsync(descriptor)
        finally: os.close(descriptor)


def read_json(path: Path, limit=65536):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError(f'Invalid bounded JSON file: {path.name}')
    return json.loads(path.read_text(encoding='utf-8'))


class FileLock:
    """The same lifetime lock is used by pack activation/rollback."""
    def __init__(self, path: Path): self.path, self.file = Path(path), None
    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if self.path.is_symlink(): raise ValueError('Lock cannot be a symlink')
        self.file = open(self.path, 'a+b')
        try:
            if os.name == 'nt':
                import msvcrt
                self.file.seek(0); self.file.write(b'\0'); self.file.flush(); self.file.seek(0)
                msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                os.fchmod(self.file.fileno(), 0o600)
        except OSError:
            self.file.close(); self.file = None
            raise RuntimeError(f'Another supervisor or installer holds {self.path.name}') from None
        return self
    def __exit__(self, *_):
        if self.file is not None: self.file.close(); self.file = None


def identifier(value):
    if not isinstance(value, str) or not 1 <= len(value) <= 64 or any(not (c.isascii() and (c.isalnum() or c in '_-')) for c in value):
        raise ValueError('Snapshot ID requires 1..64 ASCII letters, digits, underscore or hyphen')
    return value


def inventory(root: Path):
    """Reject links/devices and bound total work before any publication."""
    files, total = [], 0
    if root.is_symlink() or not root.is_dir(): raise ValueError('Data root must be an existing directory, not a link')
    for directory, dirs, names in os.walk(root, followlinks=False):
        for name in dirs:
            if Path(directory, name).is_symlink(): raise ValueError('Symlink directory in data store')
        for name in names:
            path = Path(directory, name)
            mode = path.lstat().st_mode
            if not stat.S_ISREG(mode): raise ValueError('Non-regular file in data store')
            total += path.stat().st_size
            if total > MAX_BYTES or len(files) >= MAX_FILES: raise ValueError('Store exceeds backup bound (4096 files / 2 GiB)')
            files.append(path)
    return sorted(files)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''): value.update(chunk)
    return value.hexdigest()


def validate_sqlite(root: Path):
    for path in inventory(root):
        if path.suffix != '.sqlite3': continue
        # Normal read/write open intentionally performs rollback-journal recovery
        # on this stopped, private staged copy, before the digest is committed.
        with sqlite3.connect(path, timeout=1) as connection:
            connection.execute('PRAGMA trusted_schema=OFF')
            if connection.execute('PRAGMA integrity_check').fetchall() != [('ok',)]: raise ValueError(f'SQLite integrity failed: {path.name}')
            if connection.execute('PRAGMA foreign_key_check').fetchone() is not None: raise ValueError(f'SQLite foreign keys failed: {path.name}')
            tables={row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            if {'metadata','accounts','roles','account_roles','sessions','audit'}.issubset(tables):
                schema = connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()
                if schema != ('1',): raise ValueError('Unsupported account schema in snapshot')


def copy_store(source: Path, target: Path):
    files = inventory(source)
    target.mkdir(mode=0o700)
    for path in files:
        relative = path.relative_to(source)
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        # Open the checked source without following a replaced final symlink.
        fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
        with os.fdopen(fd, 'rb') as incoming, destination.open('xb') as outgoing:
            if not stat.S_ISREG(os.fstat(incoming.fileno()).st_mode): raise ValueError('Store changed while copying')
            if os.name != 'nt': os.fchmod(outgoing.fileno(), 0o600)
            shutil.copyfileobj(incoming, outgoing, 1024 * 1024)
            outgoing.flush(); os.fsync(outgoing.fileno())
    inventory(target)


def snapshot(data: Path, backups: Path, name: str):
    name = identifier(name)
    backups.mkdir(parents=True, exist_ok=True, mode=0o700)
    destination = backups / name
    if destination.exists(): raise ValueError('Snapshot ID already exists')
    stage = backups / ('.staging-' + uuid.uuid4().hex)
    stage.mkdir(mode=0o700)
    try:
        copy_store(data, stage / 'data')
        validate_sqlite(stage / 'data')
        records = [{'path': str(p.relative_to(stage / 'data')).replace(os.sep, '/'), 'bytes': p.stat().st_size, 'sha256': digest(p)} for p in inventory(stage / 'data')]
        atomic_json(stage / 'manifest.json', {'format': 1, 'contract': 'stopped-store-copy', 'created': int(time.time()), 'files': records})
        os.rename(stage, destination)
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise
    return {'snapshot': name, 'files': len(records), 'bytes': sum(row['bytes'] for row in records)}


def validate_snapshot(backups: Path, name: str):
    root = backups / identifier(name)
    if root.is_symlink(): raise ValueError('Snapshot cannot be a link')
    manifest = read_json(root / 'manifest.json', 2 * 1024 * 1024)
    if manifest.get('format') != 1 or manifest.get('contract') != 'stopped-store-copy': raise ValueError('Unsupported snapshot format')
    files = manifest.get('files')
    if not isinstance(files, list) or len(files) > MAX_FILES: raise ValueError('Invalid snapshot file count')
    actual = {str(p.relative_to(root / 'data')).replace(os.sep, '/'): p for p in inventory(root / 'data')}
    expected = {}
    for record in files:
        name = record.get('path', '')
        if not isinstance(name, str) or '\\' in name or Path(name).is_absolute() or '..' in Path(name).parts or name in expected: raise ValueError('Invalid snapshot path')
        if name not in actual: raise ValueError('Snapshot file missing')
        path = actual[name]
        if path.stat().st_size != record.get('bytes') or digest(path) != record.get('sha256'): raise ValueError('Snapshot digest mismatch')
        expected[name] = record
    if set(actual) != set(expected): raise ValueError('Unexpected snapshot file')
    return root / 'data'


def recover_restore(data: Path, control: Path):
    journal = control / 'restore.json'
    if not journal.exists(): return
    value = read_json(journal)
    # Paths are derived from the configured data root, never trusted from JSON.
    nonce = value.get('nonce', '')
    if len(nonce) != 32 or any(c not in '0123456789abcdef' for c in nonce): raise ValueError('Invalid restore journal; preserve all directories for operator recovery')
    previous = data.with_name(data.name + '.previous-' + nonce)
    stage = data.with_name(data.name + '.restore-' + nonce)
    if not data.exists() and previous.is_dir(): os.rename(previous, data)
    if not data.exists(): raise RuntimeError('Restore recovery has no current or previous store; preserve journal')
    if stage.exists(): shutil.rmtree(stage)
    journal.unlink()


def restore(data: Path, backups: Path, name: str, control: Path):
    source = validate_snapshot(backups, name)
    nonce = uuid.uuid4().hex
    stage = data.with_name(data.name + '.restore-' + nonce)
    previous = data.with_name(data.name + '.previous-' + nonce)
    try:
        copy_store(source, stage)
        validate_sqlite(stage)
        atomic_json(control / 'restore.json', {'nonce': nonce, 'snapshot': name})
        os.rename(data, previous)
        try: os.rename(stage, data)
        except BaseException:
            os.rename(previous, data)
            raise
        (control / 'restore.json').unlink()
    except BaseException:
        if stage.exists(): shutil.rmtree(stage)
        raise
    return {'restored': name, 'recovery_directory': str(previous), 'note': 'Previous store retained; restore does not reverse application schema migrations'}


def gated_launch(arguments):
    # No server code runs before the parent has installed its process group
    # guard / Windows Job. Parent death closes stdin and fails this gate closed.
    if not arguments or os.read(0, 1) != b'\x01': return 126
    environment = dict(os.environ, SKATE_SUPERVISOR_OWNER_PID=str(os.getpid()))
    if os.name == 'nt':
        # Windows has no POSIX exec replacement. The launcher remains the owned
        # process, and its server inherits the already-established Job and pipes.
        return subprocess.call(arguments, env=environment)
    os.execve(arguments[0], arguments, environment)


def guard_group(fd, group):
    # The guard has no network or control interface. EOF means its owning
    # supervisor died or deliberately released this one process group.
    try:
        with os.fdopen(fd, 'rb', buffering=0) as pipe:
            while pipe.read(1): pass
    finally:
        try: os.killpg(group, signal.SIGKILL)
        except ProcessLookupError: pass


class GroupGuard:
    def __init__(self, pid):
        self.write = self.process = None
        if os.name == 'nt': return
        read, self.write = os.pipe()
        try:
            self.process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--guard', str(read), str(pid)],
                pass_fds=(read,), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        finally: os.close(read)
    def close(self):
        if self.write is not None:
            os.close(self.write); self.write = None
        if self.process is not None:
            self.process.wait(timeout=10); self.process = None


class WindowsJob:
    """Kill the whole trusted server tree when the supervisor exits on Windows."""
    def __init__(self, child):
        self.handle = None
        if os.name != 'nt': return
        from ctypes import wintypes as w
        class Basic(ctypes.Structure):
            _fields_ = [('ProcessTime', ctypes.c_longlong), ('JobTime', ctypes.c_longlong),
                ('LimitFlags', w.DWORD), ('MinimumWorkingSet', ctypes.c_size_t), ('MaximumWorkingSet', ctypes.c_size_t),
                ('ActiveProcessLimit', w.DWORD), ('Affinity', ctypes.c_size_t), ('PriorityClass', w.DWORD), ('SchedulingClass', w.DWORD)]
        class Io(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in ('ReadOperations', 'WriteOperations', 'OtherOperations', 'ReadBytes', 'WriteBytes', 'OtherBytes')]
        class Extended(ctypes.Structure):
            _fields_ = [('Basic', Basic), ('Io', Io), ('ProcessMemory', ctypes.c_size_t), ('JobMemory', ctypes.c_size_t), ('PeakProcessMemory', ctypes.c_size_t), ('PeakJobMemory', ctypes.c_size_t)]
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateJobObjectW.restype = w.HANDLE
        kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, w.LPCWSTR]
        kernel.SetInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD]
        kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        handle = kernel.CreateJobObjectW(None, None)
        limits = Extended(); limits.Basic.LimitFlags = 0x2000  # KILL_ON_JOB_CLOSE
        if not handle or not kernel.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)) or not kernel.AssignProcessToJobObject(handle, w.HANDLE(int(child._handle))):
            if handle: kernel.CloseHandle(handle)
            child.kill(); child.wait()
            raise RuntimeError('Cannot establish required Windows process Job; refusing unsupervised execution')
        self.handle, self.kernel = handle, kernel
    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle); self.handle = None


def linux_parent_death(parent):
    # Called in the newly forked single-threaded child before exec. Existing
    # managed workers additionally use bubblewrap --die-with-parent.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0: os._exit(126)
    if os.getppid() != parent: os._exit(126)


class Supervisor:
    def __init__(self, config_path: Path):
        self.path = config_path.resolve()
        self.config = read_json(self.path)
        known = {'executable', 'arguments', 'working_directory', 'control_root', 'data_root', 'backup_root', 'pack_root', 'startup_seconds', 'hang_seconds', 'grace_seconds', 'max_restarts', 'restart_window_seconds', 'backoff_seconds', 'max_backoff_seconds'}
        if set(self.config) - known: raise ValueError('Unknown supervisor configuration fields')
        def resolve(key, default):
            value = Path(self.config.get(key, default))
            return (self.path.parent / value).resolve()
        self.executable = resolve('executable', '')
        if not self.executable.is_file(): raise ValueError('Configure an existing trusted local server executable')
        self.arguments = self.config.get('arguments', [])
        if not isinstance(self.arguments, list) or len(self.arguments) > 64 or any(not isinstance(a, str) or len(a) > 4096 or '\0' in a for a in self.arguments): raise ValueError('arguments must be a bounded array of strings')
        self.cwd = resolve('working_directory', '.')
        self.control = resolve('control_root', '.supervisor')
        self.data = resolve('data_root', 'data')
        self.backups = resolve('backup_root', '.backups')
        self.pack = resolve('pack_root', self.config['pack_root']) if 'pack_root' in self.config else None
        if self.data == self.data.parent or self.data in self.control.parents or self.data == self.control or self.data in self.backups.parents or self.data == self.backups or self.control == self.backups:
            raise ValueError('Data, control and backup roots must be separate private locations')
        self.control.mkdir(parents=True, exist_ok=True, mode=0o700)
        if os.name != 'nt': os.chmod(self.control, 0o700)
        self.data.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.startup = self.number('startup_seconds', 30, .2, 300)
        self.hang = self.number('hang_seconds', 15, .2, 300)
        self.grace = self.number('grace_seconds', 10, .2, 120)
        self.maximum = int(self.number('max_restarts', 3, 0, 20))
        self.window = self.number('restart_window_seconds', 300, 1, 86400)
        self.backoff = self.number('backoff_seconds', 1, .01, 60)
        self.max_backoff = self.number('max_backoff_seconds', 30, .01, 300)
        self.child = None
        self.job = None
        self.guard = None
        self.stopping = False
        self.history = []
        self.failures = []
        log = self.control / 'server.log'
        self.log_bytes = log.stat().st_size if log.is_file() else 0
        self.log_lock = threading.Lock()
    def number(self, name, default, low, high):
        value = self.config.get(name, default)
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not low <= value <= high: raise ValueError(f'{name} must be between {low} and {high}')
        return value
    def event(self, event, **details):
        row = {'time': int(time.time()), 'event': event, **details}
        self.history.append(row); self.history = self.history[-64:]
        snapshots = sorted(p.name for p in self.backups.iterdir() if p.is_dir() and not p.is_symlink()
            and 1 <= len(p.name) <= 64 and all(c.isascii() and (c.isalnum() or c in '_-') for c in p.name))[:128] if self.backups.is_dir() else []
        status = {'attached': True, 'state': event, 'pid': self.child.pid if self.child else None, 'history': self.history, 'snapshots': snapshots}
        # Count and byte bounds both matter: escaped Unicode diagnostics can
        # otherwise exceed the next launch/admin reader's 64 KiB contract.
        omitted = 0
        while len(json.dumps(status, separators=(',', ':')).encode()) > 60 * 1024 and len(self.history) > 1:
            self.history.pop(0); omitted += 1
        if omitted: status['history_omitted'] = omitted
        atomic_json(self.control / 'status.json', status)
    def logs(self, pipe):
        # Bytes, not readline: a child cannot force an unbounded allocation.
        with pipe:
            while chunk := pipe.read(4096):
                with self.log_lock:
                    path = self.control / 'server.log'
                    if self.log_bytes + len(chunk) > 1024 * 1024:
                        if path.exists(): os.replace(path, self.control / 'server.previous.log')
                        self.log_bytes = 0
                    with path.open('ab') as stream: stream.write(chunk)
                    self.log_bytes += len(chunk)
    def stop_child(self):
        child = self.child
        if child is None: return
        if child.poll() is None:
            try: child.stdin.write(b'supervisor-stop\n'); child.stdin.flush()
            except (BrokenPipeError, OSError): pass
            try: child.wait(timeout=self.grace)
            except subprocess.TimeoutExpired: pass
        # Always clean the process group even if its leader crashed first.
        if os.name != 'nt':
            try: os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError: pass
            if child.poll() is None:
                try: child.wait(timeout=min(1, self.grace))
                except subprocess.TimeoutExpired: pass
            try: os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError: pass
        elif child.poll() is None:
            subprocess.run(['taskkill', '/PID', str(child.pid), '/T', '/F'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10, check=False)
        if self.job is not None: self.job.close(); self.job = None
        if self.guard is not None: self.guard.close(); self.guard = None
        child.wait(timeout=10)
        child.stdin.close()
    def run(self):
        with contextlib.ExitStack() as locks:
            held = [locks.enter_context(FileLock(self.control / 'run.lock')),
                    locks.enter_context(FileLock(self.data.with_name('.' + self.data.name + '.supervisor.lock')))]
            if self.pack: held.append(locks.enter_context(FileLock(self.pack / '.pack' / 'run.lock')))
            recover_restore(self.data, self.control)
            old = {}
            if (self.control / 'status.json').exists(): old = read_json(self.control / 'status.json')
            # No PID adoption or killing of guessed/reused PIDs. The owned lock and
            # UDP bind reject duplication; crash recovery only handles owned children.
            if isinstance(old.get('history'), list): self.history = old['history'][-63:]
            try:
                while not self.stopping:
                    for name in ('heartbeat.json', 'request.json'):
                        (self.control / name).unlink(missing_ok=True)
                    environment = dict(os.environ, SKATE_SUPERVISOR_CONTROL=str(self.control))
                    parent_pid = os.getpid()
                    self.child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--launch", str(self.executable), *self.arguments], cwd=self.cwd, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=os.name != 'nt',
                        pass_fds=tuple(lock.file.fileno() for lock in held) if os.name != 'nt' else (),
                        preexec_fn=(lambda: linux_parent_death(parent_pid)) if sys.platform == 'linux' else None)
                    self.job = WindowsJob(self.child)
                    self.guard = GroupGuard(self.child.pid)
                    self.child.stdin.write(b'\x01'); self.child.stdin.flush()
                    reader = threading.Thread(target=self.logs, args=(self.child.stdout,), daemon=True); reader.start()
                    self.event('running')
                    started = last = time.monotonic(); observed = False; beat = None; cause = None
                    while self.child.poll() is None and not self.stopping:
                        path = self.control / 'heartbeat.json'
                        try:
                            value = read_json(path)
                            if value.get('pid') == self.child.pid and value.get('tick') != beat:
                                last = time.monotonic(); observed = True; beat = value.get('tick')
                        except (OSError, ValueError, json.JSONDecodeError): pass
                        if time.monotonic() - (last if observed else started) > (self.hang if observed else self.startup):
                            cause = 'hang_timeout' if observed else 'startup_timeout'; break
                        time.sleep(.05)
                    self.stop_child(); reader.join(timeout=2)
                    code = self.child.returncode
                    request = None
                    try:
                        candidate = read_json(self.control / 'request.json')
                        if candidate.get('pid') == self.child.pid: request = candidate
                    except (OSError, ValueError, json.JSONDecodeError): pass
                    self.child = None
                    if request and request.get('kind')=='shutdown':
                        if code!=0 or cause is not None:self.event('shutdown_interrupted',exit_code=code,reason=cause or 'process failure during requested shutdown')
                        self.event('operator_stopped');return 0
                    if request and (code != 75 or cause is not None or self.stopping):
                        self.event('operation_interrupted', operation=request.get('kind'), reason=cause or 'server stopped before its requested operation')
                    if self.stopping: self.event('operator_stopped'); return 0
                    if request and code == 75 and cause is None:
                        try:
                            if request.get('kind') == 'restart': result = {'restart': 'operator-requested'}
                            elif request.get('kind') == 'backup': result = snapshot(self.data, self.backups, request.get('snapshot'))
                            elif request.get('kind') == 'restore': result = restore(self.data, self.backups, request.get('snapshot'), self.control)
                            else: raise ValueError('Unknown supervisor request')
                            self.event('operation_completed', result=result)
                        except Exception as error:
                            self.event('operation_failed', error=str(error)[:1024]); return 2
                    if code == 0 and cause is None: self.event('operator_stopped'); return 0
                    if code == 75 and cause is None and request and request.get('kind') in ('restart','backup','restore'):
                        self.event('requested_restart'); continue
                    if code == 75 and cause is None:
                        now = time.monotonic(); self.failures = [at for at in self.failures if now-at < self.window]
                        self.failures.append(now)
                        if len(self.failures)>self.maximum: self.event('recovery_exhausted'); return 2
                        self.event('requested_restart'); continue
                    now = time.monotonic(); self.failures = [at for at in self.failures if now - at < self.window]
                    self.failures.append(now)
                    self.event(cause or 'child_crashed', exit_code=code)
                    if len(self.failures) > self.maximum: self.event('recovery_exhausted'); return 2
                    delay = min(self.max_backoff, self.backoff * 2**(len(self.failures)-1))
                    self.event('backoff', seconds=delay)
                    until = time.monotonic() + delay
                    while time.monotonic() < until and not self.stopping: time.sleep(.05)
            finally:
                self.stop_child()
            return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', type=Path)
    parser.add_argument('--status', action='store_true')
    args = parser.parse_args()
    supervisor = Supervisor(args.config)
    if args.status:
        print(json.dumps(read_json(supervisor.control / 'status.json'), indent=2)); return 0
    def stop(*_): supervisor.stopping = True
    signal.signal(signal.SIGINT, stop); signal.signal(signal.SIGTERM, stop)
    return supervisor.run()

if __name__ == '__main__':
    if len(sys.argv)>=3 and sys.argv[1]=='--launch':sys.exit(gated_launch(sys.argv[2:]))
    if len(sys.argv)==4 and sys.argv[1]=='--guard' and os.name!='nt':
        guard_group(int(sys.argv[2]),int(sys.argv[3]));sys.exit(0)
    try: sys.exit(main())
    except (OSError, ValueError, RuntimeError, sqlite3.Error) as error:
        print(f'supervisor: {error}', file=sys.stderr); sys.exit(2)
