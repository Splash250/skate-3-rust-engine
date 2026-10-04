"""Local visual park authoring. Run --help, then open the printed loopback URL."""
from __future__ import annotations
import argparse
import copy
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import secrets
import signal
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.parse
import webbrowser

try:
    from tools import resource_park as park
    from tools.server_pack import no_symlinks, relative
    from tools import server_supervisor as supervision
except ModuleNotFoundError:
    import resource_park as park
    from server_pack import no_symlinks, relative
    import server_supervisor as supervision

MAX_HISTORY = 64
MAX_HISTORY_BYTES = 16 * 1024 * 1024
MAX_SCENE_BYTES = 4 * 1024 * 1024


class Editor:
    def __init__(self, workspace, game=None, assets=None):
        self.workspace = no_symlinks(workspace)
        self.workspace.mkdir(parents=True, exist_ok=True)
        self.game = no_symlinks(game) if game else None
        self.assets = no_symlinks(assets) if assets else None
        self.scene = park.make_scene('Untitled park')
        self.past, self.future = [], []
        self.saved = self.snapshot()
        self.filename = 'park.json'
        self.process = None
        self.play_directory = None
        self.guard = self.job = self.log_reader = None
        self.finished_exit = None
        self.finished_diagnostic = ''

    def snapshot(self): return json.dumps(self.scene, allow_nan=False, sort_keys=True).encode()
    def path(self, value): return no_symlinks(self.workspace / relative(value))
    def bound(self, history):
        while len(history) > MAX_HISTORY or sum(map(len, history)) > MAX_HISTORY_BYTES: history.pop(0)
    def edit(self, scene):
        park.validate(scene)
        raw = json.dumps(scene, allow_nan=False).encode()
        if len(raw) > MAX_SCENE_BYTES: raise ValueError('scene exceeds 4 MiB')
        # Validation includes rendered world coordinates and degeneracy, without
        # allocating the complete encoded map until explicit export/playtest.
        points = sum(len(o.get('points', [])) for o in scene['objects'])
        if points > 16384: raise ValueError('scene exceeds 16384 total rail points')
        for item in scene['objects']:
            for triangle in park.triangles(item):
                for point in triangle: park.vector(point)
        self.past.append(self.snapshot()); self.bound(self.past); self.future.clear()
        self.scene = copy.deepcopy(scene)
    def undo(self):
        if self.past:
            self.future.append(self.snapshot()); self.bound(self.future)
            self.scene = json.loads(self.past.pop())
    def redo(self):
        if self.future:
            self.past.append(self.snapshot()); self.bound(self.past)
            self.scene = json.loads(self.future.pop())
    def save(self, path):
        target = self.path(path)
        if target.suffix != '.json': raise ValueError('save file must end with .json')
        park.save(target, self.scene); self.saved = self.snapshot(); self.filename = path
    def load(self, path):
        scene = park.load(self.path(path)); self.edit(scene); self.saved = self.snapshot(); self.filename = path
    def export(self, path, resource_id):
        target = self.path(path)
        # Existing user scripts are kept. Never follow symlinks inside a package.
        if target.exists():
            for entry in target.rglob('*'): no_symlinks(entry)
        return park.export(self.scene, target, resource_id)
    def playtest(self):
        if not self.game or not self.game.is_file(): raise ValueError('set --game-executable to a trusted built skate3rust executable')
        if not self.assets or not self.assets.is_dir(): raise ValueError('set --assets to your prepared local assets directory')
        if self.process and self.process.poll() is None: raise ValueError('a playtest is already running; stop it before testing another revision')
        self.stop_playtest()
        self.play_directory = tempfile.TemporaryDirectory(prefix='skate-park-playtest-')
        root = Path(self.play_directory.name)
        park.export(self.scene, root / 'resource', 'editor-playtest')
        (root / 'game.log').touch()
        environment = os.environ.copy()
        if sys.platform.startswith('linux'):
            paths = [str(self.game.parent / 'deps')]
            if shutil.which('rustc'):
                result = subprocess.run(['rustc', '--print', 'target-libdir'], check=True, capture_output=True, text=True, timeout=10)
                paths.append(result.stdout.strip())
            paths.append(environment.get('LD_LIBRARY_PATH', ''))
            environment['LD_LIBRARY_PATH'] = os.pathsep.join(filter(None, paths))
        # Share supervision's trusted gated launcher and child containment.
        # Native code runs only after the group guard / Windows Job is installed.
        try:
            self.process = subprocess.Popen([sys.executable, str(Path(supervision.__file__).resolve()), '--launch', str(self.game), '--assets', str(self.assets), '--map', str(root / 'resource' / 'park.skate')], env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=os.name != 'nt')
            self.job = supervision.WindowsJob(self.process)
            self.guard = supervision.GroupGuard(self.process.pid)
            self.log_reader = threading.Thread(target=self.capture_log, args=(self.process.stdout, root), daemon=True)
            self.log_reader.start()
            self.process.stdin.write(b'\x01'); self.process.stdin.flush()
        except BaseException:
            self.stop_playtest(); raise
        return 'Native playtest started. Close the game or use Stop playtest before applying edits.'
    @staticmethod
    def capture_log(pipe, root):
        size = 0
        with pipe:
            while chunk := pipe.read1(4096):
                if size + len(chunk) > 1024 * 1024:
                    os.replace(root / 'game.log', root / 'game.previous.log'); size = 0
                with (root / 'game.log').open('ab') as log: log.write(chunk)
                size += len(chunk)
    def stop_playtest(self, preserve_diagnostics=False):
        exit_code = self.process.poll() if self.process else None
        if self.process:
            # Always retire descendants, including after the native leader exits.
            if os.name != 'nt':
                try: os.killpg(self.process.pid, signal.SIGTERM)
                except ProcessLookupError: pass
            elif self.job is not None: self.job.close(); self.job = None
            try: self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                if os.name != 'nt':
                    try: os.killpg(self.process.pid, signal.SIGKILL)
                    except ProcessLookupError: pass
                else: self.process.kill()
                self.process.wait(timeout=3)
            if self.guard is not None: self.guard.close(); self.guard = None
            if self.job is not None: self.job.close(); self.job = None
            self.process.stdin.close()
        self.process = None
        if self.log_reader: self.log_reader.join(timeout=3); self.log_reader = None
        self.finished_exit = exit_code if preserve_diagnostics else None
        self.finished_diagnostic = ''
        if preserve_diagnostics and self.play_directory:
            path = Path(self.play_directory.name) / 'game.log'
            if path.exists():
                with path.open('rb') as log:
                    log.seek(max(0, path.stat().st_size - 4096)); self.finished_diagnostic = log.read().decode(errors='replace')
        if self.play_directory: self.play_directory.cleanup(); self.play_directory = None
    def state(self):
        if self.process and self.process.poll() is not None: self.stop_playtest(preserve_diagnostics=True)
        return {'scene':self.scene, 'filename':self.filename, 'dirty':self.saved != self.snapshot(), 'undo':bool(self.past), 'redo':bool(self.future), 'playtest': {'available': bool(self.game and self.assets), 'running': bool(self.process and self.process.poll() is None), 'exit_code':self.finished_exit, 'diagnostic':self.finished_diagnostic}}


