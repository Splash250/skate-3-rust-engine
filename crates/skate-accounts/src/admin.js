'use strict';
let token = '', auditAfter = 0, accountAfter = '';
const element = id => document.getElementById(id);
const message = text => { element('message').textContent = text; };
async function api(path, body) {
  const response = await fetch(path, {method: body === undefined ? 'GET' : 'POST', headers: {'Content-Type': 'application/json', ...(token ? {'Authorization': `Bearer ${token}`} : {})}, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'omit', cache: 'no-store'});
  const value = await response.json();
  if (!response.ok) throw new Error(value.error?.message || `Request failed (${response.status})`);
  return value;
}
const show = (id, value) => { element(id).textContent = JSON.stringify(value, null, 2); };
async function refresh() {
  accountAfter = ''; await loadAccounts();
  for (const id of ['status', 'roles']) {
    try { show(id, await api(`/v1/admin/${id}`)); } catch (error) { element(id).textContent = error.message; }
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
