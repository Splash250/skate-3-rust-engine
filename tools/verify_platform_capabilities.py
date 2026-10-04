#!/usr/bin/env python3
"""Linux native two-client platform acceptance using owned local assets.

Real X11 keyboard events drive the dedicated browser. Private assets, endpoint
books, grants, captures, traces and server stores remain in the supplied output.
This is automated native acceptance, not physical controller/audio acceptance.
"""
from __future__ import annotations
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
try:
    from tools import resource_park, verify_resource_park as park
except ModuleNotFoundError:
    import resource_park
    import verify_resource_park as park

RESOURCES = ['platform-profiles', 'platform-crews', 'platform-rounds',
             'platform-map-vote', 'platform-leaderboards', 'platform-tournaments',
             'community-park', 'creator-park']


class Keyboard:
    """Target only the acceptance process's X11 window, never a title match."""
    def __init__(self):
        self.x = ctypes.CDLL('libX11.so.6')
        self.xt = ctypes.CDLL('libXtst.so.6')
        self.x.XOpenDisplay.restype = ctypes.c_void_p
        self.display = self.x.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError('An accessible X11/XWayland display is required for browser keyboard acceptance')
        D, W = ctypes.c_void_p, ctypes.c_ulong
        self.x.XDefaultRootWindow.argtypes = [D]; self.x.XDefaultRootWindow.restype = W
        self.x.XInternAtom.argtypes = [D, ctypes.c_char_p, ctypes.c_int]; self.x.XInternAtom.restype = W
        self.x.XQueryTree.argtypes = [D, W, ctypes.POINTER(W), ctypes.POINTER(W), ctypes.POINTER(ctypes.POINTER(W)), ctypes.POINTER(ctypes.c_uint)]
        self.x.XGetWindowProperty.argtypes = [D,W,W,ctypes.c_long,ctypes.c_long,ctypes.c_int,W,ctypes.POINTER(W),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(W),ctypes.POINTER(W),ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte))]
        self.x.XFree.argtypes = [ctypes.c_void_p]
        self.x.XFetchName.argtypes = [D,W,ctypes.POINTER(ctypes.c_char_p)]
        self.x.XSetInputFocus.argtypes = [D,W,ctypes.c_int,W]
        self.x.XRaiseWindow.argtypes = [D,W]
        self.x.XKeysymToKeycode.argtypes = [D,W]; self.x.XKeysymToKeycode.restype = ctypes.c_uint
        self.x.XFlush.argtypes = [D]
        self.x.XCloseDisplay.argtypes = [D]
        self.xt.XTestFakeKeyEvent.argtypes = [D,ctypes.c_uint,ctypes.c_int,W]
        self.atom = self.x.XInternAtom(self.display,b'_NET_WM_PID',0)

    def window(self, pid):
        # The game's crash reporter runs the application in a child process.
        # Accept only this launched process and its current descendants.
        pids=set();pending=[pid]
        while pending:
            current=pending.pop()
            if current in pids:continue
            pids.add(current)
            try:pending.extend(int(value) for value in Path(f'/proc/{current}/task/{current}/children').read_text().split())
            except FileNotFoundError:pass
        def visit(window):
            actual,remaining,count=ctypes.c_ulong(),ctypes.c_ulong(),ctypes.c_ulong()
            fmt=ctypes.c_int(); data=ctypes.POINTER(ctypes.c_ubyte)()
            self.x.XGetWindowProperty(self.display,window,self.atom,0,1,0,0,ctypes.byref(actual),ctypes.byref(fmt),ctypes.byref(count),ctypes.byref(remaining),ctypes.byref(data))
            match=False
            if data:
                match=fmt.value==32 and count.value==1 and ctypes.cast(data,ctypes.POINTER(ctypes.c_ulong))[0] in pids
                self.x.XFree(data)
            if match:
                name=ctypes.c_char_p()
                self.x.XFetchName(self.display,window,ctypes.byref(name))
                visible_title=name.value or b''
                if name:self.x.XFree(name)
                if visible_title.startswith(b'Platform observer'):return window
            root,parent=ctypes.c_ulong(),ctypes.c_ulong(); children=ctypes.POINTER(ctypes.c_ulong)(); n=ctypes.c_uint()
            self.x.XQueryTree(self.display,window,ctypes.byref(root),ctypes.byref(parent),ctypes.byref(children),ctypes.byref(n))
            values=[children[i] for i in range(n.value)]
            if children:self.x.XFree(children)
            for child in values:
                found=visit(child)
                if found:return found
            return None
        return visit(self.x.XDefaultRootWindow(self.display))

    def key(self,pid,keysym):
        window=self.window(pid)
        if not window:raise RuntimeError('Acceptance client X11 window is unavailable')
        self.x.XRaiseWindow(self.display,window);self.x.XSetInputFocus(self.display,window,2,0)
        key=self.x.XKeysymToKeycode(self.display,keysym)
        self.xt.XTestFakeKeyEvent(self.display,key,1,0);self.x.XFlush(self.display);time.sleep(.15)
        self.xt.XTestFakeKeyEvent(self.display,key,0,0);self.x.XFlush(self.display);time.sleep(.2)

    def close(self):self.x.XCloseDisplay(self.display)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--bin-dir',type=Path,default=park.REPO/'target/debug')
    args=parser.parse_args()
    args.assets,args.output,args.bin_dir=(p.resolve() for p in (args.assets,args.output,args.bin_dir))
    if args.output.exists() and any(args.output.iterdir()):parser.error('output must be absent or empty')
    if not args.assets.is_dir():parser.error('owned prepared assets are required')
    args.output.mkdir(parents=True,exist_ok=True)
    root=args.output/'resources';root.mkdir()
    for rid in RESOURCES:shutil.copytree(park.REPO/'resources'/rid,root/rid)
    scene=resource_park.load(root/'creator-park/placements.json')
    before=(root/'creator-park/park.skate').read_bytes()
    resource_park.export(scene,root/'creator-park','creator-park')
    assert before==(root/'creator-park/park.skate').read_bytes(),'authored scene must reproduce checked-in map bytes'
    selected=[rid for rid in RESOURCES if rid!='community-park']
    grants={rid:json.loads((root/rid/'resource.json').read_text()).get('capabilities',[]) for rid in RESOURCES}
    config={'root':str(root),'storage':str(args.output/'data'),'ensure':selected,'grants':grants,
            'world_rotation':['creator-park','community-park']}
    (args.output/'server.json').write_text(json.dumps(config))
    (args.output/'operations.json').write_text(json.dumps({'name':'Local platform acceptance','map':'Creator Courtyard','mode':'Free skate'}))
    env=park.environment(args.bin_dir,40);env['SKATE_VERIFY_INTERVAL_SECONDS']='5'
    env.pop('WAYLAND_DISPLAY',None)
    env['WINIT_UNIX_BACKEND']='x11'
    handles=[];clients=[];server=None;keyboard=None
    result={'ok':False,'phases':[],'physical_devices':False}
    try:
        out=open(args.output/'server.log','w');handles.append(out)
        server=subprocess.Popen([str(args.bin_dir/'skate-server'),'--test-world','--bind','127.0.0.1:0','--resources',str(args.output/'server.json'),'--operations',str(args.output/'operations.json')],stdin=subprocess.PIPE,stdout=out,stderr=subprocess.STDOUT,text=True,env=env,cwd=park.REPO)
        park.wait_until(lambda:'Dedicated server listening on ' in park.read(args.output/'server.log'),30,'server start',[server])
        match=re.search(r'Dedicated server listening on (127\.0\.0\.1:\d+) \| session=(\d+)',park.read(args.output/'server.log'))
        endpoint,session=match.groups()
        def command(value):server.stdin.write(value+'\n');server.stdin.flush()
        def status():
            offset=len(park.read(args.output/'server.log'));command('resources')
            park.wait_until(lambda:'creator-park:' in park.read(args.output/'server.log')[offset:],5,'server statuses',[server])
            return park.read(args.output/'server.log')[offset:]
        for i in range(2):
            cache=args.output/f'cache{i}';cache.mkdir()
            (cache/'grants.json').write_text(json.dumps({f'udp://{endpoint}/{session}':grants}))
            book=args.output/f'endpoints{i}.json'
            book.write_text(json.dumps([{'endpoint':endpoint,'favorite':True,'last_joined':0,'account_profile':None}]))
            client_env={**env,'SKATE3_RESOURCE_CACHE':str(cache),'SKATE3_MODS':str(args.output/f'mods{i}'),'SKATE3_MOD_SETTINGS':str(args.output/f'settings{i}'),'SKATE3_DEDICATED_SERVERS':str(book)}
            commandline=[str(args.bin_dir/'skate3rust'),'--assets',str(args.assets),'--test-world','--player-title',f'Platform observer {i+1}','--verify',str(args.output/f'client{i}.png')]
            if i==0:commandline+=['--connect',endpoint]
            else:client_env['SKATE_VERIFY_MENU']='dedicated'
            out=open(args.output/f'client{i}.log','w');handles.append(out)
            client=subprocess.Popen(commandline,stdout=out,stderr=subprocess.STDOUT,env=client_env,cwd=park.REPO);clients.append(client)
            if i==0:park.wait_until(lambda:'RESOURCE_ACTIVATED' in park.read(args.output/'client0.log'),40,'first native admission',[server,client])
            else:
                keyboard=Keyboard()
                park.wait_until(lambda:keyboard.window(client.pid),30,'native browser window',[server,client])
                # Wait for the verification capture to prove the real menu is laid out.
                park.wait_until(lambda:(args.output/'client1.phase01.png').exists(),25,'browser menu capture',[server,client])
                for _ in range(3):keyboard.key(client.pid,0xff54) # Endpoint -> profile -> save -> refresh
                keyboard.key(client.pid,0xff0d)
                time.sleep(2.3)
                keyboard.key(client.pid,0xff54);keyboard.key(client.pid,0xff0d) # Join
        park.wait_until(lambda:all('RESOURCE_ACTIVATED' in park.read(args.output/f'client{i}.log') for i in range(2)),30,'browser+direct native admissions',[server,*clients])
        keyboard.key(clients[1].pid,0xff1b);keyboard.key(clients[1].pid,0xff1b)
        def phase(name,world,triangles,rails):
            captures=[]
            def ready():
                captures.clear()
                for i in range(2):
                    choices=[]
                    for path in sorted(args.output.glob(f'client{i}.phase*.input.txt')):
                        text=path.read_text()
                        if re.search(rf'^World: {re.escape(world)} generation=\d+ triangles={triangles} native_grind_primitives={rails}$',text,re.M) and 'remote_count:1' in text:
                            choices.append(path)
                    if not choices:return False
                    captures.append(str(choices[-1]))
                return True
            park.wait_until(ready,20,name,[server,*clients])
            result['phases'].append({'name':name,'captures':captures[:],'resources':status()})
        phase('authored world admitted','Creator Courtyard',76,3)
        command('setting platform-rounds duration_seconds 30')
        command('setting platform-rounds mode "tournament"')
        command('restart platform-rounds')
        command('setting platform-map-vote vote_seconds 5')
        command('command map_vote')
        phase('voted world replaces collider and rail generation','Community Practice Park',32,1)
        command('profile-export '+str(args.output/'resources.trace.json'))
        park.wait_until(lambda:(args.output/'resources.trace.json').is_file(),5,'portable trace',[server])
        trace=json.loads((args.output/'resources.trace.json').read_text())
        assert trace['traceEvents'],'actual profile spans are required'
        result['trace_events']=len(trace['traceEvents'])
        park.wait_until(lambda:all(c.poll() is not None for c in clients),50,'native final captures',[server])
        for i,client in enumerate(clients):
            text=park.read(args.output/f'client{i}.log')
            assert client.returncode==0 and 'GAME_VERIFY_OK' in text,text[-2000:]
            assert 'remote_count:1' in text,'two native clients never saw one another'
            for forbidden in ('panicked at','Resource activation failed','Dedicated resources stopped','MAP_TRANSITION_FAILED','Resource failure publication:'):
                assert forbidden not in text,forbidden
        assert json.loads((args.output/'endpoints1.json').read_text())[0]['last_joined']>0,'browser admission must persist recency'
        assert 'Resource stopped generation=' not in park.read(args.output/'server.log'),'a resource failed during integration'
        result['ok']=True
    except (OSError,RuntimeError,AssertionError,subprocess.SubprocessError) as error:
        result['error']=str(error)
    finally:
        for client in reversed(clients):park.stop(client)
        if server:park.stop(server,graceful=True)
        if keyboard:keyboard.close()
        for handle in handles:handle.close()
        (args.output/'results.json').write_text(json.dumps(result,indent=2))
    print(json.dumps(result,indent=2));return 0 if result['ok'] else 1


if __name__=='__main__':raise SystemExit(main())
