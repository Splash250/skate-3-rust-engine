import json
import io
import os
import signal
import sys
import time
from pathlib import Path
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from tools import park_editor as editor
from tools import resource_park as park


class EditorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.model = editor.Editor(self.root)

    def test_edit_undo_redo_save_load_and_export_roundtrip(self):
        original = self.model.scene
        changed = json.loads(json.dumps(original)); changed['objects'][0]['rotation'] = 45
        self.model.edit(changed); self.model.undo(); self.assertEqual(self.model.scene, original)
        self.model.redo(); self.assertEqual(self.model.scene, changed)
        self.model.save('park.json'); self.model.edit(park.make_scene('Other')); self.model.load('park.json')
        self.assertEqual(self.model.scene, changed)
        self.model.export('exported', 'creator-park')
        self.assertEqual((self.root / 'exported/park.skate').read_bytes(), park.encode(changed))
        self.assertEqual(park.load(self.root / 'exported/placements.json'), changed)

    def test_invalid_changes_do_not_replace_scene_and_history_is_bounded(self):
        original = self.model.scene
        bad = json.loads(json.dumps(original)); bad['objects'][0]['position'][0] = float('nan')
        with self.assertRaises(ValueError): self.model.edit(bad)
        self.assertEqual(self.model.scene, original)
        for i in range(90): self.model.edit(park.make_scene(str(i)))
        self.assertLessEqual(len(self.model.past), editor.MAX_HISTORY)
        self.assertLessEqual(sum(map(len, self.model.past)), editor.MAX_HISTORY_BYTES)

    def test_paths_symlinks_and_missing_native_prerequisite_fail(self):
        for path in ('../outside.json', '/tmp/outside.json', 'a/../../x.json'):
            with self.subTest(path=path), self.assertRaises(ValueError): self.model.save(path)
        (self.root / 'escape').symlink_to(self.root.parent, target_is_directory=True)
        with self.assertRaises(ValueError): self.model.save('escape/outside.json')
        with self.assertRaisesRegex(ValueError, 'game.*executable'): self.model.playtest()

    def test_export_cannot_follow_existing_package_symlink(self):
        target = self.root / 'external'; target.write_text('untouched')
        (self.root / 'exported').mkdir()
        (self.root / 'exported/park.skate').symlink_to(target)
        with self.assertRaises(ValueError): self.model.export('exported', 'creator-park')
        self.assertEqual(target.read_text(), 'untouched')

    @unittest.skipUnless(os.name == 'posix', 'POSIX child cleanup regression')
    def test_stop_retires_descendant_after_native_leader_exits(self):
        executable = self.root / 'native-fixture'
        pid_path = self.root / 'descendant.pid'
        executable.write_text('#!' + sys.executable + '\nimport subprocess,sys\nfrom pathlib import Path\nchild=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"])\nPath(' + repr(str(pid_path)) + ').write_text(str(child.pid))\n')
        executable.chmod(0o700)
        model = editor.Editor(self.root, executable, self.root)
        def live(pid):
            try:
                os.kill(pid, 0)
                stat = Path(f'/proc/{pid}/stat')
                return not stat.exists() or stat.read_text().split(') ')[1][0] != 'Z'
            except ProcessLookupError: return False
        pid = None
        try:
            model.playtest()
            deadline = time.monotonic() + 3
            while not pid_path.exists() or model.process.poll() is None:
                self.assertLess(time.monotonic(), deadline)
                time.sleep(.02)
            pid = int(pid_path.read_text()); self.assertTrue(live(pid))
            model.stop_playtest()
            deadline = time.monotonic() + 2
            while live(pid) and time.monotonic() < deadline: time.sleep(.02)
            self.assertFalse(live(pid), 'native descendant remained after Stop')
        finally:
            model.stop_playtest()
            if pid and live(pid): os.kill(pid, signal.SIGKILL)

    def test_playtest_log_rotation_is_bounded(self):
        (self.root / 'game.log').touch()
        editor.Editor.capture_log(io.BytesIO(b'x' * (3 * 1024 * 1024) + b'last-line'), self.root)
        files = [self.root / 'game.log', self.root / 'game.previous.log']
        self.assertLessEqual(sum(path.stat().st_size for path in files), 2 * 1024 * 1024)
        self.assertTrue(files[0].read_bytes().endswith(b'last-line'))

    @unittest.skipUnless(os.name == 'posix', 'POSIX local executable fixture')
    def test_exited_playtest_state_preserves_tail_and_cleans_containment(self):
        executable = self.root / 'native-fixture'
        executable.write_text('#!' + sys.executable + '\nprint("expected-native-exit")\nraise SystemExit(7)\n')
        executable.chmod(0o700)
        model = editor.Editor(self.root, executable, self.root)
        self.addCleanup(model.stop_playtest)
        model.playtest(); temporary = Path(model.play_directory.name)
        deadline = time.monotonic() + 3
        while model.process.poll() is None:
            self.assertLess(time.monotonic(), deadline); time.sleep(.02)
        state = model.state()['playtest']
        self.assertFalse(state['running']); self.assertEqual(state['exit_code'], 7)
        self.assertIn('expected-native-exit', state['diagnostic'])
        self.assertFalse(temporary.exists()); self.assertIsNone(model.guard)

    def test_http_origin_token_and_host_boundaries_and_actual_roundtrip(self):
        server = editor.make_server(self.model, 0)
        self.addCleanup(server.server_close)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        self.addCleanup(server.shutdown)
        base = f'http://127.0.0.1:{server.server_port}'
        def request(path, body=None, token=True, origin=base, host=None):
            headers = {'Origin': origin}
            if token: headers['X-Editor-Token'] = server.token
            if host: headers['Host'] = host
            if body is not None: headers['Content-Type'] = 'application/json'
            return urllib.request.urlopen(urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None, headers=headers), timeout=2)
        for kwargs in ({'token':False}, {'origin':'https://evil.test'}, {'host':'evil.test'}):
            with self.subTest(kwargs=kwargs), self.assertRaises(urllib.error.HTTPError) as caught:
                request('/api/action', {'action':'save','path':'park.json'}, **kwargs)
            self.assertEqual(caught.exception.code, 403)
            caught.exception.close()
        with request('/api/action', {'action':'save','path':'park.json'}) as response:
            self.assertEqual(response.status, 200)
        with request('/api/state') as response:
            self.assertEqual(json.load(response)['scene'], self.model.scene)
        self.assertTrue((self.root / 'park.json').exists())

if __name__ == '__main__': unittest.main()
