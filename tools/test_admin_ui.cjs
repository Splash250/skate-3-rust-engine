// Dependency-free DOM contract tests. Run with Node.js: node tools/test_admin_ui.cjs
// Real TLS permissions, redaction and persistence are covered in server/tests/accounts.rs.
'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
class Element {
  constructor(tag='div'){this.tagName=tag;this.children=[];this.listeners={};this.attributes={};this.hidden=false;this.value='';this._text='';}
  set textContent(value){this._text=String(value);this.children=[];}
  get textContent(){return this._text+this.children.map(child=>child.textContent).join('');}
  append(...values){this.children.push(...values);}
  replaceChildren(...values){this._text='';this.children=values;}
  setAttribute(name,value){this.attributes[name]=String(value);}
  removeAttribute(name){delete this.attributes[name];}
  addEventListener(name,handler){(this.listeners[name]??=[]).push(handler);}
  async dispatch(name){for(const handler of this.listeners[name]||[])await handler({target:this,preventDefault(){}});}
}
const elements={};
for(const [,tag,id] of fs.readFileSync('crates/skate-accounts/src/admin.html','utf8').matchAll(/<(\w+)[^>]*\bid="([^"]+)"/g))elements[id]=new Element(tag);
for(const [id,names] of Object.entries({login:['username','password'],host:['kind','target'],mutation:['route','body'],operations:['kind','body'],'load-settings':['resource'],'load-profile':['resource']}))
  for(const name of names)elements[id][name]=new Element('input');
elements.mutation.route.value='accounts';elements.operations.kind.value='maintenance';
const revoked=[],context=vm.createContext({document:{getElementById:id=>{assert.ok(elements[id],`Unknown element ${id}`);return elements[id];},createElement:tag=>new Element(tag)},TextEncoder,Blob,URL:{revokeObjectURL:url=>revoked.push(url),createObjectURL:()=> 'blob:test'},setTimeout,fetch:()=>{throw Error('Unexpected network request');}});
vm.runInContext(fs.readFileSync('crates/skate-accounts/src/admin.js','utf8'),context);
const run=code=>vm.runInContext(code,context);
const descendants=node=>[node,...node.children.flatMap(descendants)];
const find=(node,predicate)=>descendants(node).find(predicate);
async function main(){
  run(`renderSettings('rounds',{
    duration:{definition:{type:'integer',visibility:'public',change:'restart',default:20,min:1,max:90,max_bytes:256},value:20,pending:30},
    secret:{definition:{type:'string',visibility:'private',change:'live',default:'default-private-sentinel',max_bytes:64},value:'private-sentinel',pending:null},
    enabled:{definition:{type:'boolean',visibility:'replicated',change:'live',default:false,max_bytes:256},value:true,pending:null},
    mode:{definition:{type:'enum',visibility:'public',change:'live',default:'solo',options:['solo','free'],max_bytes:8},value:'free',pending:null}
  });`);
  const editor=elements['settings-editor'];
  assert.match(editor.textContent,/minimum 1, maximum 90/);assert.match(editor.textContent,/Active value: 20/);
  assert.ok(!editor.textContent.includes('private-sentinel'));
  const fieldsets=editor.children.filter(item=>item.tagName==='fieldset');assert.equal(fieldsets.length,4);
  const duration=find(fieldsets[0],item=>item.tagName==='input');assert.equal(duration.type,'number');assert.equal(duration.value,30);assert.equal(duration.step,'1');
  const secret=find(fieldsets[1],item=>item.tagName==='input');assert.equal(secret.type,'password');assert.equal(secret.value,'private-sentinel');
  await find(fieldsets[1],item=>item.tagName==='button'&&item.textContent==='Show value').dispatch('click');assert.equal(secret.type,'text');
  assert.equal(find(fieldsets[2],item=>item.tagName==='input').checked,true);
  assert.equal(find(fieldsets[3],item=>item.tagName==='select').value,'free');
  run('globalThis.captured=[];queueOperation=async action=>captured.push(action);performOperation=async action=>captured.push(action);');
  duration.value='45';await find(fieldsets[0],item=>item.tagName==='form').dispatch('submit');
  assert.deepEqual(JSON.parse(run('JSON.stringify(captured)')),[{kind:'settings_set',resource:'rounds',key:'duration',value:45},{kind:'settings_read',resource:'rounds'}]);
  secret.value='é'.repeat(33);await find(fieldsets[1],item=>item.tagName==='form').dispatch('submit');
  assert.match(elements.message.textContent,/exceeds 64 UTF-8 bytes/);assert.equal(run('captured.length'),2);
  run(`renderProfile({enabled:true,capacity:4096,retention_ms:60000,evicted:8,timing_contract:'Inclusive durations overlap.',retained_spans:900,history_omitted:899,summaries_omitted:2,
    summaries:[{resource:'slow',generation:3,phase:'tick',samples:2,p50_wall_time_us:1000,p95_wall_time_us:2000,p99_wall_time_us:2100,max_wall_time_us:2100,exclusive_host_cpu_time_us:300,worker_cpu_time_us:null}],
    spans:[{resource:'slow',generation:3,phase:'tick',source:'server.lua',id:4,parent:2,start_us:5000,wall_time_us:2000,host_cpu_time_us:900,exclusive_host_cpu_time_us:300,worker_cpu_time_us:null,queue_wait_us:200,ipc_receive_wait_us:null,failed:false}]});`);
  const profile=elements['profile-view'];assert.match(profile.textContent,/2\.000 ms/);assert.match(profile.textContent,/slow \/ 3/);assert.match(profile.textContent,/server.lua/);
  assert.match(profile.textContent,/Unavailable/);assert.match(profile.textContent,/899 older spans/);assert.match(profile.textContent,/2 summaries omitted/);
  run("token='authorized';traceUrl='blob:private';");await elements.logout.dispatch('click');
  assert.equal(run('token'),'');assert.deepEqual(revoked,['blob:private']);assert.equal(editor.children.length,0);assert.ok(editor.hidden);assert.equal(profile.children.length,0);
  assert.equal(elements['operation-result'].textContent,'');assert.equal(elements.operations.body.value.includes('private-sentinel'),false);
  let resolve;context.fetch=()=>new Promise(done=>{resolve=done;});run("token='old-session';globalThis.pendingRead=api('/v1/admin/actions/1');");
  run("token='new-session';");resolve({ok:true,json:async()=>({value:'private-sentinel'})});
  await assert.rejects(run('pendingRead'),/Session changed/);
  console.log('Admin UI contracts passed: typed controls, bounds, pending updates, timeline values, logout cleanup and in-flight session isolation.');
}
main().catch(error=>{console.error(error);process.exitCode=1;});
