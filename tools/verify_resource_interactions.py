#!/usr/bin/env python3
"""Native Linux two-client RP interaction acceptance, using private local fixtures.

Runs the shipped resources with read-only layout/event instrumentation in copied
packages. All product actions use native X11 event injection into the actual
game windows. No resource can generate a shutter or microphone consent. The
ALSA input is a deterministic tone; this is not physical microphone/controller
acceptance. Captures, credentials, account stores and photos stay under --output.
"""
from __future__ import annotations
import argparse
import array
import ctypes
import json
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import ssl
import subprocess
import time
import threading
import urllib.request
import urllib.parse
try:
    from tools import verify_resource_park as park
    from tools.verify_platform_capabilities import Keyboard
except ModuleNotFoundError:
    import verify_resource_park as park
    from verify_platform_capabilities import Keyboard


class XImage(ctypes.Structure):
    _fields_=[('width',ctypes.c_int),('height',ctypes.c_int),('xoffset',ctypes.c_int),('format',ctypes.c_int),('data',ctypes.c_void_p),('byte_order',ctypes.c_int),('bitmap_unit',ctypes.c_int),('bitmap_bit_order',ctypes.c_int),('bitmap_pad',ctypes.c_int),('depth',ctypes.c_int),('bytes_per_line',ctypes.c_int),('bits_per_pixel',ctypes.c_int),('red_mask',ctypes.c_ulong),('green_mask',ctypes.c_ulong),('blue_mask',ctypes.c_ulong)]


class Controls(Keyboard):
    def __init__(self):
        super().__init__()
        D,W=ctypes.c_void_p,ctypes.c_ulong
        self.x.XGetGeometry.argtypes=[D,W,ctypes.POINTER(W),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_uint),ctypes.POINTER(ctypes.c_uint),ctypes.POINTER(ctypes.c_uint),ctypes.POINTER(ctypes.c_uint)]
        self.x.XTranslateCoordinates.argtypes=[D,W,W,ctypes.c_int,ctypes.c_int,ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(W)]
        self.xt.XTestFakeMotionEvent.argtypes=[D,ctypes.c_int,ctypes.c_int,ctypes.c_int,W]
        self.xt.XTestFakeButtonEvent.argtypes=[D,ctypes.c_uint,ctypes.c_int,W]
        self.x.XGetImage.argtypes=[D,W,ctypes.c_int,ctypes.c_int,ctypes.c_uint,ctypes.c_uint,ctypes.c_ulong,ctypes.c_int]
        self.x.XGetImage.restype=ctypes.POINTER(XImage)
        self.x.XDestroyImage.argtypes=[ctypes.POINTER(XImage)]
        self.x.XGetInputFocus.argtypes=[D,ctypes.POINTER(W),ctypes.POINTER(ctypes.c_int)]

    def focus(self,pid):
        window=self.window(pid)
        if not window:raise RuntimeError('Native acceptance game window disappeared')
        self.x.XRaiseWindow(self.display,window)
        self.x.XSetInputFocus(self.display,window,2,0)
        self.x.XFlush(self.display)
        time.sleep(.15)
        actual=ctypes.c_ulong();revert=ctypes.c_int()
        self.x.XGetInputFocus(self.display,ctypes.byref(actual),ctypes.byref(revert))
        if actual.value!=window:raise RuntimeError('Native X11 focus did not reach the intended game window')
        return window

    def key(self,pid,keysym,hold=.15):
        self.focus(pid)
        key=self.x.XKeysymToKeycode(self.display,keysym)
        self.xt.XTestFakeKeyEvent(self.display,key,1,0);self.x.XFlush(self.display);time.sleep(hold)
        self.xt.XTestFakeKeyEvent(self.display,key,0,0);self.x.XFlush(self.display);time.sleep(.25)

    def chord(self,pid,keysym):
        self.focus(pid)
        control=self.x.XKeysymToKeycode(self.display,0xffe3)
        self.xt.XTestFakeKeyEvent(self.display,control,1,0);self.x.XFlush(self.display)
        self.key(pid,keysym)
        self.xt.XTestFakeKeyEvent(self.display,control,0,0);self.x.XFlush(self.display);time.sleep(.3)

    def geometry(self,pid):
        window=self.focus(pid);root=ctypes.c_ulong();x=ctypes.c_int();y=ctypes.c_int()
        w,h,border,depth=(ctypes.c_uint() for _ in range(4))
        self.x.XGetGeometry(self.display,window,ctypes.byref(root),ctypes.byref(x),ctypes.byref(y),ctypes.byref(w),ctypes.byref(h),ctypes.byref(border),ctypes.byref(depth))
        child=ctypes.c_ulong()
        self.x.XTranslateCoordinates(self.display,window,self.x.XDefaultRootWindow(self.display),0,0,ctypes.byref(x),ctypes.byref(y),ctypes.byref(child))
        return x.value,y.value,w.value,h.value

    def screenshot(self,pid,path):
        from PIL import Image
        window=self.focus(pid);_,_,w,h=self.geometry(pid);time.sleep(.25)
        pointer=self.x.XGetImage(self.display,window,0,0,w,h,ctypes.c_ulong(-1).value,2)
        if not pointer:raise RuntimeError('X11 could not read the game window pixels')
        try:
            frame=pointer.contents
            if frame.bits_per_pixel!=32 or (frame.red_mask,frame.green_mask,frame.blue_mask)!=(0xff0000,0xff00,0xff):raise RuntimeError('Unsupported X11 game-window pixel format')
            data=ctypes.string_at(frame.data,frame.bytes_per_line*frame.height)
            Image.frombytes('RGB',(frame.width,frame.height),data,'raw','BGRX',frame.bytes_per_line).save(path)
        finally:self.x.XDestroyImage(pointer)

    def point(self,pid,x,y,surface,arrived):
        left,top,w,h=self.geometry(pid)
        width,height,anchor,offset,requested=surface
        scale=min(requested,(w-2*offset)/width,(h-2*offset)/height)
        if anchor=='center':ox,oy=(w-width*scale)/2,(h-height*scale)/2
        else:ox,oy=w-width*scale-offset,h-height*scale-offset
        print(f'VERIFY_NATIVE_POINTER pid={pid} window={self.window(pid)} surface=({x},{y}) desktop=({round(left+ox+x*scale)},{round(top+oy+y*scale)})',flush=True)
        # X11/winit caches the last motion separately from focus-enter cursor
        # events. A direct warp back to the last click coordinate may be deduped
        # after focus changes. Use a short physical-style path, verifying the
        # actual DOM cursor arrival at each point before the single click.
        for px,py in [(max(1,x-7),max(1,y-7)),(x,y)]:
            self.xt.XTestFakeMotionEvent(self.display,-1,round(left+ox+px*scale),round(top+oy+py*scale),0)
            self.x.XFlush(self.display)
            park.wait_until(lambda:arrived(px,py),5,'native cursor reaches rendered control')
        self.xt.XTestFakeButtonEvent(self.display,1,1,0);self.x.XFlush(self.display);time.sleep(.1)
        self.xt.XTestFakeButtonEvent(self.display,1,0,0);self.x.XFlush(self.display);time.sleep(.35)


