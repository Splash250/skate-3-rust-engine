'use strict';
let token = '', auditAfter = 0, accountAfter = '';
const element = id => document.getElementById(id);
const message = text => { element('message').textContent = text; };
async function api(path, body) {
  const requestToken=token;
  const response = await fetch(path, {method: body === undefined ? 'GET' : 'POST', headers: {'Content-Type': 'application/json', ...(requestToken ? {'Authorization': `Bearer ${requestToken}`} : {})}, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'omit', cache: 'no-store'});
  const value = await response.json();
  if(requestToken!==token)throw new Error('Session changed; please retry.');
  if (!response.ok) throw new Error(value.error?.message || `Request failed (${response.status})`);
  return value;
}
const show = (id, value) => { element(id).textContent = JSON.stringify(value, null, 2); };
async function refresh() {
  accountAfter = ''; await loadAccounts();
  for (const id of ['status', 'roles']) {
    try { const value=await api(`/v1/admin/${id}`);show(id,value);if(id==='status')resourceOptions(value.resource_platform?.resources||[]); } catch (error) { element(id).textContent = error.message; }
  }
  try { const records = await api(`/v1/admin/audit?after=${auditAfter}`); show('audit', records); if (records.length) auditAfter = records[records.length - 1].id; } catch (error) { element('audit').textContent = error.message; }
}
element('login').addEventListener('submit', async event => {
  event.preventDefault(); const form = event.target;
  try { const result = await api('/v1/login', {username: form.username.value, password: form.password.value}); token = result.token; form.password.value = ''; element('admin').hidden = false; element('login-panel').hidden = true; message('Signed in.'); await refresh(); } catch (error) { message(error.message); }
});
element('logout').addEventListener('click', () => { token = ''; auditAfter = 0; element('admin').hidden = true; element('login-panel').hidden = false; message('Signed out of this browser. Use revoke account sessions to invalidate all issued sessions.'); });
async function loadAccounts() {try {const records = await api(`/v1/admin/accounts?after=${accountAfter}`); show('accounts', records); if(records.length) accountAfter=records[records.length-1].username;}catch(error){element('accounts').textContent=error.message;}}
element('accounts-next').addEventListener('click', loadAccounts);
element('refresh').addEventListener('click', refresh); element('audit-next').addEventListener('click', refresh);
element('host').addEventListener('submit', async event => {
  event.preventDefault(); const form = event.target; const kind = form.kind.value;
  try {
    const result = await api('/v1/admin/actions', {kind, [kind === 'kick' ? 'actor' : 'resource']: form.target.value}); show('ticket', result);
    let attempts = 0; const poll = async () => { try {const state = await api(`/v1/admin/actions/${result.ticket}`); show('ticket', state); if (state.state === 'queued' && ++attempts < 30 && token) setTimeout(poll, 1000); else await refresh();} catch (error) {message(error.message);} }; setTimeout(poll, 500);
  } catch (error) { message(error.message); }
});
const templates = {accounts:{username:'player',password:''},roles:{role:'moderator',parent:null},'role-parent':{role:'moderator',parent:'viewer',grant:true},'role-permission':{role:'moderator',permission:'players.kick',grant:true},'account-role':{account:'',role:'moderator',grant:true},ban:{account:'',banned:true,reason:''},whitelist:{account:'',allowed:true},'whitelist-mode':{enabled:true},revoke:{account:''}};
const mutation = element('mutation'); const update = () => {mutation.body.value = JSON.stringify(templates[mutation.route.value], null, 2);}; mutation.route.addEventListener('change', update); update();
mutation.addEventListener('submit', async event => {event.preventDefault(); try {await api(`/v1/admin/${mutation.route.value}`, JSON.parse(mutation.body.value)); if (mutation.route.value === 'accounts') update(); message('Change applied and audited.'); await refresh();} catch (error) {message(error.message);}});

const operationTemplates = {
  maintenance:{reason:'Scheduled server restart',delay_ms:60000,restart:true},resume:{},capacity:{players:16},
  backup:{snapshot:'before-upgrade'},restore:{snapshot:'before-upgrade'},settings_read:{resource:''},
  settings_set:{resource:'',key:'',value:null},profile_read:{resource:null},profile_export:{}
};
const operations = element('operations');
const updateOperation = () => {operations.body.value=JSON.stringify(operationTemplates[operations.kind.value],null,2);};
operations.kind.addEventListener('change',updateOperation);updateOperation();
let traceUrl;

