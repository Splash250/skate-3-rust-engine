import json
import io
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
from tools import server_supervisor as supervisor


class Recovery(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.data = self.root / 'data'; self.data.mkdir()
        self.control = self.root / 'control'; self.control.mkdir()
        self.backups = self.root / 'backups'
        with sqlite3.connect(self.data / 'test.sqlite3') as db:
            db.execute('CREATE TABLE profiles(account TEXT PRIMARY KEY, score INTEGER)')
            db.execute("INSERT INTO profiles VALUES('verified-account', 100)")
    def tearDown(self): self.temp.cleanup()
    def score(self):
        with sqlite3.connect(self.data / 'test.sqlite3') as db: return db.execute('SELECT score FROM profiles').fetchone()[0]
    def test_stopped_snapshot_restore_preserves_previous_and_validates_digest(self):
        result = supervisor.snapshot(self.data, self.backups, 'known-good')
        self.assertEqual(result['files'], 1)
        with sqlite3.connect(self.data / 'test.sqlite3') as db: db.execute('UPDATE profiles SET score=900')
        restored = supervisor.restore(self.data, self.backups, 'known-good', self.control)
        self.assertEqual(self.score(), 100)
        self.assertTrue(Path(restored['recovery_directory']).is_dir())
        with sqlite3.connect(Path(restored['recovery_directory']) / 'test.sqlite3') as db:
            self.assertEqual(db.execute('SELECT score FROM profiles').fetchone()[0], 900)
        (self.backups / 'known-good/data/test.sqlite3').write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'digest'): supervisor.restore(self.data, self.backups, 'known-good', self.control)
        self.assertEqual(self.score(), 100)
    def test_interrupted_restore_recovers_previous_directory(self):
        nonce = 'a' * 32
        previous = self.data.with_name('data.previous-' + nonce)
        self.data.rename(previous)
        supervisor.atomic_json(self.control / 'restore.json', {'nonce': nonce})
        supervisor.recover_restore(self.data, self.control)
        self.assertEqual(self.score(), 100)
        self.assertFalse((self.control / 'restore.json').exists())
    def test_failed_second_rename_restores_original(self):
        supervisor.snapshot(self.data, self.backups, 'good')
        real = os.rename
        def rename(source, target):
            if '.restore-' in str(source): raise OSError('injected publish failure')
            return real(source, target)
        with mock.patch.object(supervisor.os, 'rename', side_effect=rename):
            with self.assertRaisesRegex(OSError, 'injected'): supervisor.restore(self.data, self.backups, 'good', self.control)
        self.assertEqual(self.score(), 100)
        supervisor.recover_restore(self.data, self.control)
    def test_failed_restore_copy_removes_partial_staging(self):
        supervisor.snapshot(self.data,self.backups,'good')
        def fail(source,target):
            target.mkdir();(target/'partial').write_bytes(b'partial')
            raise OSError('injected copy failure')
        with mock.patch.object(supervisor,'copy_store',side_effect=fail):
            with self.assertRaisesRegex(OSError,'injected'):supervisor.restore(self.data,self.backups,'good',self.control)
        self.assertFalse(list(self.root.glob('data.restore-*')))
        self.assertEqual(self.score(),100)
    def test_service_owned_accounts_table_is_not_mistaken_for_account_authority(self):
        with sqlite3.connect(self.data/'progression.sqlite3') as db:
            db.execute('CREATE TABLE accounts(player TEXT PRIMARY KEY, experience INTEGER)')
            db.execute("INSERT INTO accounts VALUES('test-profile', 7)")
        supervisor.snapshot(self.data,self.backups,'service-accounts')
        supervisor.restore(self.data,self.backups,'service-accounts',self.control)
        with sqlite3.connect(self.data/'progression.sqlite3') as db:
            self.assertEqual(db.execute('SELECT experience FROM accounts').fetchone()[0],7)
    def test_links_paths_and_duplicate_snapshot_are_rejected(self):
        supervisor.snapshot(self.data, self.backups, 'good')
        with self.assertRaises(ValueError): supervisor.snapshot(self.data, self.backups, 'good')
        with self.assertRaises(ValueError): supervisor.validate_snapshot(self.backups, '../escape')
        if os.name != 'nt':
            (self.data / 'escape').symlink_to(self.root / 'outside')
            with self.assertRaisesRegex(ValueError, 'Non-regular'): supervisor.snapshot(self.data, self.backups, 'unsafe')
    def test_lock_rejects_second_owner(self):
        with supervisor.FileLock(self.control / 'run.lock'):
            with self.assertRaises(RuntimeError):
                with supervisor.FileLock(self.control / 'run.lock'): pass