class EditorServer(HTTPServer):
    def server_close(self):
        self.editor.stop_playtest(); super().server_close()


def make_server(editor, port):
    class Handler(BaseHTTPRequestHandler):
        def setup(self): super().setup(); self.connection.settimeout(3)
        def log_message(self, *args): pass
        def respond(self, code, content, mime='application/json'):
            raw = json.dumps(content, allow_nan=False).encode() if mime == 'application/json' else content
            self.send_response(code); self.send_header('Content-Type', mime); self.send_header('Content-Length', str(len(raw)))
            self.send_header('Cache-Control', 'no-store'); self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
            self.end_headers(); self.wfile.write(raw)
        def authorized(self, api=False):
            host = f'127.0.0.1:{self.server.server_port}'
            if self.headers.get('Host') != host: return False
            origin = self.headers.get('Origin')
            if origin is not None and origin != f'http://{host}': return False
            return not api or secrets.compare_digest(self.headers.get('X-Editor-Token', ''), self.server.token)
        def do_GET(self):
            if not self.authorized(api=self.path.startswith('/api/')): self.respond(403, {'error':'Editor origin or session token rejected'}); return
            if self.path == '/api/state': self.respond(200, editor.state()); return
            name = {'/':'index.html', '/editor.js':'editor.js', '/editor.css':'editor.css'}.get(self.path)
            if not name: self.respond(404, {'error':'Unknown editor route'}); return
            mime = {'html':'text/html; charset=utf-8', 'js':'text/javascript; charset=utf-8', 'css':'text/css; charset=utf-8'}[name.split('.')[-1]]
            self.respond(200, (Path(__file__).parent / 'park_editor' / name).read_bytes(), mime)
        def do_POST(self):
            if not self.authorized(api=True): self.respond(403, {'error':'Editor origin or session token rejected'}); return
            if self.path != '/api/action': self.respond(404, {'error':'Unknown editor route'}); return
            try:
                if self.headers.get('Transfer-Encoding') or self.headers.get('Content-Type') != 'application/json': raise ValueError('expected bounded application/json')
                length = int(self.headers.get('Content-Length', '-1'))
                if not 0 <= length <= MAX_SCENE_BYTES: raise ValueError('request exceeds 4 MiB')
                raw = self.rfile.read(length)
                if len(raw) != length: raise ValueError('incomplete request')
                request = json.loads(raw)
                action = request['action']; message = ''
                if action == 'edit': editor.edit(request['scene'])
                elif action in ('undo', 'redo'): getattr(editor, action)()
                elif action in ('save', 'load'): getattr(editor, action)(request['path']); message = f'{action.title()} complete: {request["path"]}'
                elif action == 'export': message = f'Exported {editor.export(request["path"], request["resource_id"])} world bytes. Restart the resource on a stopped/test server to publish.'
                elif action == 'playtest': message = editor.playtest()
                elif action == 'stop_playtest': editor.stop_playtest(); message = 'Playtest stopped'
                else: raise ValueError('unknown action')
                self.respond(200, dict(editor.state(), message=message))
            except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as error: self.respond(400, {'error':str(error)})
    server = EditorServer(('127.0.0.1', port), Handler); server.token = secrets.token_urlsafe(32); server.editor = editor
    return server


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', type=Path, required=True, help='Only this local directory can be read/written')
    parser.add_argument('--scene', help='Placement JSON relative to workspace')
    parser.add_argument('--game-executable', type=Path, help='Trusted local skate3rust binary for native playtest')
    parser.add_argument('--assets', type=Path, help='Prepared local assets; never exported')
    parser.add_argument('--port', type=int, default=0); parser.add_argument('--open', action='store_true')
    args = parser.parse_args()
    try:
        model = Editor(args.workspace, args.game_executable, args.assets)
        if args.scene: model.load(args.scene)
        server = make_server(model, args.port)
        url = f'http://127.0.0.1:{server.server_port}/#{server.token}'
        print(f'Park editor: {url}', flush=True)
        if args.open: webbrowser.open(url)
        def interrupted(*_): raise KeyboardInterrupt
        signal.signal(signal.SIGTERM, interrupted)
        try: server.serve_forever()
        except KeyboardInterrupt: pass
        finally: server.server_close()
    except (ValueError, OSError) as error: parser.error(str(error))

if __name__ == '__main__': main()
