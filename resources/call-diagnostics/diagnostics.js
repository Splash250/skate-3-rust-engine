(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const post = action => window.skate.postMessage({action});
  const node = (tag, text, className) => { const value=document.createElement(tag); value.textContent=text; if(className)value.className=className; return value; };
  let authorizedUntil=0;
  function clear(message) {
    authorizedUntil=0; $('counts').hidden=true; $('counts').replaceChildren(); $('result').replaceChildren(node('p','A separate calls.test permission is required.','muted'));
    $('capacity').textContent='Counts include only this call plugin. Participant identities stay private.'; $('test').disabled=true; $('status').textContent=message;
  }
  $('close').onclick=()=>post('close');
  $('refresh').onclick=()=>{ $('status').textContent='Refreshing live call state…';post('inspect'); };
  $('test').onclick=()=>{ $('test').disabled=true;$('result').replaceChildren(node('p','Waiting for the server consistency check…'));post('test'); };
  window.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();post('close');}});
  window.addEventListener('message',event=>{
    const value=event.data;if(!value||typeof value!=='object')return;
    if(value.kind==='expired'){clear('Access could not be refreshed. Rechecking current permissions…');return;}
    if(value.kind==='error'){$('status').textContent=value.message;return;}
    if(value.kind!=='result')return;
    if(!value.ok){if(value.denied)clear(value.error||'Access denied.');else $('status').textContent=value.error||'Request failed.';return;}
    authorizedUntil=performance.now()+2000; $('test').disabled=!value.can_test;
    if(!value.can_test)$('result').replaceChildren(node('p','A separate calls.test permission is required.','muted'));
    if(value.operation==='inspect') {
      const counts=value.value||{};$('counts').replaceChildren();$('counts').hidden=false;
      for(const [key,label] of [['ringing','Ringing'],['connecting','Connecting'],['active','Connected'],['retiring','Retiring channels']]) {
        const item=node('div','','count');item.append(node('strong',String(counts[key]||0)),node('span',label));$('counts').append(item);
      }
      $('capacity').textContent=`${counts.used} of ${counts.capacity} call slots in use. Counts exclude participant identities.`;
      $('status').textContent='Live state verified. Refreshes automatically while open.';
    } else if(value.operation==='test') {
      const test=value.value||{};const root=$('result');root.replaceChildren(node('strong',test.ok?'Passed':'Invariant failure'),node('p',`${test.checked||0} checks against current server state.`));
      if(Array.isArray(test.errors)&&test.errors.length){const list=document.createElement('ul');for(const error of test.errors.slice(0,16))list.append(node('li',String(error)));root.append(list);}
      $('status').textContent='Consistency check completed without changing call or voice state.';
    }
  });
  setInterval(()=>{if(authorizedUntil&&performance.now()>authorizedUntil)clear('Access expired while waiting for the server.');},250);
})();