class Processes(unittest.TestCase):
    def fixture(self, code, **changes):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        script = root / 'child.py'; script.write_text(code)
        config = {'executable': sys.executable, 'arguments': [str(script)], 'working_directory': str(root),
                  'startup_seconds': .3, 'hang_seconds': .3, 'grace_seconds': .2,
                  'max_restarts': 1, 'restart_window_seconds': 10, 'backoff_seconds': .01}
        config.update(changes)
        path = root / 'supervisor.json'; path.write_text(json.dumps(config))
        return root, supervisor.Supervisor(path)
    def test_data_lock_prevents_distinct_control_roots_sharing_stores(self):
        root,host=self.fixture('raise SystemExit(0)\n')
        with supervisor.FileLock(host.data.with_name('.'+host.data.name+'.supervisor.lock')):
            with self.assertRaises(RuntimeError):host.run()
        self.assertIsNone(host.child)
    def test_diagnostic_history_remains_readable_at_its_byte_limit(self):
        root,host=self.fixture('raise SystemExit(0)\n')
        for _ in range(64):host.event('operation_failed',error='failure '*128)
        status=supervisor.read_json(host.control/'status.json')
        self.assertEqual(status['state'],'operation_failed')
        self.assertLessEqual(len(status['history']),64)
    def test_reopened_supervisor_counts_existing_log_bytes(self):
        root,host=self.fixture('raise SystemExit(0)\n')
        (host.control/'server.log').write_bytes(b'x'*(1024*1024-4))
        reopened=supervisor.Supervisor(root/'supervisor.json')
        reopened.logs(io.BytesIO(b'abcdef'))
        self.assertLessEqual((host.control/'server.log').stat().st_size,1024*1024)
        self.assertEqual((host.control/'server.log').read_bytes(),b'abcdef')
        self.assertLessEqual((host.control/'server.previous.log').stat().st_size,1024*1024)
    def test_clean_operator_exit_never_restarts(self):
        root, host = self.fixture('raise SystemExit(0)\n')
        self.assertEqual(host.run(), 0)
        self.assertEqual([v['event'] for v in host.history].count('running'), 1)
        self.assertEqual(host.history[-1]['event'], 'operator_stopped')
    def test_crashes_exhaust_bounded_restart_budget(self):
        root, host = self.fixture('raise SystemExit(2)\n')
        self.assertEqual(host.run(), 2)
        self.assertEqual([v['event'] for v in host.history].count('running'), 2)
        self.assertEqual(host.history[-1]['event'], 'recovery_exhausted')
    def test_shutdown_intent_suppresses_recovery_even_if_drain_hangs(self):
        root,host=self.fixture("import json,os,pathlib,time\np=pathlib.Path(os.environ['SKATE_SUPERVISOR_CONTROL'])/'request.json'\np.write_text(json.dumps({'kind':'shutdown','pid':int(os.environ.get('SKATE_SUPERVISOR_OWNER_PID',os.getpid()))}))\ntime.sleep(100)\n")
        self.assertEqual(host.run(),0)
        self.assertEqual([v['event'] for v in host.history].count('running'),1)
        self.assertIn('shutdown_interrupted',[v['event'] for v in host.history])
        self.assertEqual(host.history[-1]['event'],'operator_stopped')
    def test_requested_restart_cannot_loop_without_bound(self):
        root, host = self.fixture('raise SystemExit(75)\n')
        self.assertEqual(host.run(), 2)
        self.assertEqual([v['event'] for v in host.history].count('running'), 2)
    def test_hung_child_is_killed_and_recovery_is_bounded(self):
        root, host = self.fixture('import time\ntime.sleep(100)\n', max_restarts=0)
        start = time.monotonic()
        self.assertEqual(host.run(), 2)
        self.assertLess(time.monotonic() - start, 5)
        self.assertIn('startup_timeout', [v['event'] for v in host.history])
    @unittest.skipUnless(sys.platform == 'linux', 'Linux startup-containment acceptance')
    def test_supervisor_death_before_guard_cannot_launch_server_descendants(self):
        root,host=self.fixture("import subprocess,sys,pathlib,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(100)'])\npathlib.Path('descendant').write_text(str(p.pid))\ntime.sleep(100)\n",startup_seconds=10)
        code="""from tools import server_supervisor as s
import os,sys,signal,pathlib
def fail(pid):
 pathlib.Path(sys.argv[2]).write_text(str(pid))
 os.kill(os.getpid(),signal.SIGKILL)
s.GroupGuard=fail
s.Supervisor(pathlib.Path(sys.argv[1])).run()
"""
        process=subprocess.Popen([sys.executable,'-c',code,str(host.path),str(root/'launcher')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        process.wait(timeout=5)
        self.assertEqual(process.returncode,-signal.SIGKILL)
        self.assertFalse((root/'descendant').exists(),'server must remain gated until containment exists')
        pid=int((root/'launcher').read_text());path=Path(f'/proc/{pid}/stat');until=time.monotonic()+3
        while time.monotonic()<until and path.exists() and path.read_text().split()[2]!='Z':time.sleep(.01)
        if path.exists():self.assertEqual(path.read_text().split()[2],'Z')
    @unittest.skipUnless(sys.platform == 'linux', 'Linux parent-death/process-group acceptance')
    def test_killed_supervisor_retires_its_server_and_descendants(self):
        root, host = self.fixture("import subprocess,sys,pathlib,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(100)'])\npathlib.Path('descendant').write_text(str(p.pid))\ntime.sleep(100)\n", startup_seconds=10)
        process=subprocess.Popen([sys.executable,str(Path(supervisor.__file__)),str(host.path)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        until=time.monotonic()+5
        while time.monotonic()<until and not (root/'descendant').exists():time.sleep(.01)
        status=supervisor.read_json(host.control/'status.json')
        descendant=int((root/'descendant').read_text())
        process.kill();process.wait(timeout=5)
        for pid in (status['pid'],descendant):
            path=Path(f'/proc/{pid}/stat');until=time.monotonic()+3
            while time.monotonic()<until and path.exists() and path.read_text().split()[2]!='Z':time.sleep(.01)
            if path.exists():self.assertEqual(path.read_text().split()[2],'Z')
    @unittest.skipIf(os.name == 'nt', 'POSIX process-group acceptance')
    def test_child_crash_does_not_leave_descendant_running(self):
        root, host = self.fixture("import subprocess,sys,pathlib\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(100)'])\npathlib.Path('descendant').write_text(str(p.pid))\nraise SystemExit(2)\n", max_restarts=0)
        self.assertEqual(host.run(), 2)
        pid = int((root / 'descendant').read_text())
        stat_path = Path(f'/proc/{pid}/stat')
        if stat_path.exists(): self.assertEqual(stat_path.read_text().split()[2], 'Z')

if __name__ == '__main__': unittest.main()
