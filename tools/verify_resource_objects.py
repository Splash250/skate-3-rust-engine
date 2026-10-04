"""Real-client collision and instance phases for verify_resource_park.

Only copied resources receive these diagnostic commands. The dynamic crate is
never moved by an entity command during this scenario; players approach it via
approved travel and ordinary native physics.
"""
from __future__ import annotations

import json
import math
import re
import time

try:
    from tools import verify_resource_park as park
except ModuleNotFoundError:
    import verify_resource_park as park


def prepare_fixture(root, selected):
    package = root / 'shared-objects'
    manifest_path = package / 'resource.json'
    manifest = json.loads(manifest_path.read_text())
    manifest['client_scripts'] = ['verify.lua']
    manifest_path.write_text(json.dumps(manifest))
    (package / 'verify.lua').write_text('return {on_load=function() sdk.log("OBJECT_VERIFY_LOCAL actor="..sdk.net.info().local_id) end}')
    path = package / 'server.lua'
    path.write_text('local callbacks=(function()\n' + path.read_text() + '\nend)()\n' + '''
local proof, elapsed, observed = nil, 0, 0
local function crate()
 for _, object in ipairs(resource.entities.all()) do
  if object.resource=="shared-objects" and object.key=="crate_0" then return object end
 end
end
local function position(value) return table.concat(value,",") end
resource.command("objects_verify_approach","objects.admin",function(args)
 local id,side=args[1],tonumber(args[2])
 assert(side==-1 or side==1,"approach side must be -1 or 1")
 local found=false
 for _, player in ipairs(resource.players()) do
  if player.id==id and player.instance=="0" then found=true end
 end
 assert(found,"approach requires an admitted public player")
 local object=assert(crate(),"public crate is unavailable")
 local p=object.position
 resource.teleport(id,{position={p[1]+side*1.15,0.25,p[3]},
  heading=-side*math.pi/2,velocity={-side*4,0,0},instance=0})
 proof={actor=id,entity=object.id};elapsed=0;observed=0
 sdk.log("OBJECT_VERIFY_APPROACH actor="..id.." entity="..object.id.." position="..position(p))
end)
resource.command("objects_verify_park","objects.admin",function(args)
 local id,slot=args[1],tonumber(args[2])
 assert(slot==0 or slot==1,"parking slot must be 0 or 1")
 for _, player in ipairs(resource.players()) do
  if player.id==id then
   resource.teleport(id,{position={-8-slot*2,1,1},heading=0,velocity={0,0,0},instance=0})
   return
  end
 end
 error("parking requires an admitted player")
end)
resource.command("objects_verify_players","objects.admin",function()
 for _, player in ipairs(resource.players()) do
  sdk.log("OBJECT_VERIFY_PLAYER actor="..player.id.." instance="..player.instance)
 end
end)
local load,update=callbacks.on_load,callbacks.on_fixed_update
callbacks.on_load=function(...)
 if load then load(...) end
 -- Leave room7 empty to make privacy and native collider retirement visible.
 for _, key in ipairs({"floor_7","crate_7","portal_7"}) do resource.entity({op="remove",key=key}) end
end
callbacks.on_fixed_update=function(context,...)
 if update then update(context,...) end
 if not proof then return end
 elapsed=elapsed+context.dt
 if elapsed-observed>=0.1 then
  observed=elapsed
  local object=crate()
  if object then sdk.log("OBJECT_VERIFY_SAMPLE actor="..proof.actor.." entity="..proof.entity.." position="..position(object.position)) end
 end
 if elapsed>=2.5 then proof=nil end
end
return callbacks
''')


def contact_evidence(log, actor, entity, origin):
    """Require a genuine server contact and horizontal motion, not floor hits."""
    contact = re.search(rf'RESOURCE_ENTITY_CONTACT entity={re.escape(entity)} actor={re.escape(actor)}(?:\s|$)', log)
    samples = []
    for raw in re.findall(rf'OBJECT_VERIFY_SAMPLE actor={re.escape(actor)} entity={re.escape(entity)} position=([^\s]+)', log):
        values = [float(value) for value in raw.split(',')]
        if len(values) == 3 and all(math.isfinite(value) for value in values):
            samples.append(values)
    displacement = max((math.hypot(p[0] - origin[0], p[2] - origin[2]) for p in samples), default=0.)
    return {'ok': bool(contact) and displacement >= .03, 'actor': actor, 'entity': entity,
            'server_contact': contact.group(0) if contact else None,
            'maximum_horizontal_displacement': displacement, 'samples': samples}