LAYOUT_JS=r"""
for(const kind of ['mousemove','click'])document.addEventListener(kind,event=>{const e=event.target.closest?.('button,input,select,textarea')||event.target;skate.postMessage({action:'acceptance_pointer',kind,x:event.clientX,y:event.clientY,label:encodeURIComponent((e.getAttribute?.('aria-label')||e.textContent||e.id||'').trim().replace(/\s+/g,' ').slice(0,96))});},true);
setInterval(()=>{const entries=Array.from(document.querySelectorAll('button,input,select,textarea')).filter(e=>!e.disabled&&!e.closest('[hidden]')&&e.getClientRects().length).map(e=>{const r=e.getBoundingClientRect();let id=e.id||e.dataset.id||e.getAttribute('aria-label')||e.textContent.trim();if(e.textContent.trim()==='Save'&&e.closest('.field'))id='save:'+e.closest('.field').querySelector('input,select').id;return {id:encodeURIComponent(id.slice(0,128)),label:encodeURIComponent((e.getAttribute('aria-label')||e.textContent||e.id).trim().replace(/\s+/g,' ').slice(0,128)),x:r.x+r.width/2,y:r.y+r.height/2};}).filter(e=>e.x>=0&&e.x<innerWidth&&e.y>=0&&e.y<innerHeight);skate.postMessage({action:'acceptance_layout',entries});},1000);
"""
WRAPPER=r'''
local prior_event=callbacks.on_event
callbacks.on_event=function(value,...)
 if value.type=="browser" and value.event.kind=="message" and type(value.event.value)=="table" then
  local payload=value.event.value
  if payload.action=="acceptance_pointer" then
   sdk.log("VERIFY_DOM_POINTER key="..value.key.." kind="..tostring(payload.kind).." x="..tostring(payload.x).." y="..tostring(payload.y).." label="..tostring(payload.label));return
  end
  if payload.action=="acceptance_layout" then
   sdk.log("VERIFY_LAYOUT_BEGIN key="..value.key)
   for _,e in ipairs(payload.entries or {}) do sdk.log("VERIFY_ELEMENT key="..value.key.." id="..e.id.." label="..e.label.." x="..e.x.." y="..e.y) end
   sdk.log("VERIFY_LAYOUT_END key="..value.key)
   return
  end
  sdk.log("VERIFY_UI_ACTION key="..value.key.." action="..tostring(payload.action).." route="..tostring(payload.route).." operation="..tostring(payload.operation))
 end
 if prior_event then prior_event(value,...) end
end
return callbacks
'''


