#!/usr/bin/env python3
"""Capture imported poses, attachment axes and retirement using ordinary SDK calls.

Requires the built game/server and locally owned prepared assets. This modifies
only copied resources beneath --output. PNG review remains a required separate
step; marker logs alone are not visual animation evidence.
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


def assert_animation_counts(native, phase):
    expected=(0,0,0,0,0) if phase=='stopped' else (1,2 if phase.startswith('pose_') else 0,2,2,0)
    match=re.search(r'^Resource animation: banks:(\d+) layers:(\d+) appearances:(\d+) attachments:(\d+) pending:(\d+)$',
                    native,re.M)
    actual=tuple(map(int,match.groups())) if match else None
    if actual!=expected:
        raise RuntimeError(f'{phase}: native animation counts expected {expected}, got {actual}')


def prepare_fixture(root, selected, hat_axis='x'):
    package = root / 'presentation-demo'
    manifest_path = package / 'resource.json'
    manifest = json.loads(manifest_path.read_text())
    if 'resource.commands' not in manifest['capabilities']:
        manifest['capabilities'].append('resource.commands')
    manifest_path.write_text(json.dumps(manifest))
    bank_path = package / 'clips.json'
    bank = json.loads(bank_path.read_text())
    bank['bones'].append({'name':'RIGHTFOREARM','parent':'RIGHTARM'})
    for name, sign in [('proof_a',1),('proof_b',-1)]:
        bank['clips'].append({'name':name,'duration':600,'tracks':[
            {'bone':'HEAD','keys':[{'time':0,'translation':[0,sign*.10,0],
                                  'rotation':[0,0,sign*math.sin(.2),math.cos(.2)]}]},
            {'bone':'RIGHTFOREARM','keys':[{'time':0,
                                          'rotation':[0,0,sign*math.sin(.4),math.cos(.4)]}]}],
            'markers':[{'time':0,'name':name}]})
    bank_path.write_text(json.dumps(bank))

    server_path = package / 'server.lua'
    server_path.write_text('local callbacks=(function()\n'+server_path.read_text()+'\nend)()\n'+'''
resource.state.set("proof_phase","axis_z")
resource.command("presentation_proof","presentation.verify",function(args)
    local phase=args[1]
    assert(phase=="axis_x" or phase=="axis_y" or phase=="axis_z" or phase=="pose_a" or phase=="pose_b")
    resource.state.set("proof_phase",phase)
end)
return callbacks
''')
    client_path = package / 'client.lua'
    original = client_path.read_text()
    room_guard = 'if sender~="0" then return end'
    if original.count(room_guard) != 1:
        raise RuntimeError('presentation room handler changed; update the fixture instrumentation')
    original = original.replace(room_guard, room_guard+';proof_room=value.id')
    client_path.write_text('local proof_room\nlocal callbacks=(function()\n'+original+'\nend)()\n'+f'local pose_hat_axis="{hat_axis}"\n'+'''
local seen={}
local axes={
 x={translation={0.24,0,0},rotation={0,0.70710678,0,0.70710678}},
 y={translation={0,0.24,0},rotation={-0.70710678,0,0,0.70710678}},
 z={translation={0,0,0.24},rotation={0,0,0,1}}
}
local update=callbacks.on_update
callbacks.on_update=function(...)
 if update then update(...) end
 if not proof_room then return end
 local phase=resource.state.get("proof_phase") or "axis_z"
 local axis=string.sub(phase,1,5)=="axis_" and string.sub(phase,6) or pose_hat_axis
 local actors=resource.state.get("actors",{kind="instance",id=proof_room}) or {}
 for id,value in pairs(actors) do
  local signature=phase..":"..tostring(value.revision)
  if seen[id]~=signature then
   seen[id]=signature
   sdk.animation.submit({op="attach",key=id.."_hat",target=id,bone="HEAD",path="hat.glb",
      translation=axes[axis].translation,rotation=axes[axis].rotation})
   if phase=="pose_a" or phase=="pose_b" then
    sdk.animation.submit({op="play",key=id.."_move",bank="moves",clip=phase=="pose_a" and "proof_a" or "proof_b",
      target=id,looped=true,speed=0.05,fade_in=0,fade_out=0})
   else sdk.animation.submit({op="stop",key=id.."_move",fade_out=0}) end
   sdk.log("PRESENTATION_POSE "..phase.." actor="..id.." hat_axis="..axis)
  end
 end
 sdk.ui.text("proof","Presentation verification: "..phase.."; hat local "..axis)
end
return callbacks
''')
    # The two server-owned admission pads are (-8,1,1) and (-10,1,1).
    # This stays a normal capability-granted camera resource in the copied set.
    (root / 'verification-view' / 'client.lua').write_text(
        'return {on_update=function()sdk.camera.set({-9,2.8,-3.2},{-9,1.8,1})end}')


def drive_phases(run, command, status, expected, processes, phases):
    park.wait_until(lambda: all(park.read(run/f'client{i}.log').count('_skin:ready')>=2 for i in range(2)),
                    15,'both imported appearances on both native clients',processes)
    def next_capture(index):
        # The log is emitted when the screenshot is requested, before the PNG
        # reaches disk. Count in-flight captures so old poses cannot pass.
        requested=re.findall(r'GAME_VERIFY_PHASE (\d+)',park.read(run/f'client{index}.log'))
        return max(map(int,requested),default=0)+1
    if any(next_capture(i)>2 for i in range(2)):
        raise RuntimeError('startup consumed the capture window; increase interval or shorten asset startup')
    for phase in ['axis_z','axis_x','axis_y','pose_a','pose_b','stopped']:
        offsets=[len(park.read(run/f'client{i}.log')) for i in range(2)]
        if phase=='stopped':
            command('stop presentation-demo')
            expected['presentation-demo']=False
            # Client replacement discards the retired runtime's command queue,
            # including on_unload logs. Require the new admission and verify
            # cleanup in the resulting native report and unobstructed image.
            park.wait_until(lambda: all('RESOURCE_ACTIVATED' in park.read(run/f'client{i}.log')[offsets[i]:]
                                       for i in range(2)),10,'presentation retirement and readmission',processes)
        else:
            if phase=='axis_z': offsets=[0,0]
            command(f'command presentation_proof {phase}')
            park.wait_until(lambda: all(f'PRESENTATION_POSE {phase} ' in park.read(run/f'client{i}.log')[offsets[i]:]
                                       for i in range(2)),10,f'{phase} observed on both clients',processes)
        snapshot=status(expected)
        captures=[next_capture(i) for i in range(2)]
        if any(capture>7 for capture in captures):
            raise RuntimeError(f'{phase}: periodic capture window exhausted')
        park.wait_until(lambda: all((run/f'client{i}.phase{captures[i]:02}.png').is_file()
                                   and (run/f'client{i}.phase{captures[i]:02}.input.txt').is_file() for i in range(2)),
                        12,f'{phase} periodic captures',processes)
        evidence=[]
        for i,capture in enumerate(captures):
            path=run/f'client{i}.phase{capture:02}.png'
            report=park.inspect_capture(park.read(run/f'client{i}.log'),park.read(path.with_suffix('.input.txt')),
                                        0,True,'initial',periodic=True,peer_count=1,playing=True)
            assert_animation_counts(report['native_report'],phase)
            evidence.append({'image':str(path),'report':report})
            if not report['ok']:
                raise RuntimeError(f'{phase} client{i}: '+ '; '.join(report['failures']))
        phases.append({'name':phase,'server':snapshot,'captures':evidence})
        print(json.dumps({'phase':phase,'captures':captures,'output':str(run)}),flush=True)


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--bin-dir',type=Path,default=park.REPO/'target'/'debug')
    parser.add_argument('--hat-axis',choices=('x','y','z'),default='x',help='axis used for the held-pose captures')
    args=parser.parse_args(argv)
    args.assets,args.output,args.bin_dir=(p.resolve() for p in (args.assets,args.output,args.bin_dir))
    if not args.assets.is_dir():parser.error('--assets must be a prepared local asset directory')
    if args.output.exists() and any(args.output.iterdir()):parser.error('--output must be absent or empty')
    args.output.mkdir(parents=True,exist_ok=True)
    args.seconds,args.interval,args.timeout,args.overview=40,5,100,True
    result=park.run_scenario(args,'presentation-poses',prepare=lambda root,selected:prepare_fixture(root,selected,args.hat_axis),drive=drive_phases)
    return 0 if result['ok'] else 1


if __name__=='__main__':
    raise SystemExit(main())