async function queueOperation(action) {
  const owner=token;
  const queued=await api('/v1/admin/actions',action);show('operation-result',queued);
  for(let attempt=0;attempt<60;attempt++) {
    await new Promise(resolve=>setTimeout(resolve,500));
    if(!owner || owner!==token)throw new Error('Session changed; please retry.');
    const result=await api(`/v1/admin/actions/${queued.ticket}`);
    if(result.state==='queued')continue;
    if(!result.ok)throw new Error(result.error?.message||'Operation failed');
    return result;
  }
  throw new Error('Operation remains queued. Refresh status before retrying.');
}
async function performOperation(action) {
  const result=await queueOperation(action);show('operation-result',result);
  if(action.kind==='profile_export') {
    if(traceUrl)URL.revokeObjectURL(traceUrl);
    traceUrl=URL.createObjectURL(new Blob([result.value],{type:'application/json'}));
    element('trace-download').href=traceUrl;element('trace-download').hidden=false;
    message('Trace ready. Use the download link below the operation result.');
  } else if(action.kind==='settings_read') {
    renderSettings(action.resource,JSON.parse(result.value));element('operation-result').textContent='Settings loaded.';
  } else if(action.kind==='profile_read') {
    renderProfile(JSON.parse(result.value));element('operation-result').textContent='Performance history refreshed.';
  }
  return result;
}
operations.addEventListener('submit',async event=>{
  event.preventDefault();
  try {await performOperation({...JSON.parse(operations.body.value),kind:operations.kind.value});await refresh();}
  catch(error){message(error.message);}
});
function make(tag,text,attributes={}) {
  const item=document.createElement(tag);if(text!==undefined)item.textContent=text;
  for(const [name,value] of Object.entries(attributes))item.setAttribute(name,value);
  return item;
}
function resourceOptions(resources) {
  element('resource-options').replaceChildren(...resources.map(resource=>make('option',undefined,{value:resource.id})));
}
element('load-settings').addEventListener('submit',async event=>{
  event.preventDefault();try{await performOperation({kind:'settings_read',resource:event.target.resource.value});}catch(error){message(error.message);}
});
element('load-profile').addEventListener('submit',async event=>{
  event.preventDefault();try{await performOperation({kind:'profile_read',resource:event.target.resource.value||null});}catch(error){message(error.message);}
});
element('export-profile').addEventListener('click',async()=>{
  try{await performOperation({kind:'profile_export'});}catch(error){message(error.message);}
});
function renderSettings(resource,settings) {
  const container=element('settings-editor');container.replaceChildren(make('h3',`Settings · ${resource}`));container.hidden=false;
  if(!Object.keys(settings).length){container.append(make('p','This resource declares no operator settings.'));return;}
  for(const [key,status] of Object.entries(settings)) {
    const definition=status.definition,privateValue=definition.visibility==='private',pending=status.pending!==null&&status.pending!==undefined;
    const fieldset=make('fieldset'),legend=make('legend',key),form=make('form'),label=make('label',pending?'Pending restart value':'Value');
    let input;
    if(definition.type==='enum') {
      input=make('select');for(const option of definition.options)input.append(make('option',option,{value:option}));
    } else {
      input=make('input');input.type=definition.type==='boolean'?'checkbox':(['integer','number'].includes(definition.type)?'number':(privateValue?'password':'text'));
      if(input.type==='number') {input.step=definition.type==='integer'?'1':'any';if(definition.min!==undefined)input.min=definition.min;if(definition.max!==undefined)input.max=definition.max;input.required=true;}
      if(definition.type==='string')input.maxLength=definition.max_bytes;
    }
    const value=pending?status.pending:status.value;
    if(definition.type==='boolean')input.checked=!!value;else input.value=value;
    label.append(input);form.append(label);
    if(privateValue&&definition.type==='string') {
      const reveal=make('button','Show value',{type:'button','aria-pressed':'false'});
      reveal.addEventListener('click',()=>{const visible=input.type==='password';input.type=visible?'text':'password';reveal.textContent=visible?'Hide value':'Show value';reveal.setAttribute('aria-pressed',String(visible));});form.append(reveal);
    }
    const save=make('button',definition.change==='restart'?'Save for restart':'Apply live',{type:'submit'});form.append(save);
    const bounds=[];if(definition.min!==undefined)bounds.push(`minimum ${definition.min}`);if(definition.max!==undefined)bounds.push(`maximum ${definition.max}`);if(definition.type==='string'||definition.type==='enum')bounds.push(`maximum ${definition.max_bytes} UTF-8 bytes`);
    const details=`${definition.type} · ${definition.visibility==='private'?'server private':definition.visibility} · ${definition.change==='restart'?'requires resource restart':'changes live'}${bounds.length?' · '+bounds.join(', '):''}`;
    fieldset.append(legend,make('p',details),make('p',`Default: ${privateValue?'hidden private value':JSON.stringify(definition.default)}`));
    if(pending)fieldset.append(make('p',`A restart is pending. Active value: ${privateValue?'hidden private value':JSON.stringify(status.value)}`));
    fieldset.append(form);container.append(fieldset);
    form.addEventListener('submit',async event=>{
      event.preventDefault();save.disabled=true;
      try {
        let updated=definition.type==='boolean'?input.checked:(['integer','number'].includes(definition.type)?Number(input.value):input.value);
        if(typeof updated==='number'&&!Number.isFinite(updated))throw new Error('Enter a finite number.');
        if(typeof updated==='string'&&new TextEncoder().encode(updated).length>definition.max_bytes)throw new Error(`Value exceeds ${definition.max_bytes} UTF-8 bytes.`);
        await queueOperation({kind:'settings_set',resource,key,value:updated});
        await performOperation({kind:'settings_read',resource});message(`${key} saved${definition.change==='restart'?'; restart the resource to apply it':'.'}`);
      }catch(error){message(error.message);}finally{save.disabled=false;}
    });
  }
}
const milliseconds=value=>value===null||value===undefined?'Unavailable':`${(value/1000).toFixed(3)} ms`;
function table(caption,headers,rows) {
  const wrapper=make('div',undefined,{class:'table-scroll',tabindex:'0'}),result=make('table');result.append(make('caption',caption));
  const head=make('thead'),header=make('tr');for(const name of headers)header.append(make('th',name,{scope:'col'}));head.append(header);result.append(head);
  const body=make('tbody');for(const row of rows){const tr=make('tr');for(const cell of row)tr.append(make('td',String(cell)));body.append(tr);}result.append(body);wrapper.append(result);return wrapper;
}
function renderProfile(snapshot) {
  const container=element('profile-view');container.hidden=false;container.replaceChildren(make('h3','Resource performance history'));
  container.append(make('p',snapshot.timing_contract),make('p',`Profiling ${snapshot.enabled?'enabled':'disabled'} · capacity ${snapshot.capacity} spans · retention ${snapshot.retention_ms/1000}s · ${snapshot.evicted} evicted.`));
  const summaries=[...(snapshot.summaries||[])].sort((a,b)=>b.p95_wall_time_us-a.p95_wall_time_us);
  container.append(table('Retained samples, sorted by p95 wall time. CPU columns are measured separately.',
    ['Resource / generation','Callback','Samples','p50 wall','p95 wall','p99 wall','Maximum wall','Exclusive host CPU','Worker CPU'],
    summaries.map(s=>[`${s.resource} / ${s.generation}`,s.phase,s.samples,milliseconds(s.p50_wall_time_us),milliseconds(s.p95_wall_time_us),milliseconds(s.p99_wall_time_us),milliseconds(s.max_wall_time_us),milliseconds(s.exclusive_host_cpu_time_us),milliseconds(s.worker_cpu_time_us)])));
  const spans=[...(snapshot.spans||[])].sort((a,b)=>a.start_us-b.start_us);
  container.append(table('Recent timeline. Shared start times and parent IDs correlate callbacks with host tick stalls; inclusive durations overlap.',
    ['Start','Resource / generation','Phase / source','Span / parent','Wall','Host CPU inclusive','Host CPU exclusive','Worker CPU','Queue wait','IPC receive wait','Result'],
    spans.map(s=>[milliseconds(s.start_us),`${s.resource} / ${s.generation}`,`${s.phase}${s.source?' · '+s.source:''}`,`${s.id} / ${s.parent??'—'}`,milliseconds(s.wall_time_us),milliseconds(s.host_cpu_time_us),milliseconds(s.exclusive_host_cpu_time_us),milliseconds(s.worker_cpu_time_us),milliseconds(s.queue_wait_us),milliseconds(s.ipc_receive_wait_us),s.failed?'Failed':'OK'])));
  if(snapshot.history_omitted)container.append(make('p',`${snapshot.history_omitted} older spans omitted from this view (${snapshot.retained_spans} retained). Export the trace for a larger bounded timeline.`));
  if(snapshot.summaries_omitted)container.append(make('p',`${snapshot.summaries_omitted} summaries omitted to keep this response bounded.`));
}
element('logout').addEventListener('click',()=>{
  if(traceUrl){URL.revokeObjectURL(traceUrl);traceUrl=undefined;}
  element('trace-download').hidden=true;element('trace-download').removeAttribute('href');
  element('operation-result').textContent='';element('ticket').textContent='';
  for(const id of ['settings-editor','profile-view']){element(id).replaceChildren();element(id).hidden=true;}
  updateOperation();update();
  for(const id of ['status','accounts','roles','audit'])element(id).textContent='';
});