def instrument(root):
    policy=root/'interaction-policy/client.lua'
    policy_source=policy.read_text()
    policy_source=policy_source.replace('local function policy() sdk.ui.interaction_policy', 'local function policy() resource.send("__host_admin",{seq="native-forged",action={kind="settings_read",resource="phone-calls"}}); sdk.ui.interaction_policy')
    policy_source=policy_source.replace('function(payload,sender)\n', 'function(payload,sender)\n    if payload.seq=="native-forged" then sdk.log("VERIFY_ADMIN_FORGED ok="..tostring(payload.ok).." sender="..tostring(sender));return end\n',1)
    policy.write_text(policy_source)
    for rid in ['phone','master-menu','admin-dashboard','inventory-ui','call-diagnostics']:
        directory=root/rid
        if not directory.exists():continue
        script=directory/'client.lua';original=script.read_text()
        if rid=='phone':
            # Observe the production polling result. Calling this export from a
            # separate probe would renew the presenter lease and mask failures.
            before='local function snapshot() return resource.call("phone-calls","snapshot",{}) end'
            after='''local function snapshot()
 local call=resource.call("phone-calls","snapshot",{})
 local v=call.voice or {};local stats=v.stats or {}
 sdk.log("VERIFY_CALL state="..tostring(call.call and call.call.state).." incoming="..tostring(call.call and call.call.incoming))
 sdk.log("VERIFY_VOICE device="..tostring(v.device and v.device.state).." channel="..tostring(v.channel).." captured="..tostring(stats.captured_frames).." audible="..tostring(stats.audible_samples).." muted="..tostring(v.muted).." deafened="..tostring(v.deafened))
 return call
end'''
            if original.count(before)!=1:raise RuntimeError('Production phone snapshot observer needs updating')
            original=original.replace(before,after,1)
        if rid=='master-menu':original=original.replace('if not ok then entries={}', 'if not ok then sdk.log("VERIFY_FILTER_ERROR "..tostring(entries)); entries={}')
        if rid=='inventory-ui':original=original.replace('sdk.log(payload.ok and', 'sdk.log("VERIFY_INVENTORY ok="..tostring(payload.ok).." coins="..tostring(payload.coins).." deck="..tostring(payload.owned and payload.owned.deck_blue));sdk.log(payload.ok and')
        if rid=='call-diagnostics':original=original.replace('    if value.ok then last_authorized', '    sdk.log("VERIFY_DIAGNOSTICS operation="..ticket.kind.." ok="..tostring(value.ok).." passed="..tostring(value.value and value.value.ok).." checked="..tostring(value.value and value.value.checked))\n    if value.ok then last_authorized')
        # Add a packaged instrumentation script to this private copy, retaining
        # the same immutable allowlist and isolated message bridge as production.
        original=re.sub(r'files\s*=\s*\{','files={"acceptance.js",',original,count=1)
        script.write_text('local callbacks=(function()\n'+original+'\nend)()\n'+WRAPPER)
        html=directory/'index.html';html.write_text(html.read_text().replace('</body>','<script src="acceptance.js"></script></body>'))
        (directory/'acceptance.js').write_text(LAYOUT_JS)
        manifest=json.loads((directory/'resource.json').read_text());manifest.setdefault('files',[]).append('acceptance.js')
        (directory/'resource.json').write_text(json.dumps(manifest))


def private_json(path,value):
    path.write_text(json.dumps(value));path.chmod(0o600)


