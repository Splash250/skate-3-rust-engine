'use strict';
const $ = id => document.getElementById(id);
const token = location.hash.slice(1);
history.replaceState(null, '', location.pathname);
let state, selected = null, busy = false, view = 'top', zoom = 12, center = [0,0,0], drag = null, hits = [];
const canvas = $('viewport'), ctx = canvas.getContext('2d');
const clone = value => JSON.parse(JSON.stringify(value));
const snap = value => { const unit = Math.max(0, Number($('snap').value) || 0); return unit ? Math.round(value / unit) * unit : value; };
function status(message, error=false) { $('status').textContent = message; $('status').dataset.error = String(error); }
async function request(action, extra={}) {
  if (busy) return;
  busy = true;
  try {
    const response = await fetch(action ? '/api/action' : '/api/state', {method:action ? 'POST' : 'GET', headers:{'X-Editor-Token':token, ...(action ? {'Content-Type':'application/json'} : {})}, ...(action ? {body:JSON.stringify({action,...extra})} : {})});
    const value = await response.json();
    if (!response.ok) throw new Error(value.error || 'Editor request failed');
    state = value; render();
    if (value.message) status(value.message);
    else if (action === 'edit') status('Change applied. Save to keep this revision.');
  } catch (error) {status(error.message, true); if (state) render();}
  finally {busy = false;}
}
function fields(parent, prefix, values) {
  $(parent).replaceChildren(...['X','Y','Z'].map((axis,i) => {
    const label=document.createElement('label'); label.textContent=axis;
    const input=document.createElement('input'); input.type='number'; input.step='0.1'; input.id=prefix+'-'+i; input.value=values[i]; label.append(input); return label;
  }));
}
const numbers = prefix => [0,1,2].map(i => Number($(prefix+'-'+i).value));
function object() {return state?.scene.objects.find(o=>o.id === selected);}
function mutate(callback) {if (!state || busy) return; const scene=clone(state.scene); callback(scene); request('edit',{scene});}
function choose(id) { selected=id; renderInspector(); renderList(); draw(); }
function renderList() {
  const filter=$('filter').value.toLowerCase();
  $('object-list').replaceChildren(...state.scene.objects.filter(o=>(o.id+' '+o.kind).includes(filter)).map(o=>{
    const button=document.createElement('button'); button.className='object'; button.setAttribute('role','option'); button.setAttribute('aria-selected',String(o.id===selected));
    button.textContent=o.id; const kind=document.createElement('small');kind.textContent=o.kind==='marker' ? (o.marker_type||'interaction')+' marker' : o.kind; button.append(kind);button.onclick=()=>choose(o.id);return button;
  }));
  $('duplicate').disabled=$('delete').disabled=!object();
}
function renderInspector() {
  const o=object(); $('empty-selection').hidden=!!o; $('inspector-form').hidden=!o; if(!o)return;
  $('object-id').value=o.id; $('object-kind').textContent=o.kind==='rail'?'Native rail · mesh + collision + grind path':o.kind==='marker'?'Marker · visible, no collision':o.kind+' · mesh + collision';
  fields('position-fields','position',o.position);fields('size-fields','size',o.size||[1,1,1]);$('rotation').value=o.rotation||0;
  $('size-label').textContent=o.kind==='rail'?'Rail scale · multiplier':o.kind==='marker'?'Radius X · height Y · depth Z':'Size · metres';
  $('color').value='#'+(o.color||[.55,.6,.65]).map(v=>Math.round(v*255).toString(16).padStart(2,'0')).join('');
  $('rail-controls').hidden=o.kind!=='rail';$('marker-controls').hidden=o.kind!=='marker';
  if(o.kind==='rail'){$('rail-points').value=o.points.map(p=>p.join(', ')).join('\n');$('rail-closed').checked=!!o.closed;}
  if(o.kind==='marker'){$('marker-type').value=o.marker_type||'interaction';$('marker-order').value=o.order||0;$('marker-label').value=o.label||o.id;}
}
function render() {
  if(!object())selected=state.scene.objects[0]?.id||null;
  $('document-state').textContent=state.scene.name+(state.dirty?' · Unsaved changes':' · Saved');
  $('filename').value=state.filename;$('undo').disabled=!state.undo;$('redo').disabled=!state.redo;
  renderPlaytest();
  fields('spawn-fields','spawn',state.scene.spawn);$('spawn-heading').value=state.scene.heading||0;$('park-name').value=state.scene.name;
  renderList();renderInspector();draw();
}
function project(p) {
  const x=p[0]-center[0],y=p[1]-center[1],z=p[2]-center[2],w=canvas.clientWidth,h=canvas.clientHeight;
  if(view==='top')return [w/2+x*zoom,h/2+z*zoom];
  if(view==='front')return [w/2+x*zoom,h/2-y*zoom];
  return [w/2+(x-z)*.7071*zoom,h/2+((x+z)*.35355-y*.866)*zoom];
}
function transform(o,p,scale=false) {
  const size=o.size||[1,1,1], a=(o.rotation||0)*Math.PI/180,c=Math.cos(a),s=Math.sin(a),v=p.map((n,i)=>n*(scale?size[i]:1));
  return [o.position[0]+v[0]*c+v[2]*s,o.position[1]+v[1],o.position[2]-v[0]*s+v[2]*c];
}
function corners(o) {
  const [x,y,z]=(o.size||[1,1,1]).map(v=>v/2);
  const points=o.kind==='ramp'?[[-x,-y,-z],[x,-y,-z],[x,-y,z],[-x,-y,z],[-x,y,z],[x,y,z]]:[[-x,-y,-z],[x,-y,-z],[x,-y,z],[-x,-y,z],[-x,y,-z],[x,y,-z],[x,y,z],[-x,y,z]];
  return points.map(p=>transform(o,p));
}
function polygon(points,fill,stroke) {ctx.beginPath();points.forEach((p,i)=>i?ctx.lineTo(...p):ctx.moveTo(...p));ctx.closePath();if(fill){ctx.fillStyle=fill;ctx.fill();}ctx.strokeStyle=stroke;ctx.stroke();}
function draw() {
  const scale=window.devicePixelRatio||1,w=canvas.clientWidth,h=canvas.clientHeight;
  if(canvas.width!==Math.round(w*scale)||canvas.height!==Math.round(h*scale)){canvas.width=Math.round(w*scale);canvas.height=Math.round(h*scale);}
  ctx.setTransform(scale,0,0,scale,0,0);ctx.clearRect(0,0,w,h);hits=[];
  if(!state)return;
  ctx.lineWidth=1;const extent=Math.min(1000,Math.ceil(Math.max(w,h)/zoom)+20),stride=zoom<5?5:1;
  for(let i=-extent;i<=extent;i+=stride){const line=(a,b)=>{ctx.beginPath();ctx.moveTo(...project(a));ctx.lineTo(...project(b));ctx.stroke();};ctx.strokeStyle=i===0?'#9babA3':'#d0dbd4';
    if(view==='front'){line([-extent,i,0],[extent,i,0]);line([i,-extent,0],[i,extent,0]);}else{line([-extent,0,i],[extent,0,i]);line([i,0,-extent],[i,0,extent]);}}
  const objects=[...state.scene.objects];
  if(view==='iso')objects.sort((a,b)=>(a.position[0]+a.position[2]+a.position[1]*.5)-(b.position[0]+b.position[2]+b.position[1]*.5));
  for(const original of objects){const o=drag?.kind==='move'&&drag.id===original.id?{...original,position:drag.position}:original;const chosen=o.id===selected;ctx.lineWidth=chosen?2.5:1;const col=(o.color||[.55,.6,.65]).map(v=>Math.round(v*255));const color=`rgba(${col.join(',')},${o.kind==='box'&&o.id==='floor'?.45:.82})`;const stroke=chosen?'#913b19':'#52645c';let screen=[];
    if(o.kind==='rail'){screen=o.points.map(p=>project(transform(o,p,true)));ctx.beginPath();screen.forEach((p,i)=>i?ctx.lineTo(...p):ctx.moveTo(...p));if(o.closed)ctx.closePath();ctx.strokeStyle=stroke;ctx.lineWidth=chosen?5:3;ctx.stroke();for(const p of screen){ctx.beginPath();ctx.arc(...p,chosen?4:2,0,Math.PI*2);ctx.fillStyle=chosen?'#fff':'#415a50';ctx.fill();ctx.stroke();}}
    else if(o.kind==='marker'){const p=project(o.position),radius=Math.max(6,(o.size?.[0]||1)*zoom);ctx.beginPath();ctx.ellipse(...p,radius,view==='iso'?radius*.5:radius,0,0,Math.PI*2);ctx.fillStyle=color;ctx.fill();ctx.strokeStyle=stroke;ctx.stroke();screen=[[p[0]-radius,p[1]-radius],[p[0]+radius,p[1]+radius]];ctx.fillStyle='#233b30';ctx.font='11px system-ui';ctx.fillText(o.marker_type==='checkpoint'?String(o.order||0):'M',p[0]-4,p[1]+4);}
    else {const world=corners(o);screen=world.map(project);const faces=o.kind==='ramp'?[[0,3,2,1],[0,1,5,4],[3,4,5,2],[0,4,3],[1,2,5]]:[[0,3,2,1],[0,1,5,4],[3,7,6,2],[0,4,7,3],[1,2,6,5],[4,5,6,7]];for(const indices of faces)polygon(indices.map(i=>screen[i]),color,stroke);}
    const xs=screen.map(p=>p[0]),ys=screen.map(p=>p[1]);hits.push({id:o.id,minX:Math.min(...xs)-5,maxX:Math.max(...xs)+5,minY:Math.min(...ys)-5,maxY:Math.max(...ys)+5});
    if(chosen){const p=project(o.position);ctx.fillStyle='#763017';ctx.font='600 12px system-ui';ctx.fillText(o.id,p[0]+9,p[1]-10);}}
  const spawn=project(state.scene.spawn);ctx.strokeStyle='#126094';ctx.lineWidth=2;ctx.beginPath();ctx.moveTo(spawn[0]-7,spawn[1]);ctx.lineTo(spawn[0]+7,spawn[1]);ctx.moveTo(spawn[0],spawn[1]-7);ctx.lineTo(spawn[0],spawn[1]+7);ctx.stroke();ctx.fillStyle='#174761';ctx.font='11px system-ui';ctx.fillText('spawn',spawn[0]+10,spawn[1]+4);
}
function fit() {
  if(!state)return;const points=state.scene.objects.flatMap(o=>o.kind==='rail'?o.points.map(p=>transform(o,p,true)):o.kind==='marker'?[o.position]:corners(o));
  if(!points.length)return;const min=[0,1,2].map(i=>Math.min(...points.map(p=>p[i]))),max=[0,1,2].map(i=>Math.max(...points.map(p=>p[i])));center=min.map((v,i)=>(v+max[i])/2);zoom=Math.max(2,Math.min(60,Math.min(canvas.clientWidth/(max[0]-min[0]+8),canvas.clientHeight/(view==='front'?max[1]-min[1]+8:max[2]-min[2]+8))));$('zoom').value=zoom;draw();
}
$('inspector-form').onsubmit=e=>{e.preventDefault();const old=object();if(!old)return;try{const updated={...clone(old),id:$('object-id').value,position:numbers('position'),size:numbers('size'),rotation:Number($('rotation').value),color:$('color').value.slice(1).match(/../g).map(v=>parseInt(v,16)/255)};if(old.kind==='rail'){updated.points=$('rail-points').value.trim().split('\n').map(line=>line.trim().split(/[,\s]+/).map(Number));updated.closed=$('rail-closed').checked;}if(old.kind==='marker'){updated.marker_type=$('marker-type').value;updated.order=Number($('marker-order').value);updated.label=$('marker-label').value;}mutate(scene=>{scene.objects[scene.objects.findIndex(o=>o.id===old.id)]=updated;selected=updated.id;});}catch(error){status(error.message,true);}};
$('append-point').onclick=()=>{const lines=$('rail-points').value.trim().split('\n'),last=lines.at(-1).split(/[,\s]+/).map(Number);if(last.length!==3||last.some(v=>!Number.isFinite(v))){status('Enter three valid coordinates for the last point.',true);return;}last[2]+=2;$('rail-points').value+='\n'+last.join(', ');};
function nextId(prefix){let n=1;while(state.scene.objects.some(o=>o.id===prefix+'-'+n))n++;return prefix+'-'+n;}
$('add').onclick=()=>{const kind=$('new-kind').value,id=nextId(kind);mutate(scene=>{const o={id,kind,position:[snap(center[0]),1,snap(center[2])],size:kind==='ramp'?[4,2,6]:kind==='box'?[2,1,2]:[1,1,1],color:kind==='marker'?[.85,.4,.18]:[.52,.62,.55]};if(kind==='rail')o.points=[[0,0,-2],[0,0,2]];if(kind==='marker'){o.marker_type='interaction';o.label=id;}scene.objects.push(o);selected=id;});};
$('duplicate').onclick=()=>{const o=object();if(!o)return;const id=nextId(o.kind);mutate(scene=>{const item=clone(o);item.id=id;item.position[0]+=2;scene.objects.push(item);selected=id;});};
$('delete').onclick=()=>{if(object())mutate(scene=>{scene.objects=scene.objects.filter(o=>o.id!==selected);selected=null;});};
$('apply-spawn').onclick=()=>mutate(scene=>{scene.spawn=numbers('spawn');scene.heading=Number($('spawn-heading').value);scene.name=$('park-name').value;});
$('save').onclick=()=>request('save',{path:$('filename').value});$('load').onclick=()=>{if(!state.dirty||confirm('Load another scene? Unsaved changes stay in Undo history.'))request('load',{path:$('filename').value});};
$('export').onclick=()=>request('export',{path:$('export-path').value,resource_id:$('resource-id').value});$('playtest').onclick=()=>request('playtest');$('stop-playtest').onclick=()=>request('stop_playtest');$('undo').onclick=()=>request('undo');$('redo').onclick=()=>request('redo');$('filter').oninput=renderList;
$('view').onchange=()=>{view=$('view').value;$('view-badge').textContent=view.toUpperCase()+' VIEW · Y-up metres';draw();};$('zoom').oninput=()=>{zoom=Number($('zoom').value);draw();};$('fit').onclick=fit;$('center').onclick=()=>{const o=object();if(o){center=[...o.position];draw();}};
canvas.onpointerdown=e=>{if(!state||busy)return;const rect=canvas.getBoundingClientRect(),x=e.clientX-rect.left,y=e.clientY-rect.top;canvas.focus();canvas.setPointerCapture(e.pointerId);if(e.button===1){drag={kind:'pan',x,y,center:[...center]};e.preventDefault();return;}const hit=[...hits].reverse().find(o=>x>=o.minX&&x<=o.maxX&&y>=o.minY&&y<=o.maxY);if(hit){choose(hit.id);if(view!=='iso')drag={kind:'move',id:hit.id,x,y,original:[...object().position],position:[...object().position]};}};
canvas.onpointermove=e=>{if(!drag)return;const rect=canvas.getBoundingClientRect(),dx=(e.clientX-rect.left-drag.x)/zoom,dy=(e.clientY-rect.top-drag.y)/zoom;if(drag.kind==='pan'){center=[...drag.center];center[0]-=dx;if(view==='front')center[1]+=dy;else center[2]-=dy;}else{drag.position=[...drag.original];drag.position[0]=snap(drag.original[0]+dx);const axis=view==='front'?1:2;drag.position[axis]=snap(drag.original[axis]+(view==='front'?-dy:dy));}draw();};
canvas.onpointerup=()=>{const done=drag;drag=null;if(done?.kind==='move'&&done.position.some((v,i)=>v!==done.original[i]))mutate(scene=>{scene.objects.find(o=>o.id===done.id).position=done.position;});draw();};canvas.onpointercancel=()=>{drag=null;draw();};
canvas.onwheel=e=>{e.preventDefault();zoom=Math.max(2,Math.min(60,zoom*(e.deltaY>0?.9:1.1)));$('zoom').value=zoom;draw();};
document.addEventListener('keydown',e=>{if((e.ctrlKey||e.metaKey)&&e.key.toLowerCase()==='z'){e.preventDefault();request(e.shiftKey?'redo':'undo');return;}if(e.target!==canvas||!object())return;const step=(Number($('snap').value)||.1)*(e.shiftKey?5:1);const directions={ArrowLeft:[0,-step],ArrowRight:[0,step],ArrowUp:[view==='front'?1:2,view==='front'?step:-step],ArrowDown:[view==='front'?1:2,view==='front'?-step:step]};if(directions[e.key]){e.preventDefault();const [axis,delta]=directions[e.key];mutate(scene=>{const o=scene.objects.find(o=>o.id===selected);o.position[axis]=snap(o.position[axis]+delta);});}});
window.addEventListener('beforeunload',e=>{if(state?.dirty){e.preventDefault();e.returnValue='';}});
new ResizeObserver(draw).observe(canvas);
request().then(fit);

function renderPlaytest() {
  $('playtest').disabled=state.playtest.running;$('stop-playtest').disabled=!state.playtest.running;
  $('playtest').title=state.playtest.available?'Launch this park in the native game':'Configure --game-executable and --assets when starting the editor';
  $('playtest-log').textContent=state.playtest.diagnostic||'No completed playtest diagnostics.';
}
// Refresh process diagnostics without replacing in-progress inspector edits.
setInterval(async()=>{
  if(busy||!state?.playtest.running)return;
  try {
    const response=await fetch('/api/state',{headers:{'X-Editor-Token':token}});
    if(!response.ok)return;
    const value=await response.json();state.playtest=value.playtest;renderPlaytest();
    if(!value.playtest.running)status('Playtest exited with code '+value.playtest.exit_code+'. See Playtest diagnostics.',value.playtest.exit_code!==0);
  } catch(error) {status(error.message,true);}
},1000);
