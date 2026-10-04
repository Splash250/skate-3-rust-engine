#!/usr/bin/env python3
"""Verify a server-simulated native trick with two real graphical clients.

The selected player enters a solitary instance and supplies only ordinary mapped
controller actions. Trusted native physics derives the stock landing and score.
Requires freshly built matching game/server binaries and owned prepared assets.
"""
from __future__ import annotations

import argparse
import heapq
import json
import random
import re
import socket
import threading
import time
from pathlib import Path

try:
    from tools import verify_resource_park as park
    from tools.verify_native_actions import native_executable
except ModuleNotFoundError:
    import verify_resource_park as park
    from verify_native_actions import native_executable


class ImpairedEndpoint:
    """Bounded local UDP forwarding; no packet contents are retained in evidence."""
    MAX_DATAGRAMS = 4096
    MAX_BYTES = 2 * 1024 * 1024

    def __init__(self, upstream, seed, *, delay=(.025, .075), loss=.01):
        host, port = upstream.rsplit(':', 1)
        if host != '127.0.0.1' or not 0 < int(port) <= 65535:
            raise ValueError('the impairment endpoint only forwards to loopback')
        if not 0 <= delay[0] <= delay[1] <= 1 or not 0 <= loss <= 1:
            raise ValueError('invalid bounded delay/loss settings')
        self.upstream = (host, int(port))
        self.seed, self.delay, self.loss = seed, delay, loss
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(('127.0.0.1', 0))
        self.socket.settimeout(.005)
        self.address = self.socket.getsockname()
        self.endpoint = f'{self.address[0]}:{self.address[1]}'
        self.stopping = threading.Event()
        self.metrics = {'seed': seed, 'delay_ms_per_direction': [value * 1000 for value in delay],
                        'loss_probability': loss, 'forwarded': {'client_to_server': 0, 'server_to_client': 0},
                        'forwarded_bytes': 0, 'random_drops': 0, 'capacity_drops': 0,
                        'unknown_sender_drops': 0, 'reordered_deliveries': 0,
                        'peak_queued_datagrams': 0, 'peak_queued_bytes': 0,
                        'queued_datagrams': 0, 'queued_bytes': 0, 'shutdown_discards': 0,
                        'error': None}
        self.thread = threading.Thread(target=self._run, name='native-udp-impairment', daemon=True)
        self.thread.start()

    def _run(self):
        rng = random.Random(self.seed)
        pending, client, serial = [], None, 0
        delivered = {'client_to_server': 0, 'server_to_client': 0}
        try:
            while not self.stopping.is_set():
                now = time.monotonic()
                while pending and pending[0][0] <= now:
                    _, order, direction, target, data = heapq.heappop(pending)
                    self.metrics['queued_bytes'] -= len(data)
                    self.metrics['queued_datagrams'] -= 1
                    self.socket.sendto(data, target)
                    self.metrics['forwarded'][direction] += 1
                    self.metrics['forwarded_bytes'] += len(data)
                    if order < delivered[direction]: self.metrics['reordered_deliveries'] += 1
                    delivered[direction] = max(order, delivered[direction])
                try:
                    data, source = self.socket.recvfrom(65536)
                except socket.timeout:
                    continue
                if source == self.upstream and client is not None:
                    direction, target = 'server_to_client', client
                elif source != self.upstream and source[0] == '127.0.0.1' and (client is None or source == client):
                    client = source
                    direction, target = 'client_to_server', self.upstream
                else:
                    self.metrics['unknown_sender_drops'] += 1
                    continue
                serial += 1
                if rng.random() < self.loss:
                    self.metrics['random_drops'] += 1
                    continue
                size = self.metrics['queued_bytes'] + len(data)
                if len(pending) >= self.MAX_DATAGRAMS or size > self.MAX_BYTES:
                    self.metrics['capacity_drops'] += 1
                    continue
                heapq.heappush(pending, (time.monotonic() + rng.uniform(*self.delay), serial, direction, target, data))
                self.metrics['queued_bytes'] = size
                self.metrics['queued_datagrams'] = len(pending)
                self.metrics['peak_queued_datagrams'] = max(self.metrics['peak_queued_datagrams'], len(pending))
                self.metrics['peak_queued_bytes'] = max(self.metrics['peak_queued_bytes'], size)
        except OSError as error:
            self.metrics['error'] = str(error)
        finally:
            self.metrics['shutdown_discards'] = len(pending)
            pending.clear()
            self.metrics['queued_datagrams'] = self.metrics['queued_bytes'] = 0

    def close(self):
        self.stopping.set()
        self.thread.join(timeout=2)
        self.socket.close()
        self.metrics['thread_alive'] = self.thread.is_alive()
        return dict(self.metrics)