def audio_fixture(output,index,workers):
    source=output/f'input{index}.raw'
    os.mkfifo(source,0o600)
    stopped=threading.Event();workers.append(stopped)
    def produce():
        fd=None;n=0
        try:
            while not stopped.is_set():
                try:fd=os.open(source,os.O_WRONLY|os.O_NONBLOCK);break
                except OSError:stopped.wait(.05)
            while fd is not None and not stopped.is_set():
                samples=array.array('f',(math.sin((n+j)*2*math.pi*(440+index*110)/48000)*.3 for j in range(960)))
                try:os.write(fd,samples.tobytes());n+=960
                except BlockingIOError:pass
                except BrokenPipeError:break
                stopped.wait(.02)
        finally:
            if fd is not None:os.close(fd)
    threading.Thread(target=produce,daemon=True).start()
    config=output/f'alsa{index}.conf'
    config.write_text(f'pcm.null {{ type null }}\npcm.capture {{ type file slave.pcm "null" file "/dev/null" infile "{source}" format "raw" }}\npcm.!default {{ type asym capture.pcm "capture" playback.pcm "null" }}\n')
    return config


def layout(log,key):
    text=park.read(log)
    opens=list(re.finditer(r'RESOURCE_BROWSER_OPEN resource=\S+ key='+re.escape(key)+r'\b',text))
    closes=list(re.finditer(r'RESOURCE_BROWSER_CLOSED resource=\S+ key='+re.escape(key)+r'\b',text))
    latest=opens[-1].start() if opens else -1
    if closes and closes[-1].start()>latest:return []
    starts=[m for m in re.finditer(r'VERIFY_LAYOUT_BEGIN key='+re.escape(key)+r'\b',text) if m.start()>latest]
    for start in reversed(starts):
        end=text.find('VERIFY_LAYOUT_END key='+key,start.end())
        if end<0:continue
        return [dict(key=m[1],id=urllib.parse.unquote(m[2]),label=urllib.parse.unquote(m[3]),x=float(m[4]),y=float(m[5])) for m in re.finditer(r'VERIFY_ELEMENT key=(\S+) id=(\S*) label=(\S*) x=([\d.e+-]+) y=([\d.e+-]+)',text[start.end():end])]
    return []


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--bin-dir',type=Path,default=park.REPO/'target/debug')
    args=parser.parse_args()
    args.assets,args.output,args.bin_dir=(p.resolve() for p in (args.assets,args.output,args.bin_dir))
    if args.output.exists() and any(args.output.iterdir()):parser.error('output must be absent or empty')
    if not args.assets.is_dir():parser.error('prepared owned assets are required')
    args.output.mkdir(parents=True,mode=0o700);args.output.chmod(0o700)
    result={'ok':False,'phases':[],'physical_audio':False,'physical_controller':False,'input':'native X11 injected keyboard/pointer','voice':'deterministic ALSA capture/null playback'}
    handles=[];clients=[];workers=[];server=None;controls=None
    try:
        root=args.output/'resources';root.mkdir()
        config=json.loads((park.REPO/'resources/rp-server.json').read_text())
        # Dependencies are copied from the explicit recipe grants, then grants
        # are read from each current shipped manifest to avoid a stale fixture.
        for rid in config['grants']:shutil.copytree(park.REPO/'resources'/rid,root/rid)
        instrument(root)
        grants={rid:json.loads((root/rid/'resource.json').read_text()).get('capabilities',[]) for rid in config['grants']}
        config.update(root=str(root),storage=str(args.output/'storage'),grants=grants)
        private_json(args.output/'server.json',config)
        accountdir=args.output/'accounts'
        password=secrets.token_urlsafe(32)
        subprocess.run([str(args.bin_dir/'skate-account'),'init',str(accountdir),'administrator'],input=password+'\n',text=True,check=True,stdout=subprocess.DEVNULL)
        private_json(args.output/'accounts.json',{'database':str(accountdir/'accounts.sqlite3'),'certificate':str(accountdir/'certificate.pem'),'key':str(accountdir/'private-key.pem'),'bind':'127.0.0.1:0'})
        env=park.environment(args.bin_dir,240);env['SKATE_VERIFY_INTERVAL_SECONDS']='3';env.pop('WAYLAND_DISPLAY',None);env['WINIT_UNIX_BACKEND']='x11';env['GDK_BACKEND']='x11'
        output=open(args.output/'server.log','w');handles.append(output)
        server=subprocess.Popen([str(args.bin_dir/'skate-server'),'--test-world','--bind','127.0.0.1:0','--resources',str(args.output/'server.json'),'--accounts',str(args.output/'accounts.json')],stdin=subprocess.PIPE,stdout=output,stderr=subprocess.STDOUT,text=True,env=env,cwd=park.REPO)
        park.wait_until(lambda:'HTTPS listening on ' in park.read(args.output/'server.log'),30,'authenticated server startup',[server])
        text=park.read(args.output/'server.log')
        endpoint,session=re.search(r'Dedicated server listening on (127\.0\.0\.1:\d+) \| session=(\d+)',text).groups()
        authority='https://'+re.search(r'HTTPS listening on (127\.0\.0\.1:\d+)',text)[1]
        context=ssl.create_default_context(cafile=str(accountdir/'certificate.pem'))
        def request(path,data=None,token=None):
            headers={'Content-Type':'application/json'}
            if token:headers['Authorization']='Bearer '+token
            req=urllib.request.Request(authority+path,data=None if data is None else json.dumps(data).encode(),headers=headers)
            with urllib.request.urlopen(req,context=context,timeout=5) as reply:return json.load(reply)
        token=request('/v1/login',{'username':'administrator','password':password})['token']
        request('/v1/admin/accounts',{'username':'player','password':password},token)
        controls=Controls()
        for i,username in enumerate(['administrator','player']):
            cache=args.output/f'cache{i}';cache.mkdir()
            private_json(cache/'grants.json',{f'udp://{endpoint}/{session}':grants})
            passwd=args.output/f'password{i}';passwd.write_text(password);passwd.chmod(0o600)
            private_json(args.output/f'account{i}.json',{'endpoint':authority,'ca_certificate':str(accountdir/'certificate.pem'),'username':username,'password_file':str(passwd)})
            xdg=args.output/f'xdg{i}';xdg.mkdir();desktop=args.output/f'desktop{i}';desktop.mkdir()
            (xdg/'user-dirs.dirs').write_text(f'XDG_DESKTOP_DIR="{desktop}"\n')
            client_env={**env,'XDG_CONFIG_HOME':str(xdg),'ALSA_CONFIG_PATH':str(audio_fixture(args.output,i,workers)),'SKATE3_RESOURCE_CACHE':str(cache),'SKATE3_MODS':str(args.output/f'mods{i}'),'SKATE3_MOD_SETTINGS':str(args.output/f'settings{i}')}
            log=open(args.output/f'client{i}.log','w');handles.append(log)
            client=subprocess.Popen([str(args.bin_dir/'skate3rust'),'--assets',str(args.assets),'--test-world','--player-title',f'Platform observer {i+1}','--connect',endpoint,'--account-config',str(args.output/f'account{i}.json'),'--voice'],stdout=log,stderr=subprocess.STDOUT,env=client_env,cwd=park.REPO);clients.append(client)
            park.wait_until(lambda:'RESOURCE_ACTIVATED' in park.read(args.output/f'client{i}.log'),45,f'client{i} admission',[server,client])
        park.wait_until(lambda:all('MAP_TRANSITION_COMMITTED' in park.read(args.output/f'client{i}.log') for i in range(2)),60,'both clients enter Creator Courtyard',[server,*clients])
        time.sleep(2)
        def command(value):server.stdin.write(value+'\n');server.stdin.flush()
        surfaces={'phone':(390,760,'bottom_right',24,1),'menu':(520,680,'center',0,1),'admin':(1120,760,'center',0,1),'inventory':(900,640,'center',0,1),'diagnostics':(960,660,'center',0,1)}
        def elements(index,key):return layout(args.output/f'client{index}.log',key)
        def click(index,key,identity):
            # A focus transition intentionally waits for a neutral host frame.
            # Read a newly rendered layout after focus, never click stale
            # background coordinates while the ownership release gate is armed.
            log=args.output/f'client{index}.log'
            observed=park.read(log).count('VERIFY_LAYOUT_BEGIN key='+key)
            controls.focus(clients[index].pid)
            park.wait_until(lambda:park.read(log).count('VERIFY_LAYOUT_BEGIN key='+key)>observed,5,f'focused {key} layout',[server,*clients])
            found=[]
            def ready():
                found[:]=[e for e in elements(index,key) if identity(e)]
                return bool(found)
            park.wait_until(ready,12,f'visible {key} control',[server,*clients])
            e=found[0]
            def arrived(x,y):
                points=re.findall(r'VERIFY_DOM_POINTER key='+re.escape(key)+r' kind=mousemove x=([\d.e+-]+) y=([\d.e+-]+)',park.read(log))
                return bool(points) and abs(float(points[-1][0])-x)<2 and abs(float(points[-1][1])-y)<2
            controls.point(clients[index].pid,e['x'],e['y'],surfaces[key],arrived);return e
        def capture(name):
            time.sleep(.5)
            saved=[]
            for i in range(2):
                target=args.output/f'{name}-client{i}.png';controls.screenshot(clients[i].pid,target);saved.append(str(target))
            memory={}
            for i in range(2):
                for pid in re.findall(r'RESOURCE_BROWSER_OPEN [^\n]*pid=(\d+)',park.read(args.output/f'client{i}.log')):
                    try:
                        group=next(line[4:] for line in Path(f'/proc/{pid}/cgroup').read_text().splitlines() if line.startswith('0::/'))
                        memory[pid]=int((Path('/sys/fs/cgroup')/group/'memory.current').read_text())
                    except (OSError,StopIteration,ValueError):pass
            result['phases'].append({'name':name,'captures':saved,'browser_tree_memory_bytes':memory})
        controls.key(clients[0].pid,0xffbf,.75) # F2 master menu
        park.wait_until(lambda:elements(0,'menu'),15,'composited master menu',[server,*clients])
        capture('master-menu')
        # Exercise the same directional DOM navigation contract used by the
        # controller adapter, through actual native keyboard events here.
        controls.key(clients[0].pid,0xff54) # Down to Inventory
        controls.key(clients[0].pid,0xff52) # Up to Phone
        controls.key(clients[0].pid,0xff0d) # Enter activates the focused action
        park.wait_until(lambda:elements(0,'phone'),12,'native directional navigation opens phone',[server,*clients])
        result['native_directional_navigation']=True
        controls.key(clients[1].pid,0xffbf,.75)
        park.wait_until(lambda:elements(1,'menu'),15,'ordinary player menu',[server,*clients])
        assert not any(e['id'] in ['admin-dashboard/open','call-diagnostics/open'] for e in elements(1,'menu')),'ordinary player menu leaked administrator interface'
        park.wait_until(lambda:'VERIFY_ADMIN_FORGED ok=false sender=0' in park.read(args.output/'client1.log'),10,'direct unauthorized private settings request denied',[server,*clients])
        controls.key(clients[1].pid,0xff1b);controls.key(clients[1].pid,ord('p'))
        park.wait_until(lambda:all(elements(i,'phone') for i in range(2)),15,'both composited phones',[server,*clients])
        capture('phone-home')
        def voice(index):
            matches=list(re.finditer(r'VERIFY_VOICE device=(\S+) channel=(\S*) captured=(\d+) audible=(\d+) muted=(true|false) deafened=(true|false)',park.read(args.output/f'client{index}.log')))
            if not matches:return None
            m=matches[-1];return {'device':m[1],'channel':m[2],'captured':int(m[3]),'audible':int(m[4]),'muted':m[5]=='true','deafened':m[6]=='true'}
        park.wait_until(lambda:all(voice(i) for i in range(2)),10,'voice device telemetry',[server,*clients])
        for key,field in [('m','muted'),('d','deafened')]:
            controls.chord(clients[0].pid,ord(key))
            park.wait_until(lambda:voice(0)[field],5,f'local {field} with phone open',[server,*clients])
            controls.chord(clients[0].pid,ord(key))
            park.wait_until(lambda:not voice(0)[field],5,f'local {field} restored',[server,*clients])
        result['local_voice_shortcuts']=True
        # Subsequent native actions are selected by their actual rendered labels,
        # never by evaluating JavaScript or invoking a host resource callback.
        click(0,'phone',lambda e:e['label']=='Phone' or 'contact' in e['label'].lower() or e['id']=='contacts')
        capture('contacts')
        click(0,'phone',lambda e:'call' in e['label'].lower() and 'close' not in e['label'].lower())
        park.wait_until(lambda:'VERIFY_CALL state=ringing incoming=true' in park.read(args.output/'client1.log'),10,'incoming player call',[server,*clients])
        assert all(not voice(i)['channel'] for i in range(2)),'ringing must not join a voice channel before acceptance'
        click(1,'phone',lambda e:'accept' in e['label'].lower())
        park.wait_until(lambda:all('VERIFY_CALL state=active' in park.read(args.output/f'client{i}.log') for i in range(2)),10,'accepted server-owned call',[server,*clients])
        capture('accepted-call')
        controls.key(clients[0].pid,0xff1b)
        closed_at=len(park.read(args.output/'client0.log'))
        park.wait_until(lambda:'VERIFY_CALL state=active' in park.read(args.output/'client0.log')[closed_at:],6,'accepted call survives phone closure',[server,*clients])
        park.wait_until(lambda:voice(0)['channel'] and voice(0)['channel']==voice(1)['channel'],6,'accepted peers join the same call voice channel',[server,*clients])
        before=[voice(i) for i in range(2)]
        assert all(v['channel'] for v in before),'accepted call must own an actual voice channel'
        controls.key(clients[0].pid,ord('v'),1.5)
        controls.key(clients[1].pid,ord('v'),1.5)
        park.wait_until(lambda:all(voice(i)['captured']>before[i]['captured'] and voice(i)['audible']>before[i]['audible'] for i in range(2)),10,'bidirectional encoded call audio decoded to playback',[server,*clients])
        result['voice_deltas']=[{'captured_frames':voice(i)['captured']-before[i]['captured'],'audible_samples':voice(i)['audible']-before[i]['audible']} for i in range(2)]
        capture('call-audio')
        controls.key(clients[0].pid,ord('p'))
        click(0,'phone',lambda e:'return' in e['label'].lower())
        click(0,'phone',lambda e:'hang' in e['label'].lower())
        park.wait_until(lambda:all(not voice(i)['channel'] for i in range(2)),6,'hang-up releases both local call channels',[server,*clients])
        controls.key(clients[0].pid,0xff1b);controls.key(clients[0].pid,ord('p'))
        click(0,'phone',lambda e:'camera' in e['label'].lower())
        capture('viewfinder')
        controls.key(clients[0].pid,0xffc9) # F12 local shutter
        park.wait_until(lambda:list((args.output/'desktop0').rglob('*.png')),12,'desktop photo export',[server,*clients])
        from PIL import Image,ImageStat
        result['photos']=[]
        for photo in (args.output/'desktop0').rglob('*.png'):
            with Image.open(photo) as image:
                image.load();assert image.format=='PNG' and image.size==(1920,1080),'desktop photo must decode at full resolution'
                assert max(ImageStat.Stat(image.convert('RGB')).stddev)>15,'photograph must contain varied scene pixels'
                result['photos'].append({'path':str(photo),'width':image.width,'height':image.height})
        controls.key(clients[0].pid,0xff1b)
        capture('gallery')
        controls.key(clients[0].pid,0xff1b);controls.key(clients[0].pid,ord('i'))
        park.wait_until(lambda:'Inventory browser synchronized' in park.read(args.output/'client0.log'),10,'real inventory backend response',[server,*clients])
        click(0,'inventory',lambda e:e['label'].startswith('Buy Blue deck'))
        park.wait_until(lambda:'VERIFY_INVENTORY ok=true coins=75 deck=1' in park.read(args.output/'client0.log'),10,'inventory purchase saved through existing account backend',[server,*clients])
        capture('inventory')
        controls.key(clients[0].pid,0xff1b);controls.key(clients[0].pid,0xffbf,.75)
        click(0,'menu',lambda e:'admin' in e['label'].lower())
        click(0,'admin',lambda e:e['id']=='phone-calls')
        capture('administrator-dashboard')
        field=click(0,'admin',lambda e:e['id'].startswith('setting-') and ('ring' in e['id'] or 'timeout' in e['id']))
        for _ in range(5):controls.key(clients[0].pid,0xff08,.02)
        for character in '24':controls.key(clients[0].pid,ord(character),.03)
        click(0,'admin',lambda e:e['id']=='save:'+field['id'])
        def read_saved_setting():
            files=list((args.output/'storage').glob('*/settings/phone-calls.json'))
            for path in files:
                try:
                    value=json.loads(path.read_text())
                    if 'phone-calls' in str(path) and value.get('ring_seconds')==24:return True
                except (OSError,ValueError):pass
            return False
        park.wait_until(read_saved_setting,10,'authoritative persisted ring_seconds=24',[server,*clients])
        result['admin_persisted_setting']={'resource':'phone-calls','key':'ring_seconds','value':24}
        capture('administrator-saved-setting')
        controls.key(clients[0].pid,0xff1b);controls.key(clients[0].pid,0xffbf,.75)
        click(0,'menu',lambda e:e['id']=='call-diagnostics/open')
        click(0,'diagnostics',lambda e:e['id']=='test')
        park.wait_until(lambda:re.search(r'VERIFY_DIAGNOSTICS operation=test ok=true passed=true checked=[1-9]',park.read(args.output/'client0.log')),10,'authorized custom dashboard bounded server test',[server,*clients])
        capture('plugin-diagnostics')
        controls.key(clients[0].pid,0xff1b)
        controls.key(clients[0].pid,ord('p'))
        click(0,'phone',lambda e:e['label']=='Phone')
        click(0,'phone',lambda e:e['label'].lower().startswith('call player'))
        controls.screenshot(clients[1].pid,args.output/'second-call-before-accept-client1.png')
        click(1,'phone',lambda e:'accept' in e['label'].lower())
        controls.screenshot(clients[1].pid,args.output/'second-call-after-accept-client1.png')
        park.wait_until(lambda:voice(0)['channel'] and voice(0)['channel']==voice(1)['channel'],8,'second accepted call before resource restart',[server,*clients])
        click(0,'phone',lambda e:e['id']=='home-button')
        click(0,'phone',lambda e:e['label']=='Camera')
        capture('restart-viewfinder')
        closed=[park.read(args.output/f'client{i}.log').count('RESOURCE_BROWSER_CLOSED resource=phone') for i in range(2)]
        command('restart phone-calls')
        park.wait_until(lambda:all(park.read(args.output/f'client{i}.log').count('RESOURCE_BROWSER_CLOSED resource=phone')>closed[i] for i in range(2)),12,'call resource restart tears down dependent interfaces and active viewfinder',[server,*clients])
        park.wait_until(lambda:all(not voice(i)['channel'] for i in range(2)),10,'resource retirement clears both accepted voice channels',[server,*clients])
        result['active_call_restart_cleanup']=True
        capture('restart-cleanup')
        # Repeated real opens exercise process/image/camera retirement. No extra
        # export is requested; a reopened gallery may show only managed photos.
        for cycle in range(3):
            controls.key(clients[0].pid,ord('p'))
            click(0,'phone',lambda e:e['label']=='Camera')
            capture(f'cycle-{cycle+1}-viewfinder')
            controls.key(clients[0].pid,0xff1b)
            park.wait_until(lambda:any(e['id']=='home-button' for e in elements(0,'phone')),8,'gallery restored after camera release',[server,*clients])
            controls.key(clients[0].pid,0xff1b)
        # Inject a renderer process failure only after gameplay acceptance. It
        # must be classified as an error and shown by the engine-owned notice;
        # an unexpected EOF must not masquerade as a graceful page close.
        controls.key(clients[0].pid,ord('p'))
        park.wait_until(lambda:elements(0,'phone'),12,'phone ready before renderer failure',[server,*clients])
        renderer=int(re.findall(r'RESOURCE_BROWSER_OPEN resource=phone key=phone pid=(\d+)',park.read(args.output/'client0.log'))[-1])
        os.kill(renderer,9)
        park.wait_until(lambda:'Lua [phone]: browser phone: browser companion exited' in park.read(args.output/'client0.log'),8,'renderer failure reaches engine-owned error path',[server,*clients])
        result['renderer_failure_detected']=True
        capture('renderer-failure')
        observed=len(park.read(args.output/'client0.log'))
        controls.key(clients[0].pid,0xff1b)
        park.wait_until(lambda:'paused:true' in park.read(args.output/'client0.log')[observed:],6,'core pause remains accessible after renderer failure',[server,*clients])
        pause_capture=args.output/'renderer-failure-core-pause-client0.png'
        controls.screenshot(clients[0].pid,pause_capture)
        result['core_pause_after_renderer_failure']=str(pause_capture)
        def renderer_alive(pid):
            try:os.kill(int(pid),0);return True
            except ProcessLookupError:return False
        renderer_pids=set()
        for i in range(2):renderer_pids.update(re.findall(r'RESOURCE_BROWSER_OPEN [^\n]*pid=(\d+)',park.read(args.output/f'client{i}.log')))
        park.wait_until(lambda:not any(renderer_alive(pid) for pid in renderer_pids),10,'all retired browser host processes exit',[server,*clients])
        result['repeated_camera_cycles']=3
        result['retired_browser_hosts']=len(renderer_pids)
        result['voice_observations']={str(i):re.findall(r'VERIFY_VOICE [^\n]+',park.read(args.output/f'client{i}.log'))[-12:] for i in range(2)}
        result['ok']=True
    except (OSError,RuntimeError,AssertionError,subprocess.SubprocessError,ValueError,KeyError,IndexError) as error:
        result['error']=str(error)
    finally:
        for stopped in workers:stopped.set()
        for client in reversed(clients):park.stop(client)
        if server:park.stop(server,graceful=True)
        if controls:controls.close()
        for handle in handles:handle.close()
        (args.output/'results.json').write_text(json.dumps(result,indent=2))
    print(json.dumps(result,indent=2));return 0 if result['ok'] else 1


if __name__=='__main__':raise SystemExit(main())
