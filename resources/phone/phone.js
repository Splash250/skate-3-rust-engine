'use strict';
const $ = id => document.getElementById(id);
const NS='http://www.w3.org/2000/svg';
const paths={
  phone:['M7 3 4 4c-3 5 8 16 13 16l3-3-4-4-3 2c-2-1-4-3-5-5l2-3Z'],camera:['M4 7h4l2-3h4l2 3h4v13H4Z','M16 13a4 4 0 1 1-8 0 4 4 0 0 1 8 0'],gallery:['M3 4h18v16H3Z','m3 17 5-6 4 4 3-3 6 6','M8 8h.01'],settings:['M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8','m10 3-.6 2.3-2 .9-2.1-.7-2 3.5 1.7 1.6v2.5l-1.7 1.7 2 3.5 2.1-.7 2 .9L10 21h4l.6-2.5 2-.9 2.1.7 2-3.5-1.7-1.7v-2.5l1.7-1.6-2-3.5-2.1.7-2-.9L14 3Z'],back:['m14 6-6 6 6 6'],close:['m6 6 12 12','m6 18 12-12'],user:['M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0','M4 21v-3a8 8 0 0 1 16 0v3'],inventory:['M4 7h16v14H4Z','m4 7 4-4h8l4 4','M9 11h6'],voice:['M9 5a3 3 0 0 1 6 0v7a3 3 0 0 1-6 0Z','M6 10v2a6 6 0 0 0 12 0v-2','M12 18v4','M9 22h6'],app:['M4 4h6v6H4Z','M14 4h6v6h-6Z','M4 14h6v6H4Z','M14 14h6v6h-6Z'],map:['m3 6 6-3 6 3 6-3v15l-6 3-6-3-6 3Z','M9 3v15','M15 6v15'],refresh:['M20 7V3l-3 3a8 8 0 1 0 2 11','M15 7h5']
}
;
function icon(name){
  const svg=document.createElementNS(NS,'svg');
  svg.setAttribute('viewBox','0 0 24 24');
  svg.setAttribute('aria-hidden','true');
  for(const d of paths[name]||paths.app){
    const p=document.createElementNS(NS,'path');
    p.setAttribute('d',d);
    svg.append(p);
  }
  return svg;
}
function el(tag,text,cls){
  const n=document.createElement(tag);
  if(text!==undefined)n.textContent=text;
  if(cls)n.className=cls;
  return n;
}
function button(text,fn,cls='secondary'){
  const b=el('button',text,cls);
  b.type='button';
  b.addEventListener('click',fn);
  return b;
}
function post(value){
  if(!window.skate?.postMessage){
    notice('Open Phone from a connected Skate server.');
    return false;
  }
  window.skate.postMessage(value);
  return true;
}
let route='home',preferences={
  theme:'twilight',scale:1
}
,calls={
  call:{
    state:'idle'
  }
  ,contacts:[],voice:{
    enabled:false
  }
}
,apps=[],photos=[],page=0;
let lastCalls='',lastApps='',pending=false,requestTimer,noticeTimer,clockTimer,closing=false;
const thumbs=new Map(),wanted=new Set();
const ongoing=()=>['ringing','connecting','active'].includes(calls.call?.state);
function notice(message){
  if(typeof message!=='string'||!message)return;
  $('notice').textContent=message;
  $('notice').hidden=false;
  clearTimeout(noticeTimer);
  noticeTimer=setTimeout(()=>$('notice').hidden=true,5500);
}
function theme(){
  document.body.dataset.theme=preferences.theme==='paper'?'paper':'twilight';
}
function close(){
  if(closing)return;
  closing=true;
  post({
    action:'route',route:'home'
  }
  );
  $('phone').classList.add('closing');
  setTimeout(()=>post({
    action:'close'
  }
  ),200);
}
function go(next){
  route=next;
  pending=false;
  clearTimeout(requestTimer);
  $('phone').hidden=next==='camera';
  $('camera-overlay').hidden=next!=='camera';
  $('home').hidden=next!=='home';
  $('navigation').hidden=next==='home';
  $('content').hidden=next==='home';
  $('title').textContent={
    contacts:'Phone',gallery:'Gallery',settings:'Settings',call:'Call',photo:'Photograph',voice:'Voice'
  }
  [next]||'';
  post({
    action:'route',route:next
  }
  );
  render();
  requestAnimationFrame(()=>focusFirst());
}
function focusFirst(){
  const candidates=focusable();
  (candidates.find(n=>n.closest('#content')||n.closest('#apps'))||candidates[0])?.focus({
    preventScroll:true
  }
  );
}
function focusable(){
  return [...document.querySelectorAll('button:not(:disabled),select:not(:disabled)')].filter(n=>n.getClientRects().length&&!n.closest('[hidden]'));
}
function callRequest(operation,target){
  if(pending)return;
  pending=true;
  post({
    action:'call',operation,target,id:calls.call?.id
  }
  );
  if(operation==='dial')notice('Calling player…');
  requestTimer=setTimeout(()=>{
    pending=false;
    notice('The server has not replied. Refresh or try again.');
    render();
  }
  ,6000);
  render();
}
function renderApps(){
  const root=$('apps');
  const active=document.activeElement?.dataset.app;
  root.replaceChildren();
  const builtin=[['contacts','Phone','phone'],['camera','Camera','camera'],['gallery','Gallery','gallery'],['settings','Settings','settings']];
  for(const [id,label,glyph] of builtin){
    const b=button('',()=>go(id),'app');
    b.dataset.app=id;
    b.setAttribute('aria-label',label);
    const face=el('span',undefined,'app-face');
    face.append(icon(glyph));
    b.append(face,el('span',label,'app-label'));
    root.append(b);
  }
  for(const entry of apps){
    if(!entry.phone||entry.version!==1||entry.id==='phone/open')continue;
    const b=button('',()=>post({
      action:'invoke',id:entry.id,generation:entry.generation
    }
    ),'app');
    b.dataset.app=entry.id;
    b.setAttribute('aria-label',entry.label);
    if(entry.disabled_reason){
      b.disabled=true;
      b.title=entry.disabled_reason;
    }
    const face=el('span',undefined,'app-face');
    face.append(icon(entry.icon));
    b.append(face,el('span',entry.label,'app-label'));
    root.append(b);
  }
  if(active)[...root.querySelectorAll('button')].find(b=>b.dataset.app===active)?.focus({
    preventScroll:true
  }
  );
}
function empty(root,glyph,title,body){
  const box=el('div',undefined,'empty');
  box.append(icon(glyph),el('h2',title),el('p',body));
  root.append(box);
}
function renderContacts(root){
  root.append(el('p','Players in your instance. Calls stay connected when you close your phone.','intro'));
  if(!calls.voice?.enabled)root.append(el('p','Voice is off. Relaunch with --voice to make and receive calls.','voice-note'));
  if(!Array.isArray(calls.contacts)||!calls.contacts.length)empty(root,'user','No one to call yet','Refresh after another player joins your instance.');
  for(const person of (Array.isArray(calls.contacts)?calls.contacts:[])){
    const row=el('div',undefined,'row'),avatar=el('span',person.id.slice(-2),'avatar'),detail=el('div',undefined,'detail');
    detail.append(el('strong',person.label||`Player ${person.id}`),el('small',person.available?'Available':person.reason||'Unavailable'));
    const b=button('',()=>callRequest('dial',person.id),'');
    b.append(icon('phone'));
    b.setAttribute('aria-label',`Call ${person.label||person.id}`);
    b.disabled=!person.available||!calls.voice?.enabled||pending||ongoing();
    row.append(avatar,detail,b);
    root.append(row);
  }
  root.append(button('Refresh players',()=>{
    post({
      action:'refresh_contacts'
    }
    );
    notice('Refreshing players…');
  }
  ));
}
function renderCall(root){
  const c=calls.call||{
    state:'idle'
  }
  ;
  const label={
    ringing:c.incoming?'Incoming call':'Calling…',connecting:'Connecting voice…',active:'Call connected',ended:c.reason||'Call ended',idle:'No active call'
  }
  [c.state]||'Call unavailable';
  root.append(el('p',label,'call-state'));
  const avatar=el('div',undefined,'call-avatar');
  avatar.append(icon('user'));
  root.append(avatar,el('h2',c.peer?`Player ${c.peer}`:'Your next conversation','call-name'));
  const actions=el('div',undefined,'call-actions');
  if(c.state==='ringing'&&c.incoming){
    actions.append(button('Decline',()=>callRequest('decline'), 'danger'),button('Accept',()=>callRequest('accept'),'primary'));
  }
  else if(c.state==='ringing'){
    actions.append(button('Cancel call',()=>callRequest('cancel'),'danger'));
  }
  else if(c.state==='active'||c.state==='connecting'){
    actions.append(button('Hang up',()=>callRequest('hangup'),'danger'));
  }
  else actions.append(button('Find a player',()=>go('contacts'),'primary'));
  for(const b of actions.children)b.disabled=pending;
  root.append(actions);
  if(ongoing())root.append(el('p','Hold V or LB to talk. Local mute and deafen remain in effect.','call-help'));
  if(calls.voice?.muted)root.append(el('p','Your microphone is muted. Ctrl+M toggles local mute.','voice-note'));
  if(calls.voice?.deafened)root.append(el('p','You are deafened. Ctrl+D restores playback.','voice-note'));
  const device=calls.voice?.device;
  if(device?.state==='error')root.append(el('p','Audio device unavailable. Check the Voice interface and your operating-system audio settings.','voice-note'));
}
function renderGallery(root){
  root.append(el('p','Your photographs are saved locally to Desktop / Skate Photos. This gallery holds the latest 32 photos from this phone session.','intro'));
  if(!photos.length){
    empty(root,'gallery','Nothing here. Yet.','Open Camera, frame a moment, then press X or F12 to save a photograph.');
    root.append(button('Open Camera',()=>go('camera'),'primary'));
    return;
  }
  const grid=el('div',undefined,'photo-grid');
  const visible=photos.slice(page*8,page*8+8);
  for(const photo of visible){
    const b=button('',()=>showPhoto(photo),'photo');
    b.setAttribute('aria-label',`View photograph ${photos.indexOf(photo)+1}`);
    if(thumbs.has(photo.id)){
      const image=el('img');
      image.src=thumbs.get(photo.id);
      image.alt='Photographed game scene';
      b.append(image);
    }
    else{
      b.append(el('span','Loading…','loading-thumb'));
      if(!wanted.has(photo.id)){
        wanted.add(photo.id);
        post({
          action:'thumbnail',id:photo.id
        }
        );
      }
    }
    b.append(el('span',`${photo.width} × ${photo.height} · PNG`));
    grid.append(b);
  }
  root.append(grid);
  if(photos.length>8){
    const pager=el('div',undefined,'pager');
    const previous=button('Previous',()=>{
      page--;
      thumbs.clear();
      wanted.clear();
      render();
    }
    ),next=button('Next',()=>{
      page++;
      thumbs.clear();
      wanted.clear();
      render();
    }
    );
    previous.disabled=page===0;
    next.disabled=(page+1)*8>=photos.length;
    pager.append(previous,next);
    root.append(pager);
  }
}
function showPhoto(photo){
  route='photo';
  $('title').textContent='Photograph';
  const root=$('content');
  root.replaceChildren();
  const image=el('img');
  image.src=thumbs.get(photo.id)||'';
  image.alt='Saved photograph preview';
  image.className='photo-view';
  root.append(image,el('h2','Saved to your desktop','settings-heading'),el('p',`${photo.width} × ${photo.height} PNG. Open Skate Photos on your desktop for the full-resolution photograph.`,'intro'),button('Back to Gallery',()=>go('gallery')));
  focusFirst();
}
function remap(root){
  const entries=apps.filter(e=>e.binding);
  if(!entries.length)return;
  root.append(el('h2','Remap shortcuts','settings-heading'),el('p','Saved on this device. Reserved gameplay controls are excluded; conflicts are rejected.','voice-note'));
  const field=(label,id,choices)=>{
    const l=el('label',label,'voice-note');
    l.htmlFor=id;
    const select=el('select');
    select.id=id;
    for(const [value,name] of choices){
      const option=el('option',name);
      option.value=String(value);
      select.append(option);
    }
    root.append(l,select);
    return select;
  }
  ;
  const action=field('Action','bind-action',entries.map(e=>[e.id,e.label]));
  const key=field('Keyboard','bind-key',['F2','F3','F4','F7','F8','KeyI','KeyP','KeyO'].map(k=>[k,k.replace('Key','')]));
  const pad=field('Controller','bind-pad',[['','None'],[32,'View / Back / Select'],[64,'Left-stick click'],[128,'Right-stick click'],[32768,'Y / Triangle']]);
  const hold=field('Hold duration','bind-hold',[[0,'Press'],[300,'300 ms'],[600,'600 ms'],[1000,'1 second'],[1500,'1.5 seconds'],[2000,'2 seconds']]);
  const hint=el('p','','voice-note');
  root.append(hint);
  const populate=()=>{
    const entry=apps.find(e=>e.id===action.value);
    hint.textContent=entry?.binding_conflict||'';
    const binding=entry?.binding;
    if(!binding)return;
    key.value=binding.key;
    pad.value=binding.button==null?'':String(binding.button);
    hold.value=String(binding.hold_ms||0);
    if(!hold.value)hold.value='600';
  }
  ;
  action.addEventListener('change',populate);
  populate();
  const save=button('Save shortcut',()=>{
    const value={
      action:'binding',id:action.value,key:key.value,hold_ms:Number(hold.value)
    }
    ;
    if(pad.value)value.button=Number(pad.value);
    save.disabled=true;
    save.textContent='Saving…';
    post(value);
  }
  );
  save.id='save-binding';
  root.append(save);
}
function renderSettings(root){
  root.append(el('p','Make it yours. These preferences stay on this device and do not change server settings.','intro'),el('h2','Appearance','settings-heading'));
  const themes=el('div',undefined,'themes');
  for(const [id,label,description] of [['twilight','Twilight','Warm after-hours colors'],['paper','Paper','Soft daylight colors']]){
    const b=button('',()=>{
      preferences.theme=id;
      theme();
      post({
        action:'preferences',theme:id
      }
      );
      render();
    }
    ,`theme-choice ${id}${preferences.theme===id?' selected':''}`);
    b.setAttribute('aria-pressed',String(preferences.theme===id));
    b.append(el('div',undefined,'theme-swatch'),el('strong',label),el('small',description));
    themes.append(b);
  }
  root.append(themes,el('h2','Phone size','settings-heading'));
  const label=el('label','Overlay scale');
  label.htmlFor='scale';
  label.className='intro';
  const select=el('select');
  select.id='scale';
  for(const [value,name] of [[.75,'Compact · 75%'],[.85,'Comfortable · 85%'],[1,'Full · 100%']]){
    const option=el('option',name);
    option.value=String(value);
    option.selected=preferences.scale===value;
    select.append(option);
  }
  select.addEventListener('change',()=>post({
    action:'preferences',scale:Number(select.value)
  }
  ));
  root.append(label,select,el('h2','Controls','settings-heading'));
  for(const [name,key] of [['Open phone','P'],['Navigate','D-pad / arrows'],['Choose','A / Enter'],['Back / close','B / Escape'],['Push to talk','V / LB'],['Photograph','X / F12']]){
    const row=el('div',undefined,'shortcut');
    row.append(el('span',name),el('strong',key));
    root.append(row);
  }
  remap(root);
  root.append(el('p','Calls use push-to-talk. Voice must be enabled locally with --voice. Nothing is uploaded from your gallery.','voice-note'));
}
function renderVoice(root){
  const v=calls.voice||{
  }
  ,c=calls.controls||{
  }
  ;
  root.append(el('p','Hold V or LB to transmit. Your physical push-to-talk, local mute and deafen always take precedence.','intro'));
  if(!v.enabled){
    empty(root,'voice','Voice is off','Relaunch Skate with --voice to enable microphone and playback devices.');
    return;
  }
  root.append(el('p',`Audio: ${v.device?.state||'waiting'}`,'voice-note'));
  for(const [field,name] of [['muted','microphone'],['deafened','playback']]){
    root.append(button(`${c[field]?'Enable':'Mute'} ${name} in Phone`,()=>{
      post({
        action:'call',operation:'voice',[field]:!c[field]
      }
      );
    }
    ));
  }
  for(const [field,label,list] of [['input_device','Microphone',calls.devices?.inputs],['output_device','Playback device',calls.devices?.outputs]]){
    const heading=el('label',label,'settings-heading');
    heading.htmlFor=field;
    heading.style.display='block';
    const select=el('select');
    select.id=field;
    const option=el('option','System default');
    option.value='';
    select.append(option);
    for(const device of (Array.isArray(list)?list:[])){
      const option=el('option',device);
      option.value=device;
      select.append(option);
    }
    select.value=c[field]||'';
    select.addEventListener('change',()=>post({
      action:'call',operation:'voice',[field]:select.value
    }
    ));
    root.append(heading,select);
  }
  root.append(button('Refresh audio devices',()=>post({
    action:'call',operation:'devices'
  }
  )));
  if(calls.devices?.truncated)root.append(el('p','Showing a bounded list of audio devices. Choose System default to use another device selected in your operating system.','voice-note'));
  if(v.muted)root.append(el('p','Microphone is muted. If Phone mute is off, press Ctrl+M to clear your local mute.','voice-note'));
  if(v.deafened)root.append(el('p','Playback is deafened. If Phone mute is off, press Ctrl+D to clear your local deafen.','voice-note'));
  if(v.device?.state==='error')root.append(el('p',v.device.message||v.device.value?.message||'Device unavailable. Select another device or check your operating-system audio settings.','voice-note'));
}
function render(preserveContent=false){
  const focused=document.activeElement;
  const focusedLabel=focused?.getAttribute('aria-label')||focused?.textContent;
  const focusInContent=$('content').contains(focused);
  const root=$('content');
  const keep=preserveContent&&['settings','gallery','photo','voice'].includes(route);
  if(!keep){
    if(route!=='photo')root.replaceChildren();
    if(route==='home')renderApps();
    else if(route==='contacts')renderContacts(root);
    else if(route==='call')renderCall(root);
    else if(route==='gallery')renderGallery(root);
    else if(route==='settings')renderSettings(root);
    else if(route==='voice')renderVoice(root);
  }
  const banner=$('call-banner');
  banner.hidden=!ongoing()||route==='call'||route==='camera';
  banner.replaceChildren();
  if(!banner.hidden)banner.append(button(calls.call.state==='ringing'?(calls.call.incoming?'Incoming call · Answer':'Calling · View call'):'Call connected · Return',()=>go('call'),''));
  if(focusInContent&&!keep){
    const candidate=focusable().find(n=>(n.getAttribute('aria-label')||n.textContent)===focusedLabel);
    (candidate||focusable().find(n=>root.contains(n)))?.focus({
      preventScroll:true
    }
    );
  }
}
function navigate(action){
  if(action==='close'){
    close();
    return;
  }
  if(action==='back'){
    if(route==='home')close();
    else if(route==='photo')go('gallery');
    else go('home');
    return;
  }
  const nodes=focusable();
  if(!nodes.length)return;
  if(action==='confirm'){
    document.activeElement?.click();
    return;
  }
  const index=nodes.indexOf(document.activeElement);
  if(index<0){
    focusFirst();
    return;
  }
  const current=nodes[index].getBoundingClientRect();
  const horizontal=action==='left'||action==='right';
  const forward=action==='right'||action==='down';
  let best=null,score=Infinity;
  for(const node of nodes){
    if(node===nodes[index])continue;
    const r=node.getBoundingClientRect();
    const dx=r.x+r.width/2-current.x-current.width/2,dy=r.y+r.height/2-current.y-current.height/2;
    const along=horizontal?dx:dy,cross=horizontal?dy:dx;
    if((forward?along:-along)<4)continue;
    const cost=Math.abs(along)+Math.abs(cross)*3;
    if(cost<score){
      score=cost;
      best=node;
    }
  }
  (best||nodes[(index+(forward?1:-1)+nodes.length)%nodes.length]).focus();
}
window.addEventListener('keydown',event=>{
  const action={
    ArrowUp:'up',ArrowDown:'down',ArrowLeft:'left',ArrowRight:'right',Enter:'confirm',Escape:'back'
  }
  [event.key];
  if(!action)return;
  if(event.target.tagName==='SELECT'&&event.key!=='Escape')return;
  event.preventDefault();
  navigate(action);
}
);
window.addEventListener('message',event=>{
  const data=event.data;
  if(!data||typeof data!=='object')return;
  if(data.kind==='navigate'){
    navigate(data.action);
    return;
  }
  if(data.kind==='binding_result'){
    const b=$('save-binding');
    if(b){
      b.disabled=false;
      b.textContent='Save shortcut';
    }
    notice(data.ok?'Shortcut saved on this device.':data.error||'Could not save shortcut.');
    return;
  }
  if(data.kind==='route'){
    go(data.route);
    return;
  }
  if(data.kind==='boot'){
    preferences=data.preferences||preferences;
    calls=data.calls||calls;
    lastCalls=JSON.stringify(calls);
    theme();
    go(data.route||'home');
    return;
  }
  if(data.kind==='preferences'){
    preferences=data.value;
    theme();
    render();
    return;
  }
  if(data.kind==='apps'){
    const next=Array.isArray(data.entries)?data.entries:[];
    const signature=JSON.stringify(next);
    if(lastApps!==signature){
      apps=next;
      lastApps=signature;
      if(route==='home')renderApps();
    }
    return;
  }
  if(data.kind==='calls'){
    const signature=JSON.stringify(data.value);
    if(signature===lastCalls)return;
    const previous=calls.call?.state;
    const callChanged=JSON.stringify(calls.call)!==JSON.stringify(data.value.call)||calls.message!==data.value.message;
    const voiceUi=value=>JSON.stringify({controls:value.controls,devices:value.devices,enabled:value.voice?.enabled,device:value.voice?.device,muted:value.voice?.muted,deafened:value.voice?.deafened});
    const changedVoiceUi=voiceUi(calls)!==voiceUi(data.value);
    lastCalls=signature;
    calls=data.value;
    if(callChanged){clearTimeout(requestTimer);pending=false;}
    if(calls.message)notice(calls.message);
    if(calls.call?.state==='ringing'&&previous!=='ringing'&&route!=='camera'){
      go('call');
    }
    else render(!(route==='voice'&&changedVoiceUi));
    $('voice-status').textContent=!calls.voice?.enabled?'Voice off':calls.voice?.device?.state==='error'?'Audio error':calls.voice?.muted?'Mic muted':ongoing()?'In call':'Connected';
    return;
  }
  if(data.kind==='error'){
    notice(data.message);
    return;
  }
  if(data.kind==='photo'){
    const p=data.value;
    if(p.kind==='gallery'){
      photos=Array.isArray(p.photos)?p.photos:[];
      page=Math.min(page,Math.max(0,Math.ceil(photos.length/8)-1));
      thumbs.clear();
      wanted.clear();
      if(route==='gallery')render();
    }
    else if(p.kind==='thumbnail'&&typeof p.data==='string'&&p.data.startsWith('data:image/jpeg;base64,')&&photos.slice(page*8,page*8+8).some(x=>x.id===p.id)){
      if(thumbs.size<8)thumbs.set(p.id,p.data);
      if(route==='gallery')render();
    }
    else if(p.kind==='mode'&&!p.enabled){
      go('gallery');
    }
    else if(p.kind==='saving'){
      $('camera-status').textContent='Saving photograph…';
    }
    else if(p.kind==='saved'){
      $('camera-status').textContent='Saved to Desktop / Skate Photos.';
      notice('Photograph saved.');
    }
    else if(p.kind==='error'){
      $('camera-status').textContent=p.message;
      notice(p.message);
      if(route==='camera'&&!p.saving){
        $('phone').hidden=false;
        $('camera-overlay').hidden=true;
        go('gallery');
      }
    }
  }
}
);
$('back').append(icon('back'));
$('close').append(icon('close'));
$('home-close').append(icon('close'));
$('back').addEventListener('click',()=>navigate('back'));
$('close').addEventListener('click',close);
$('home-close').addEventListener('click',close);
$('home-button').addEventListener('click',()=>go('home'));
function clock(){
  const now=new Date(),text=now.toLocaleTimeString([], {
    hour:'2-digit',minute:'2-digit',hour12:false
  }
  );
  $('clock').textContent=text;
  $('large-clock').textContent=text;
  $('date').textContent=now.toLocaleDateString([], {
    weekday:'long',month:'long',day:'numeric'
  }
  );
}
clock();
clockTimer=setInterval(clock,1000);
theme();
render();
setTimeout(()=>{
  if(!window.skate?.postMessage)notice('Waiting for a connected Skate resource.');
  focusFirst();
}
,300);