def inspect_attempt(server, client, actor):
    failures = []
    escaped = re.escape(actor)
    starts = re.findall(rf'NATIVE_VERIFY_STARTED actor={escaped} epoch=([1-9]\d*)', server)
    results = re.findall(rf'NATIVE_VERIFY_RESULT actor={escaped} kind=completed ticks=(\d+) points=([\d.]+) publications=(\d+) landings=(\d+) trick=(\S+) rules=(\S+)', server)
    if len(starts) != 1 or len(results) != 1:
        failures.append('exactly one native authority admission and completion are required')
    result = results[0] if len(results) == 1 else None
    if result and (int(result[0]) != 300 or float(result[1]) <= 0 or int(result[2]) < 1 or int(result[3]) < 1
                   or 'HEELFLIP' not in result[4] or result[5] != 'native-input-v1'):
        failures.append('the server must independently bank the stock heelflip landing')
    if re.search(rf'NATIVE_VERIFY_RESULT actor={escaped} kind=(rejected|cancelled|error)', server):
        failures.append('native authority rejected or cancelled the attempt')
    epoch = starts[0] if len(starts) == 1 else None
    if epoch and not re.search(rf'NATIVE_AUTHORITY_COMPLETED epoch={epoch} tick=300 reconciled=true', client):
        failures.append('the graphical client must reconcile to the authoritative terminal tick')
    acks = re.findall(r'NATIVE_AUTHORITY_ACK epoch=(\d+) tick=(\d+) predicted=(\d+) matched=true', client)
    if not any(e == epoch and int(tick) >= 240 for e, tick, _ in acks):
        failures.append('fresh matching prediction acknowledgements through the landing are required')
    if 'Native authority stopped:' in client or 'Native replay differs from authority' in client:
        failures.append('native client reported prediction or runtime failure')
    replays = re.findall(r'NATIVE_AUTHORITY_REPLAY epoch=(\d+) acknowledged=(\d+) through=(\d+) terminal=(true|false)', client)
    return {'ok': not failures, 'failures': failures, 'actor': actor, 'epoch': epoch,
            'completion': dict(zip(('ticks', 'points', 'publications', 'landings', 'trick', 'rules'), result)) if result else None,
            'matching_acknowledgements': sum(e == epoch for e, _, _ in acks),
            'replays': [dict(zip(('epoch', 'acknowledged', 'through', 'terminal'), row)) for row in replays if row[0] == epoch]}


def prepare_fixture(root, selected):
    package = root / 'verification-native'
    package.mkdir()
    (package / 'resource.json').write_text(json.dumps({
        'format': 1, 'api': 1, 'id': 'verification-native', 'version': '1.0.0', 'language': 'lua',
        'client_scripts': ['client.lua'], 'server_scripts': ['server.lua'],
        'dependencies': {'community-park': '1.0.0'},
        'capabilities': ['resource.competition', 'resource.commands', 'resource.state', 'resource.events',
                         'resource.network', 'resource.teleport', 'engine.input', 'engine.ui']}))
    selected.append('verification-native')
    (package / 'server.lua').write_text('''
local actor,expected_instance,before_epoch,ready_epoch=nil,nil,nil,nil
local function request_instance(instance)
 expected_instance=tostring(instance);before_epoch=nil;ready_epoch=nil
 for _,player in ipairs(resource.players()) do
  if player.id==actor then before_epoch=player.movement_epoch end
 end
 resource.teleport(actor,{position={-8,0.2,0},heading=0,velocity={0,0,0},instance=instance})
end
resource.command("native_verify_private","native.verify",function(args)
 assert(args[1],"player required");actor=args[1]
 request_instance(41)
end)
resource.command("native_verify_start","native.verify",function()
 local admitted=false
 for _,player in ipairs(resource.players()) do
  if player.id==actor and player.instance=="41" and player.movement_epoch==ready_epoch then admitted=true end
 end
 assert(admitted,"wait for the server's private instance admission proof")
 expected_instance=nil
 resource.competition.submit({kind="native_start",player=actor,ticks=300})
end)
resource.command("native_verify_return","native.verify",function()
 request_instance(0)
end)
resource.on("competition_result",function(result)
 if result.operation=="native_start" then
  if result.ok then
   sdk.log("NATIVE_VERIFY_STARTED actor="..actor.." epoch="..result.value.movement_epoch)
   resource.state.set("drive",{actor=actor,stage="run"})
  else sdk.log("NATIVE_VERIFY_RESULT actor="..actor.." kind=error reason="..tostring(result.error)) end
 elseif result.kind then
  if result.kind=="completed" then
   local score=result.score
   sdk.log("NATIVE_VERIFY_RESULT actor="..actor.." kind=completed ticks="..result.ticks..
    " points="..score.awarded.." publications="..score.publications.." landings="..score.landing_seq.." trick="..score.landed_trick.." rules="..result.verified_rules)
  else sdk.log("NATIVE_VERIFY_RESULT actor="..actor.." kind="..result.kind.." reason="..tostring(result.reason)) end
  resource.state.set("drive",{actor=actor,stage="finished"})
 end
end)
return {on_fixed_update=function()
 if not expected_instance or ready_epoch then return end
 -- resource.players is filtered by the host's resource admission state. A
 -- client-side activation log does not establish that its ACK reached here.
 for _,player in ipairs(resource.players()) do
  if player.id==actor and player.instance==expected_instance and player.movement_epoch~=before_epoch then
   ready_epoch=player.movement_epoch
   sdk.log("NATIVE_VERIFY_READY actor="..actor.." instance="..expected_instance.." epoch="..ready_epoch)
   return
  end
 end
end}
''')
    (package / 'client.lua').write_text('''
local armed=false
local function release()
 sdk.input.override_action(67,nil);sdk.input.override_action(68,nil)
end
return {on_load=function() sdk.log("NATIVE_VERIFY_LOCAL actor="..sdk.net.info().local_id) end,
 on_fixed_update=function()
  local drive=resource.state.get("drive")
  if not drive or drive.actor~=sdk.net.info().local_id or drive.stage~="run" then release();return end
  local animation=sdk.engine.read("animation") or {}
  local tick=animation.tick or -1
  if tick>=0 and tick<60 then armed=true end
  local x,y=0,0
  if armed and tick>=120 and tick<150 then y=-1
  elseif armed and tick>=150 and tick<153 then x=-0.89442;y=0.44722 end
  sdk.input.override_action(67,x);sdk.input.override_action(68,y)
 end,on_unload=release}
''')


