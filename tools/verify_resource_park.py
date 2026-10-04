#!/usr/bin/env python3
"""Run owned-asset graphical acceptance; never build or extract private assets.

Each scenario runs a real dedicated server and two delayed-start native clients.
Captures and logs remain under --output, including failures. Run in a graphical
session (or explicitly under xvfb-run on Linux). Inspect the PNGs separately:
protocol/physics assertions alone do not establish correct visual presentation.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]
RESOURCES = ('community-park', 'shared-objects', 'presentation-demo')
SCENARIOS = ('initial', 'world-stop', 'world-restart', 'presentation-stop', 'presentation-restart', 'lifecycle', 'interaction')
ERRORS = ('panicked at', 'Resource activation failed', 'Dedicated resources stopped',
          'MAP_TRANSITION_FAILED', 'Resource failure publication:', 'Shared entity community-park:')


def inspect_capture(log, native, exit_code, screenshot, scenario, *, periodic=False,
                    object_count=3, peer_count=None, presentation=None, playing=False):
    failures = []
    for valid, reason in (
        (exit_code == 0, 'client did not exit successfully'),
        (screenshot, 'screenshot is missing'),
        (('GAME_VERIFY_PHASE' if periodic else 'GAME_VERIFY_OK') in log, 'native verification did not complete'),
        ('RESOURCE_ACTIVATED' in log, 'resource admission is missing'),
        ('MAP_TRANSITION_COMMITTED' in log, 'required world never committed'),
        ('remote_count:1' in log, 'second native player was never observed'),
    ):
        if not valid:
            failures.append(reason)
    world = re.search(r'^World: (.+) generation=(\d+) triangles=(\d+) native_grind_primitives=(\d+)$', native, re.M)
    if not world:
        failures.append('world diagnostics are missing')
    elif scenario == 'world-stop':
        if world[1] != 'Test world' or int(world[2]) < 2:
            failures.append('stopped park did not restore the local test world')
    elif world[1] != 'Community Practice Park' or int(world[3]) != 32 or int(world[4]) < 1:
        failures.append('required park collision/native grind world is absent at capture')
    solids = re.search(r'^Shared objects: registered:(\d+) live_solids:(\d+)', native, re.M)
    if not solids or tuple(map(int, solids.groups())) != (object_count, object_count):
        failures.append(f'exactly {object_count} shared objects must remain registered and collidable')
    if peer_count is not None and not re.search(rf'^Multiplayer: .*remote_count:{peer_count}(?:\s|$)', native, re.M):
        failures.append('current native peer visibility is incorrect')
    if object_count == 0:
        inventory = re.search(r'^Replicated object inventory: (.+)$', native, re.M)
        if not inventory or json.loads(inventory[1]) != []:
            failures.append('private empty instance retains a replicated public object')
        if 'Current shared entity contact ids: []\n' not in native:
            failures.append('private empty instance retains an active public object contact')
    if presentation is not None:
        animation = re.search(r'^Resource animation: banks:(\d+) layers:(\d+) appearances:(\d+) attachments:(\d+) pending:(\d+)$', native, re.M)
        values = tuple(map(int, animation.groups())) if animation else None
        if (not presentation and values != (0, 0, 0, 0, 0)) or (presentation and (not values or values[2:4] != (2, 2) or values[4] != 0)):
            failures.append('resource presentation has not reached the expected lifecycle state')
    if playing and ('Gameplay paused: false\n' not in native or 'Pause menu open: false\n' not in native):
        failures.append('resource lifecycle left gameplay paused or the loading menu open')
    failures.extend(line for line in log.splitlines() if any(error in line for error in ERRORS))
    return {'ok': not failures, 'failures': failures, 'exit': exit_code,
            'world': world.group(0) if world else None,
            'shared_objects': solids.group(0) if solids else None,
            'presentation_events': sorted(set(re.findall(r'PRESENTATION_CHECK ([^\n]+)', log))),
            'native_report': native}


def assert_statuses(log, expected):
    statuses = dict(re.findall(r'^([\w-]+): (started|stopped) generation=\d+$', log, re.M))
    for name, running in expected.items():
        required = 'started' if running else 'stopped'
        if statuses.get(name) != required:
            raise RuntimeError(f'{name}: expected {required}, got {statuses.get(name, "no status")}')


def read(path):
    return path.read_text(errors='replace') if path.exists() else ''


def wait_until(predicate, timeout, description, processes=()):
    deadline = time.monotonic() + timeout
    while not predicate():
        if any(process.poll() is not None for process in processes):
            raise RuntimeError(f'{description}: a required process exited; inspect logs')
        if time.monotonic() >= deadline:
            raise RuntimeError(f'{description}: timed out after {timeout:g}s')
        time.sleep(0.1)


def stop(process, graceful=False):
    if process.poll() is not None:
        return
    if graceful and process.stdin:
        try:
            process.stdin.write('quit\n')
            process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
    else:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def instrument_client(path):
    original = path.read_text()
    # The production example is unchanged; wrap its returned callbacks in this
    # run's private copy to record real lifecycle and animation events.
    path.write_text('local callbacks=(function()\n' + original + '\nend)()\n' + '''
local load, unload, event = callbacks.on_load, callbacks.on_unload, callbacks.on_event
local seen={}
callbacks.on_load=function(...) if load then load(...) end; sdk.log("VERIFY_PRESENTATION_LOAD") end
callbacks.on_unload=function(...) if unload then unload(...) end; sdk.log("VERIFY_PRESENTATION_UNLOAD") end
callbacks.on_event=function(value,...)
    if value.type=="animation" then
        local key=tostring(value.key)..":"..tostring(value.event.kind)
        if not seen[key] then sdk.log("PRESENTATION_CHECK "..key);seen[key]=true end
    end
    if event then event(value,...) end
end
return callbacks
''')


def environment(bin_dir, seconds):
    env = os.environ.copy()
    env.update(RUST_LOG='info', SKATE_VERIFY_SECONDS=str(seconds))
    if sys.platform.startswith('linux'):
        paths = [str(bin_dir / 'deps')]
        if shutil.which('rustc'):
            result = subprocess.run(['rustc', '--print', 'target-libdir'], check=True,
                                    capture_output=True, text=True, timeout=10)
            paths.append(result.stdout.strip())
        paths.append(env.get('LD_LIBRARY_PATH', ''))
        env['LD_LIBRARY_PATH'] = os.pathsep.join(filter(None, paths))
    return env


def run_scenario(args, scenario, *, prepare=None, drive=None):
    run = args.output / scenario
    run.mkdir()
    root = run / 'resources'
    root.mkdir()
    for name in RESOURCES:
        shutil.copytree(REPO / 'resources' / name, root / name)
    instrument_client(root / 'presentation-demo' / 'client.lua')
    selected = list(RESOURCES)
    if args.overview:
        camera = root / 'verification-view'
        camera.mkdir()
        (camera / 'resource.json').write_text(json.dumps({
            'format': 1, 'api': 1, 'id': 'verification-view', 'version': '1.0.0',
            'language': 'lua', 'client_scripts': ['client.lua'], 'capabilities': ['engine.camera']}))
        (camera / 'client.lua').write_text('return {on_update=function() sdk.camera.set({-14,7,-11},{-2,1,1}) end}')
        selected.append('verification-view')
    if prepare:
        prepare(root, selected)
    grants = {name: json.loads((root / name / 'resource.json').read_text()).get('capabilities', []) for name in selected}
    server_config = {'root': str(root), 'storage': str(run / 'storage'),
                     'ensure': selected, 'grants': grants}
    server_config.update(getattr(args, 'server_config', {}))
    (run / 'server.json').write_text(json.dumps(server_config))
    env = environment(args.bin_dir, args.seconds)
    env['SKATE_VERIFY_INTERVAL_SECONDS'] = str(args.interval)
    if scenario == 'interaction':
        env['SKATE_RESOURCE_DIAGNOSTICS'] = '1'
    suffix = '.exe' if os.name == 'nt' else ''
    handles, processes, clients, phases, endpoints = [], [], [], [], []
    result = {'scenario': scenario, 'ok': False, 'phases': phases, 'clients': [],
              'visual_review_required': True}
    try:
        server_log = open(run / 'server.log', 'w')
        handles.append(server_log)
        server = subprocess.Popen([str(args.bin_dir / ('skate-server' + suffix)), '--test-world', '--bind',
                                   '127.0.0.1:0', '--resources', str(run / 'server.json')],
                                  stdin=subprocess.PIPE, stdout=server_log, stderr=subprocess.STDOUT,
                                  text=True, env=env, cwd=REPO)
        processes.append(server)
        wait_until(lambda: 'Dedicated server listening on ' in read(run / 'server.log'),
                   args.timeout, 'server startup', [server])
        server_text = read(run / 'server.log')
        address = re.search(r'Dedicated server listening on (127\.0\.0\.1:\d+) \| session=(\d+)', server_text)
        if not address:
            raise RuntimeError('server did not publish a loopback endpoint and session')
        endpoint, session = address.groups()

        def command(line):
            server.stdin.write(line + '\n')
            server.stdin.flush()

        def status(expected):
            offset = len(read(run / 'server.log'))
            command('resources')
            wait_until(lambda: all(re.search(rf'^{re.escape(name)}: (started|stopped) generation=', read(run / 'server.log')[offset:], re.M)
                                   for name in expected), 5, 'console resource status', [server])
            snapshot = read(run / 'server.log')[offset:]
            assert_statuses(snapshot, expected)
            return snapshot

        expected = dict.fromkeys(selected, True)
        status(expected)
        for index in range(2):
            if index:
                wait_until(lambda: 'RESOURCE_ACTIVATED' in read(run / 'client0.log'), args.timeout,
                           'first native client admission', processes)
                status(expected)
                if scenario != 'interaction':
                    command('command objects_push 0')
            cache = run / f'cache{index}'
            cache.mkdir()
            client_endpoint = endpoint
            factory = getattr(args, 'endpoint_factory', None)
            if factory:
                forwarding = factory(endpoint, index)
                endpoints.append(forwarding)
                client_endpoint = forwarding.endpoint
            (cache / 'grants.json').write_text(json.dumps({f'udp://{client_endpoint}/{session}': grants}))
            client_env = {**env, 'SKATE3_RESOURCE_CACHE': str(cache), 'SKATE3_MODS': str(run / f'mods{index}'),
                          'SKATE3_MOD_SETTINGS': str(run / f'settings{index}')}
            log = open(run / f'client{index}.log', 'w')
            handles.append(log)
            client = subprocess.Popen([str(args.bin_dir / ('skate3rust' + suffix)), '--assets', str(args.assets),
                                       '--test-world', '--connect', client_endpoint, '--player-title', f'Park observer {index+1}',
                                       '--verify', str(run / f'client{index}.png')],
                                      stdout=log, stderr=subprocess.STDOUT, env=client_env, cwd=REPO)
            processes.append(client)
            clients.append(client)
        wait_until(lambda: all('RESOURCE_ACTIVATED' in read(run / f'client{i}.log') for i in range(2)),
                   args.timeout, 'two native client admissions', processes)
        status(expected)
        if scenario != 'interaction':
            command('command objects_push 0')
        def phase_number(index):
            # Requested captures count too: an image still in flight belongs
            # to its old phase even if its file appears after readmission.
            values = [int(value) for value in re.findall(r'GAME_VERIFY_PHASE (\d+)', read(run / f'client{index}.log'))]
            return max(values, default=0)

        def capture_phase(name, expected_world, previous, presentation=None):
            evidence = [None, None]
            def ready():
                for index in range(2):
                    for image in sorted(run.glob(f'client{index}.phase*.png')):
                        number = int(re.search(r'phase(\d+)\.png$', image.name)[1])
                        if number <= previous[index] or not image.with_suffix('.input.txt').is_file():
                            continue
                        log = read(run / f'client{index}.log')
                        report = inspect_capture(log, read(image.with_suffix('.input.txt')), 0, True,
                                                 'initial' if expected_world else 'world-stop', periodic=True, presentation=presentation, playing=True)
                        if report['ok']:
                            evidence[index] = {'image': str(image), 'report': report}
                return all(evidence)
            wait_until(ready, args.timeout, f'{name} periodic snapshots', processes)
            phases.append({'name': name, 'server': status(expected), 'captures': evidence})

        plan = getattr(args, 'phase_plan', None)
        if plan is None and scenario == 'lifecycle':
            plan = [
                {'name': 'presentation stopped', 'commands': ['stop presentation-demo'],
                 'running': {'presentation-demo': False}, 'world': True, 'presentation': False},
                {'name': 'presentation restored', 'commands': ['start presentation-demo'],
                 'running': {'presentation-demo': True}, 'world': True, 'presentation': True},
                {'name': 'world stopped', 'commands': ['stop community-park'],
                 'running': {'community-park': False}, 'world': False},
                {'name': 'world restored', 'commands': ['start community-park'],
                 'running': {'community-park': True}, 'world': True},
            ]
        elif plan is None and scenario != 'initial':
            resource = 'community-park' if scenario.startswith('world-') else 'presentation-demo'
            plan = [{'name': f'{action} {resource}', 'commands': [f'{action} {resource}'],
                     'running': {resource: action == 'start'},
                     'world': not (resource == 'community-park' and action == 'stop')}
                    for action in (['stop', 'start'] if scenario.endswith('restart') else ['stop'])]
        if drive:
            drive(run, command, status, expected, processes, phases)
        elif plan:
            capture_phase('admitted', True, [phase_number(i) for i in range(2)], True)
            for stage in plan:
                previous = [phase_number(i) for i in range(2)]
                offsets = [len(read(run / f'client{i}.log')) for i in range(2)]
                for text in stage['commands']:
                    command(text)
                expected.update(stage.get('running', {}))
                status(expected)
                if stage.get('readmission', True):
                    wait_until(lambda: all('RESOURCE_ACTIVATED' in read(run / f'client{i}.log')[offsets[i]:]
                                           for i in range(2)), args.timeout, stage['name'] + ' readmission', processes)
                previous = [max(previous[i], phase_number(i)) for i in range(2)]
                capture_phase(stage['name'], stage.get('world', True), previous, stage.get('presentation'))
        else:
            phases.append({'name': 'admitted', 'server': read(run / 'server.log')})
        wait_until(lambda: all(client.poll() is not None for client in clients), args.timeout,
                   'native captures', [server])
        status(expected)
        for index, client in enumerate(clients):
            report = inspect_capture(read(run / f'client{index}.log'), read(run / f'client{index}.input.txt'),
                                     client.returncode, (run / f'client{index}.png').is_file(), scenario)
            result['clients'].append({'client': index, **report})
        result['ok'] = all(client['ok'] for client in result['clients'])
        if not result['ok']:
            raise RuntimeError('native capture assertions failed; inspect results.json and images')
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        result['error'] = str(error)
    finally:
        for client in reversed(clients):
            stop(client)
        if processes:
            stop(processes[0], graceful=True)
        if endpoints:
            result['network_impairment'] = [endpoint.close() for endpoint in endpoints]
            if any(report.get('error') or report.get('thread_alive') or report.get('capacity_drops')
                   for report in result['network_impairment']):
                result['ok'] = False
                result['error'] = 'network impairment exceeded its bounds or failed cleanup'
        for handle in handles:
            handle.close()
        (run / 'results.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({'scenario': scenario, 'ok': result['ok'], 'output': str(run), 'error': result.get('error')}), flush=True)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path, help='locally owned prepared asset root')
    parser.add_argument('--output', required=True, type=Path, help='empty output directory; evidence is never removed')
    parser.add_argument('--bin-dir', type=Path, default=REPO / 'target' / 'debug', help='directory containing built game/server binaries')
    parser.add_argument('--scenario', choices=(*SCENARIOS, 'all'), default='initial')
    parser.add_argument('--seconds', type=int, default=36, help='native final capture delay, 20..40 seconds')
    parser.add_argument('--interval', type=int, default=5, help='periodic capture interval, 4..10 seconds; max8 captures')
    parser.add_argument('--timeout', type=float, default=100, help='per-phase timeout, 10..180 seconds')
    parser.add_argument('--overview', action='store_true', help='temporary diagnostic resource holds an overview camera')
    args = parser.parse_args(argv)
    args.assets, args.output, args.bin_dir = (path.resolve() for path in (args.assets, args.output, args.bin_dir))
    if not args.assets.is_dir() or not 20 <= args.seconds <= 40 or not 10 <= args.timeout <= 180 or not 4 <= args.interval <= 10:
        parser.error('assets must be a directory, seconds20..40, interval4..10 and timeout10..180')
    if args.scenario in ('lifecycle', 'interaction', 'all') and args.seconds < args.interval * 6:
        parser.error('lifecycle needs seconds >= 6 * interval for two-client phase snapshots')
    suffix = '.exe' if os.name == 'nt' else ''
    if any(not (args.bin_dir / (name + suffix)).is_file() for name in ('skate3rust', 'skate-server')):
        parser.error('--bin-dir must contain built skate3rust and skate-server executables')
    if args.output.exists() and any(args.output.iterdir()):
        parser.error('--output must be absent or empty; existing evidence is never overwritten')
    args.output.mkdir(parents=True, exist_ok=True)
    scenarios = SCENARIOS if args.scenario == 'all' else (args.scenario,)
    results = []
    for scenario in scenarios:
        if scenario == 'interaction':
            try:
                from tools import verify_resource_objects as objects
            except ModuleNotFoundError:
                import verify_resource_objects as objects
            results.append(run_scenario(args, scenario, prepare=objects.prepare_fixture, drive=objects.drive_phases))
        else:
            results.append(run_scenario(args, scenario))
    (args.output / 'results.json').write_text(json.dumps(results, indent=2))
    return 0 if all(result['ok'] for result in results) else 1


if __name__ == '__main__':
    raise SystemExit(main())
