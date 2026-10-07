'use strict';
const $=id=>document.getElementById(id);
function post(action,extra={}){window.skate.postMessage({action,...extra})}
let onlinePlayers=[],selectedPlayer='',pickerOpen=false,playerSignature='',latestSnapshot=null;
function refreshPicker(){
  const toggle=$('target-toggle'),menu=$('target-menu');
  const selected=onlinePlayers.find(player=>player.id===selectedPlayer);
  toggle.textContent=(selected?.label||(onlinePlayers.length?'Choose a player…':'No other players online'))+'  ▾';
  toggle.disabled=onlinePlayers.length===0;
  toggle.setAttribute('aria-expanded',String(pickerOpen));
  menu.classList.toggle('hidden',!pickerOpen||onlinePlayers.length===0);
  $('invite').disabled=!selected;
}
function choosePlayer(id){
  if(!onlinePlayers.some(player=>player.id===id))return;
  selectedPlayer=id;pickerOpen=false;refreshPicker();
}
function render(v){
  if(!v||typeof v!=='object')return;
  latestSnapshot=v;
  $('price').textContent='$'+(v.unit?.price??50);
  const lease=!!v.lease;
  $('lease-card').classList.toggle('hidden',!lease);
  $('rent').classList.toggle('hidden',lease);
  $('enter').classList.toggle('hidden',!lease||v.in_property);
  $('exit').classList.toggle('hidden',!v.in_property);
  $('lease-copy').textContent=lease?`Private apartment · Room ${v.lease.instance}`:'Your apartment is ready.';
  $('status').textContent=v.error?`${v.message||'Request failed'} (${v.error})`:v.message|| (v.ready?'Your account balance: $'+(v.balance??'…'):'Connecting to the property service…');
  const players=(Array.isArray(v.online_players)?v.online_players:[])
    .filter(player=>typeof player?.id==='string'&&typeof player.label==='string');
  const signature=JSON.stringify(players.map(player=>[player.id,player.label]));
  if(signature!==playerSignature){
    onlinePlayers=players;
    const menu=$('target-menu');menu.replaceChildren();
    for(const player of players){
      const choice=document.createElement('button');choice.type='button';choice.className='target-choice';choice.setAttribute('role','option');choice.textContent=player.label;choice.onclick=()=>choosePlayer(player.id);menu.append(choice);
    }
    if(!players.some(player=>player.id===selectedPlayer))selectedPlayer='';
    if(!players.length)pickerOpen=false;
    playerSignature=signature;
  }
  refreshPicker();
  const list=$('invite-list');list.replaceChildren();
  const invitations=Array.isArray(v.invitations)?v.invitations:Object.values(v.invitations||{});
  for(const invite of invitations){const row=document.createElement('div');row.className='invite';const name=document.createElement('span');name.textContent=invite.unit+' · '+invite.owner;const button=document.createElement('button');button.textContent='Visit';button.onclick=()=>post('accept_invite',{owner:invite.owner});row.append(name,button);list.append(row)}
  $('invites').classList.toggle('hidden',invitations.length===0);
}
window.addEventListener('message',e=>{if(e.data?.kind==='snapshot')render(e.data.value)});
$('rent').onclick=()=>post('rent',{unit:'studio'});
$('enter').onclick=()=>post('enter');
$('exit').onclick=()=>post('exit');
$('target-toggle').onclick=()=>{if(!onlinePlayers.length)return;pickerOpen=!pickerOpen;refreshPicker()};
$('invite').onclick=()=>{if(selectedPlayer)post('invite',{target:selectedPlayer});};
$('close').onclick=()=>post('close');
