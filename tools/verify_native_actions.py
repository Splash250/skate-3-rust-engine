#!/usr/bin/env python3
"""Exercise real native authority actions on a redistributable static world.

This is headless companion evidence, not network or graphical acceptance. Only
the trusted map/spawn and normalized controller actions enter the worker. Save
its input log and derived outcomes outside the repository with --output.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import queue
import subprocess
import threading
from pathlib import Path

try:
    from tools import resource_park, verify_resource_park
except ModuleNotFoundError:
    import resource_park
    import verify_resource_park


def native_executable(bin_dir):
    return bin_dir / ('skate3rust.exe' if os.name == 'nt' else 'skate3rust')


class Worker:
    def __init__(self, executable, assets, world, directory):
        self.log = open(directory / 'worker.log', 'w')
        self.process = subprocess.Popen(
            [str(executable), '--native-authority', '--assets', str(assets),
             '--map', str(world), '--difficulty', 'normal'], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.log,
            env=verify_resource_park.environment(executable.parent, 0))
        self.queue = queue.Queue(maxsize=1)

        def read():
            try:
                while line := self.process.stdout.readline(32769):
                    if len(line) > 32768 or not line.endswith(b'\n'):
                        raise RuntimeError('native reply framing exceeded32KiB')
                    self.queue.put(json.loads(line), timeout=5)
                self.queue.put(RuntimeError('native worker closed'), timeout=5)
            except Exception as error:
                try:
                    self.queue.put(error, timeout=5)
                except queue.Full:
                    pass

        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        try:
            self.snapshot = self.request({'kind': 'start', 'admission':
                                          {'version': 1, 'epoch': 1, 'instance': 41, 'generation': 1}})
        except Exception:
            self.close()
            raise

    def request(self, request):
        payload = json.dumps(request, separators=(',', ':')).encode() + b'\n'
        if len(payload) > 2048:
            raise RuntimeError('native input frame exceeds2KiB')
        self.process.stdin.write(payload)
        self.process.stdin.flush()
        try:
            reply = self.queue.get(timeout=20)
        except queue.Empty as error:
            raise RuntimeError('native worker deadline exceeded') from error
        if isinstance(reply, Exception):
            raise reply
        if reply['kind'] == 'rejected':
            raise RuntimeError(reply['error'])
        return reply['snapshot']

    def step(self, tick, actions):
        self.snapshot = self.request({'kind': 'step', 'input': {'epoch': 1, 'tick': tick, 'actions': actions}})
        return self.snapshot

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.reader.join(timeout=6)
        self.log.close()


def scene(rail_height, spawn_x=0, rail_x=.3):
    return {'format': 1, 'name': 'Native authority action fixture', 'spawn': [spawn_x, 0, -10],
            'heading': 0, 'objects': [
                {'id': 'floor', 'kind': 'box', 'position': [0, -.25, 0], 'size': [80, .5, 120]},
                {'id': 'rail', 'kind': 'rail', 'position': [rail_x, rail_height, 0],
                 'points': [[0, 0, -5], [0, 0, 1.5]]}]}


def scenario_world(scenario, rail_height=.65, spawn_x=0, rail_x=.3, drop_height=6):
    world = scene(rail_height, spawn_x, rail_x)
    if scenario != 'rail':
        world['objects'].pop()
    if scenario in ('grab', 'combo'):
        world['objects'][0]['position'][1] -= drop_height
        world['objects'].append({'id': 'runway', 'kind': 'box', 'position': [0, -0.5, -22],
                                 'size': [20, 1, 36]})
    return world


def drive(worker, ticks, pop_position, scenario):
    history, observations = [], []
    publications = []
    preload, saw_grind, saw_grab = None, False, False
    grab_started = None
    combo_started = None
    peak_multiplier = 1.0
    for tick in range(1, ticks + 1):
        actions = [0.0] * 18
        previous = worker.snapshot
        if scenario == 'rail':
            if tick > 30 and preload is None:
                actions[16] = 1.0
                if previous['root']['p'][2] >= pop_position:
                    preload = tick
            if preload is not None:
                age = tick - preload
                if age < 24:
                    actions[4] = -1.0
                elif age < 27:
                    actions[4] = 1.0
        elif scenario in ('grab', 'combo'):
            if grab_started is None:
                if 200 <= previous['state'] < 300:
                    grab_started = tick
                elif tick > 30:
                    actions[16] = 1.0
            if grab_started is not None and tick - grab_started < 35:
                actions[6] = 1.0
            if scenario == 'combo':
                if combo_started is None and previous['score']['landing_seq'] == 1 and previous['state'] == 100:
                    combo_started = tick
                if combo_started is not None:
                    age = tick - combo_started
                    if age < 150:
                        actions[17] = 1.0  # ordinary foot brake before the next pop
                    elif 180 <= age < 210:
                        actions[4] = -1.0
                    elif 210 <= age < 213:
                        actions[3:5] = [-.89442, .44722]
        else:
            if 120 <= tick < 150:
                actions[4] = -1.0
            elif 150 <= tick < 153:
                actions[3:5] = [-.89442, .44722] if scenario == 'flip' else [0.0, 1.0]
        state = worker.step(tick, actions)
        history.append({'epoch': 1, 'tick': tick, 'actions': actions})
        saw_grind |= 400 <= state['state'] <= 405
        saw_grab |= 'GRAB' in state['score']['trick']
        peak_multiplier = max(peak_multiplier, state['score']['multiplier'])
        published = state['score']['publications'] > previous['score']['publications']
        if published:
            publications.append({'tick': tick, 'applied_multiplier': previous['score']['multiplier'],
                                 'score': state['score']})
        if published or tick % 10 == 0 or state['state'] != previous['state'] or state['score']['trick_seq'] != previous['score']['trick_seq']:
            observation = {key: state[key] for key in ('tick', 'state', 'root', 'score')}
            observation['deck'] = state['bodies'][6]
            observations.append(observation)
    score = worker.snapshot['score']
    semantic = saw_grind if scenario == 'rail' else saw_grab if scenario in ('grab', 'combo') else 'HEELFLIP' in score['landed_trick']
    banked = score['completed_lines'] > 0 and score['line'] == 0 and peak_multiplier > 1
    success = (semantic and score['landing_seq'] == (2 if scenario == 'combo' else 1) and score['bail_seq'] == 0 and
               worker.snapshot['state'] == 100 and score.get('awarded', 0) > 0 and
               score.get('publications', 0) > 0)
    success &= banked if scenario != 'flip' else score['sequence'] > 0
    if scenario == 'combo':
        success &= (len(publications) == 2 and score['publications'] == 2 and
                    all(p['score']['clean'] and not p['score']['sketchy'] for p in publications))
        if len(publications) == 2:
            first, second = publications
            success &= ('GRAB' in first['score']['landed_trick'] and
                        'HEELFLIP' in second['score']['landed_trick'] and
                        second['applied_multiplier'] == 1.5 and second['score']['sequence'] == 33.0 and
                        second['score']['awarded'] - first['score']['awarded'] == 33.0 and
                        score['awarded'] == second['score']['awarded'])
    return {'ok': success,
            'scenario': scenario, 'saw_grind': saw_grind, 'saw_grab': saw_grab,
            'peak_multiplier': peak_multiplier, 'preload_tick': preload,
            'combo_start_tick': combo_started, 'publications': publications,
            'final_score': score, 'final_snapshot': worker.snapshot,
            'history': history, 'observations': observations}


def snapshot_digest(snapshot):
    """Evidence file SHA-256; the protocol uses its own BLAKE3 state digest."""
    return hashlib.sha256(json.dumps(snapshot, sort_keys=True, separators=(',', ':'),
                                     allow_nan=False).encode()).hexdigest()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--executable', type=Path, default=native_executable(verify_resource_park.REPO / 'target/debug'))
    parser.add_argument('--scenario', choices=['flip', 'grab', 'rail', 'combo'], default='rail')
    parser.add_argument('--ticks', type=int, default=900)
    parser.add_argument('--pop-position', type=float, default=-7.5)
    parser.add_argument('--rail-height', type=float, default=.65)
    parser.add_argument('--spawn-x', type=float, default=0)
    parser.add_argument('--rail-x', type=float, default=.3)
    parser.add_argument('--drop-height', type=float, default=6)
    args = parser.parse_args(argv)
    if not 300 <= args.ticks <= 3600:
        parser.error('--ticks must be300..3600')
    args.assets, args.executable, args.output = (p.resolve() for p in (args.assets, args.executable, args.output))
    args.output.mkdir(parents=True, exist_ok=False)
    world = scenario_world(args.scenario, args.rail_height, args.spawn_x, args.rail_x, args.drop_height)
    (args.output / 'world.json').write_text(json.dumps(world, indent=2))
    map_path = args.output / 'world.skate'
    map_path.write_bytes(resource_park.encode(world))
    worker = None
    try:
        worker = Worker(args.executable, args.assets, map_path, args.output)
        result = drive(worker, args.ticks, args.pop_position, args.scenario)
        worker.close()
        worker = None
        replay_dir = args.output / 'replay'
        replay_dir.mkdir()
        worker = Worker(args.executable, args.assets, map_path, replay_dir)
        for entry in result['history']:
            worker.step(entry['tick'], entry['actions'])
        result['replay_matches'] = worker.snapshot == result['final_snapshot']
        result['final_snapshot_sha256'] = snapshot_digest(result['final_snapshot'])
        result['replay_snapshot_sha256'] = snapshot_digest(worker.snapshot)
        result['ok'] &= result['replay_matches']
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        result = {'ok': False, 'error': str(error)}
    finally:
        if worker:
            worker.close()
    (args.output / 'results.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({key: value for key, value in result.items() if key not in ('history', 'observations', 'final_snapshot')}))
    return 0 if result['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
