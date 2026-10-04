#!/usr/bin/env python3
"""Verify a native course using mapped controls and server-derived results.

Requires fresh matching game/server binaries and locally owned prepared assets.
Only copied resources change. No score, body pose, velocity or in-attempt travel
is injected: the client mounts, pulses the native push action, then brakes.
"""
from __future__ import annotations

import argparse
import json
import math
import re
from pathlib import Path

try:
    from tools import verify_resource_park as park
except ModuleNotFoundError:
    import verify_resource_park as park


def inspect_attempt(server, client, actor):
    failures = []
    # Engine command tracing and resource flush both print sdk.log output.
    # Keep the flushed observation once while retaining native physics logs.
    if 'RESOURCE_LOG verification-course:' in client:
        client = '\n'.join(line for line in client.splitlines() if 'Lua [verification-course]:' not in line)
    escaped = re.escape(actor)
    starts = re.findall(rf'COURSE_VERIFY_STARTED actor={escaped} epoch=([1-9]\d*)', server)
    epoch = starts[0] if len(starts) == 1 else None
    completed = re.findall(rf'COURSE_VERIFY_RESULT kind=completed actor={escaped} score=(\d+) elapsed_ms=(\d+) checkpoints=(\d+) pickups=(\d+) contacts=(\d+) rules=([^\s]+)', server)
    result = completed[0] if len(completed) == 1 else None
    if not epoch or not result:
        failures.append('exactly one authoritative start and completion are required')
    if result and (tuple(map(int, (result[0], result[2], result[3], result[4]))) != (175, 3, 1, 0)
                   or not 0 < int(result[1]) <= 25000 or result[5] != 'course-v1'):
        failures.append('host result has incorrect awards, timing or verified rules')
    if re.search(rf'COURSE_VERIFY_RESULT kind=(?:rejected|cancelled|error) actor={escaped}(?:\s|$)', server):
        failures.append('the host rejected or cancelled this attempt')
    running = client.find(f'COURSE_VERIFY_RUNNING actor={actor}')
    finished = client.find(f'COURSE_VERIFY_FINISHED actor={actor}')
    if running < 0 or finished < running:
        failures.append('native input start and completion observations are missing')
    attempt = client[running:finished] if running >= 0 and finished >= running else client
    resets = attempt.count('player teleport spawn ')
    if resets:
        failures.append('native reconciliation or travel interrupted the continuous attempt')
    samples = []
    for raw, onboard, state in re.findall(rf'COURSE_VERIFY_NATIVE actor={escaped} position=([^\s]+) on_board=(true|false) state=([^\s]+)', attempt):
        try:
            position = [float(value) for value in raw.split(',')]
        except ValueError:
            continue
        if len(position) == 3 and all(math.isfinite(value) for value in position):
            sample = {'position': position, 'on_board': onboard == 'true', 'state': state}
            if not samples or samples[-1] != sample:
                samples.append(sample)
    progress = max((s['position'][2] for s in samples), default=0) - min((s['position'][2] for s in samples), default=0)
    if len(samples) < 6 or progress < 5 or not all(s['on_board'] for s in samples):
        failures.append('native observations must show continuous onboard movement through the route')
    return {'ok': not failures, 'failures': failures, 'actor': actor,
            'movement_epoch': epoch,
            'completion': dict(zip(('score', 'elapsed_ms', 'checkpoints', 'pickups', 'contacts', 'rules'), result)) if result else None,
            'native_forward_distance': progress, 'native_resets_during_run': resets, 'native_samples': samples}