def drive_phases(run, command, status, expected, processes, phases):
    def local_actor(index):
        values = re.findall(r'OBJECT_VERIFY_LOCAL actor=(\d+)', park.read(run / f'client{index}.log'))
        return values[-1] if values and values[-1] != '0' else None

    park.wait_until(lambda: all(local_actor(i) for i in range(2)), 10,
                    'both authoritative client identities', processes)
    actors = [local_actor(i) for i in range(2)]

    def verify_instances(instances):
        offset = len(park.read(run / 'server.log'))
        requested = 0.
        def ready():
            nonlocal requested
            snapshot = park.read(run / 'server.log')[offset:]
            if all(f'OBJECT_VERIFY_PLAYER actor={actor} instance={instance}' in snapshot
                   for actor, instance in zip(actors, instances)):
                return True
            if time.monotonic() - requested >= .25:
                command('command objects_verify_players')
                requested = time.monotonic()
            return False
        park.wait_until(ready, 5, 'authoritative player instances', processes)

    def capture(name, private=None, contact=None):
        next_captures = []
        for index in range(2):
            numbers = [int(n) for n in re.findall(r'GAME_VERIFY_PHASE (\d+)', park.read(run / f'client{index}.log'))]
            next_captures.append(max(numbers, default=0) + 1)
        if max(next_captures) > 8:
            raise RuntimeError('interaction phases exhausted the bounded periodic capture window')
        evidence = []
        for index, number in enumerate(next_captures):
            image = run / f'client{index}.phase{number:02}.png'
            native = image.with_suffix('.input.txt')
            park.wait_until(lambda: image.is_file() and native.is_file(), 12,
                            name + f' client{index} capture', processes)
            report = park.inspect_capture(park.read(run / f'client{index}.log'), park.read(native), 0, True,
                'initial', periodic=True, object_count=0 if private == index else 3,
                peer_count=0 if private is not None else 1, playing=True)
            if contact and contact[0] == index:
                matches = re.search(r'^Shared entity observed contact frames: (.+)$', park.read(native), re.M)
                counts = json.loads(matches[1]) if matches else {}
                if counts.get(contact[1], 0) < 1:
                    report['failures'].append('this native client never solved a contact against the moving crate')
                    report['ok'] = False
            evidence.append({'client': index, 'image': str(image), 'report': report})
        phases.append({'name': name, 'server': status(expected), 'captures': evidence})
        if not all(item['report']['ok'] for item in evidence):
            raise RuntimeError(name + ': native interaction/privacy assertions failed')

    capture('admitted')
    for index, side in enumerate([-1, 1]):
        offset = len(park.read(run / 'server.log'))
        command(f'command objects_verify_approach {actors[index]} {side}')
        pattern = rf'OBJECT_VERIFY_APPROACH actor={actors[index]} entity=(\d+) position=([^\s]+)'
        park.wait_until(lambda: re.search(pattern, park.read(run / 'server.log')[offset:]), 5,
                        'trusted crate approach approval', processes)
        match = re.search(pattern, park.read(run / 'server.log')[offset:])
        entity, origin = match[1], [float(value) for value in match[2].split(',')]
        proof = lambda: contact_evidence(park.read(run / 'server.log')[offset:], actors[index], entity, origin)
        park.wait_until(lambda: proof()['ok'], 5, f'actor{index} real server crate contact and displacement', processes)
        phases.append({'name': f'actor{index} server contact', **proof()})
        capture(f'actor{index} native crate contact', contact=(index, entity))
        # Distinct trusted pads prevent unrelated player/player recovery loops.
        command(f'command objects_verify_park {actors[index]} {index}')

    offsets = [len(park.read(run / f'client{i}.log')) for i in range(2)]
    command(f'command objects_room {actors[1]} 7')
    park.wait_until(lambda: 'RESOURCE_ACTIVATED' in park.read(run / 'client1.log')[offsets[1]:], 10,
                    'private instance readmission', processes)
    verify_instances([0, 7])
    capture('isolated empty instance7', private=1)
    offset = len(park.read(run / 'client1.log'))
    command(f'command objects_room {actors[1]} 0')
    park.wait_until(lambda: 'RESOURCE_ACTIVATED' in park.read(run / 'client1.log')[offset:], 10,
                    'public instance return', processes)
    verify_instances([0, 0])
    capture('public instance restored')