def wait_admission(run, actor, instance, offset, processes):
    pattern = rf'NATIVE_VERIFY_READY actor={re.escape(actor)} instance={re.escape(instance)} epoch=([1-9]\d*)(?:\r?\n|$)'
    def ready():
        server = park.read(run / 'server.log')[offset:]
        if '[verification-native] Resource stopped' in server:
            raise RuntimeError('native verification resource stopped; inspect server.log')
        return re.search(pattern, server)
    park.wait_until(ready, 20, f'server resource admission to instance {instance}', processes)
    return {'actor': actor, 'instance': instance, 'epoch': ready()[1]}


def drive_phases(run, command, status, expected, processes, phases):
    pattern = r'NATIVE_VERIFY_LOCAL actor=([1-9]\d*)'
    park.wait_until(lambda: re.search(pattern, park.read(run / 'client0.log')), 10,
                    'native input driver identity', processes)
    actor = re.search(pattern, park.read(run / 'client0.log'))[1]
    offset = len(park.read(run / 'server.log'))
    command(f'command native_verify_private {actor}')
    admission = wait_admission(run, actor, '41', offset, processes)
    phases.append({'name': 'private server admission', **admission})
    command('command native_verify_start')
    terminal = rf'NATIVE_VERIFY_RESULT actor={actor} kind=(completed|rejected|cancelled|error)'
    def outcome():
        server = park.read(run / 'server.log')
        if '[verification-native] Resource stopped' in server:
            raise RuntimeError('native verification resource stopped; inspect server.log')
        result = re.search(terminal, server)
        if result and result[1] != 'completed':
            raise RuntimeError(f'native authority {result[1]}; inspect server.log')
        return result
    park.wait_until(outcome, 35,
                    'native authoritative outcome', processes)
    park.wait_until(lambda: 'NATIVE_AUTHORITY_COMPLETED' in park.read(run / 'client0.log'), 15,
                    'complete client prediction reconciliation', processes)
    proof = inspect_attempt(park.read(run / 'server.log'), park.read(run / 'client0.log'), actor)
    phases.append({'name': 'native stock scoring', **proof})
    if not proof['ok']:
        raise RuntimeError('; '.join(proof['failures']))
    offset = len(park.read(run / 'server.log'))
    command('command native_verify_return')
    admission = wait_admission(run, actor, '0', offset, processes)
    phases.append({'name': 'public return', 'admission': admission, 'server': status(expected)})


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--bin-dir', type=Path, default=park.REPO / 'target' / 'debug')
    parser.add_argument('--impaired', action='store_true', help='seeded real loopback UDP:25–75ms per direction,1%% loss and reordering; not WAN')
    args = parser.parse_args(argv)
    args.assets, args.output, args.bin_dir = (p.resolve() for p in (args.assets, args.output, args.bin_dir))
    if not args.assets.is_dir():
        parser.error('--assets must be a prepared owned asset directory')
    if args.output.exists() and any(args.output.iterdir()):
        parser.error('--output must be absent or empty')
    args.output.mkdir(parents=True, exist_ok=True)
    args.seconds, args.interval, args.timeout, args.overview = 65, 5, 100, True
    args.server_config = {'native_authority': {'executable': str(native_executable(args.bin_dir)),
                                              'assets': str(args.assets), 'max_workers': 1}}
    if args.impaired:
        args.endpoint_factory = lambda endpoint, index: ImpairedEndpoint(endpoint, 20261004 + index)
    result = park.run_scenario(args, 'native-scoring', prepare=prepare_fixture, drive=drive_phases)
    return 0 if result['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