def prepare_fixture(root, selected):
    package = root / 'verification-course'
    package.mkdir()
    (package / 'resource.json').write_text(json.dumps({
        'format': 1, 'api': 1, 'id': 'verification-course', 'version': '1.0.0', 'language': 'lua',
        'client_scripts': ['client.lua'], 'server_scripts': ['server.lua'],
        'dependencies': {'community-park': '1.0.0'},
        'capabilities': ['resource.competition', 'resource.commands', 'resource.state', 'resource.events',
                         'resource.network', 'engine.input', 'engine.ui']}))
    selected.append('verification-course')
    (package / 'server.lua').write_text('''
local actor, stage, observed = nil, "idle", 0
local function publish(value)
 stage=value
 resource.state.set("drive",{actor=actor,stage=stage})
end
resource.competition.submit({kind="define",course={name="native_push",instance="0",
 checkpoints={{position={-8,0.2,2},radius=0.9,points=50},
              {position={-8,0.2,4},radius=0.9,points=50},
              {position={-8,0.2,6},radius=0.9,points=50}},
 pickups={{position={-8,0.2,3},radius=0.9,points=25}},
 limits={max_speed=15,max_acceleration=60,max_gap_ms=500,max_airborne_ms=2500}}})
resource.command("course_verify_mount","course.verify",function(args)
 assert(args[1] and not actor,"one attempt per verification process")
 for _,p in ipairs(resource.players()) do
  if p.id==args[1] and p.instance=="0" then actor=p.id;publish("mount");return end
 end
 error("select an admitted public player")
end)
resource.command("course_verify_start","course.verify",function()
 assert(actor and stage=="mount","mount the chosen native player before starting")
 resource.competition.submit({kind="start",name="native_push",player=actor})
end)
resource.on("competition_result",function(result)
 if result.operation=="define" then assert(result.ok,result.error);return end
 if result.operation=="start" then
  if result.ok then
   sdk.log("COURSE_VERIFY_STARTED actor="..actor.." epoch="..result.value.movement_epoch)
   publish("run")
  else
   sdk.log("COURSE_VERIFY_RESULT kind=error actor="..tostring(actor).." reason="..tostring(result.error))
   publish("finished")
  end
 elseif result.kind then
  if result.kind=="completed" then
   sdk.log("COURSE_VERIFY_RESULT kind=completed actor="..result.player.." score="..result.score..
    " elapsed_ms="..result.elapsed_ms.." checkpoints="..result.checkpoints.." pickups="..result.pickups..
    " contacts="..result.contacts.." rules="..result.verified_rules)
  else sdk.log("COURSE_VERIFY_RESULT kind="..result.kind.." actor="..result.player.." reason="..tostring(result.reason)) end
  publish("finished")
 end
end)
return {on_fixed_update=function(context)
 if stage~="run" then return end
 observed=observed+context.dt
 if observed<0.2 then return end
 observed=0
 for _,p in ipairs(resource.players()) do
  if p.id==actor and p.position then sdk.log("COURSE_VERIFY_OBSERVED actor="..actor.." position="..table.concat(p.position,",")) end
 end
end}
''')
    (package / 'client.lua').write_text('''
local stage, elapsed, observed, mounted, running = "idle", 0, 0, false, false
local function release()
 for _,action in ipairs({79,80,81}) do sdk.input.override_action(action,nil) end
end
local function sample(id,player)
 sdk.log("COURSE_VERIFY_NATIVE actor="..id.." position="..table.concat(player.position,",")..
  " on_board="..tostring(player.on_board).." state="..tostring(player.state))
end
return {
 on_load=function() sdk.log("COURSE_VERIFY_LOCAL actor="..sdk.net.info().local_id) end,
 on_update=function(context)
  local drive=resource.state.get("drive")
  local id=sdk.net.info().local_id
  if not drive or drive.actor~=id then return end
  local player=sdk.player.read()
  if stage~=drive.stage then
   release();stage=drive.stage;elapsed=0;observed=0
   if stage=="finished" then sample(id,player);sdk.log("COURSE_VERIFY_FINISHED actor="..id) end
  end
  elapsed=elapsed+context.dt;observed=observed+context.dt
  if stage=="mount" then
   if player.on_board then
    sdk.input.override_action(79,nil)
    if not mounted then mounted=true;sdk.log("COURSE_VERIFY_MOUNTED actor="..id) end
   else sdk.input.override_action(79,elapsed%1.2<0.1 and 1 or 0) end
  elseif stage=="run" then
   -- The normal Start operation establishes a fresh server spawn. Let that
   -- one native reset settle before applying ordinary push controls.
   if elapsed>0.8 then
    if not running then running=true;sdk.log("COURSE_VERIFY_RUNNING actor="..id) end
    sdk.input.override_action(80,elapsed%0.8<0.5 and 1 or 0)
    if observed>=0.2 then
     observed=0
     sample(id,player)
    end
   end
  elseif stage=="finished" then sdk.input.override_action(81,1) end
  sdk.ui.text("course_proof","Native course proof: "..stage)
 end,
 on_unload=release
}
''')
    (root / 'verification-view' / 'client.lua').write_text(
        'return {on_update=function() sdk.camera.set({-16,7,-8},{-8,0.8,4}) end}')


def drive_phases(run, command, status, expected, processes, phases):
    pattern = r'COURSE_VERIFY_LOCAL actor=([1-9]\d*)'
    park.wait_until(lambda: re.search(pattern, park.read(run / 'client0.log')), 8,
                    'native course driver identity', processes)
    actor = re.search(pattern, park.read(run / 'client0.log'))[1]
    command(f'command course_verify_mount {actor}')
    park.wait_until(lambda: f'COURSE_VERIFY_MOUNTED actor={actor}' in park.read(run / 'client0.log'), 10,
                    'normal native board mount', processes)
    server_offset = len(park.read(run / 'server.log'))
    client_offset = len(park.read(run / 'client0.log'))
    command('command course_verify_start')
    terminal = rf'COURSE_VERIFY_RESULT kind=(?:completed|rejected|cancelled|error) actor={actor}(?:\s|$)'
    park.wait_until(lambda: re.search(terminal, park.read(run / 'server.log')[server_offset:]), 25,
                    'authoritative course terminal result', processes)
    park.wait_until(lambda: f'COURSE_VERIFY_FINISHED actor={actor}' in park.read(run / 'client0.log')[client_offset:], 5,
                    'client observes terminal result and releases pushing', processes)
    proof = inspect_attempt(park.read(run / 'server.log')[server_offset:], park.read(run / 'client0.log')[client_offset:], actor)
    phases.append({'name': 'native course attempt', **proof})
    if not proof['ok']:
        raise RuntimeError('; '.join(proof['failures']))
    captures = []
    for index in range(2):
        numbers = re.findall(r'GAME_VERIFY_PHASE (\d+)', park.read(run / f'client{index}.log'))
        number = max(map(int, numbers), default=0) + 1
        image = run / f'client{index}.phase{number:02}.png'
        park.wait_until(lambda: image.is_file() and image.with_suffix('.input.txt').is_file(), 10,
                        'completed native course capture', processes)
        report = park.inspect_capture(park.read(run / f'client{index}.log'), park.read(image.with_suffix('.input.txt')),
                                     0, True, 'initial', periodic=True, peer_count=1, playing=True)
        captures.append({'client': index, 'image': str(image), 'report': report})
    phases.append({'name': 'native course completed', 'server': status(expected), 'captures': captures})
    if not all(item['report']['ok'] for item in captures):
        raise RuntimeError('completed native course capture failed')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--bin-dir', type=Path, default=park.REPO / 'target' / 'debug')
    args = parser.parse_args(argv)
    args.assets, args.output, args.bin_dir = (p.resolve() for p in (args.assets, args.output, args.bin_dir))
    if not args.assets.is_dir():
        parser.error('--assets must be a prepared local asset directory')
    if args.output.exists() and any(args.output.iterdir()):
        parser.error('--output must be absent or empty')
    args.output.mkdir(parents=True, exist_ok=True)
    args.seconds, args.interval, args.timeout, args.overview = 40, 5, 100, True
    result = park.run_scenario(args, 'native-course', prepare=prepare_fixture, drive=drive_phases)
    return 0 if result['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
