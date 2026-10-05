// Trusted fixed DOM input adapter. Only the owning host sends bounded values.
// It offers ordinary HTML controls and a keyboard event contract for custom UI.
(()=>{'use strict';
const controls=()=>Array.from(document.querySelectorAll('button,input,select,textarea,a[href],[tabindex]')).filter(e=>!e.disabled&&e.tabIndex>=0&&e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden');
const focus=e=>{if(e){e.focus({preventScroll:true});e.scrollIntoView({block:'nearest',inline:'nearest'});}};
const numericText=new WeakMap(),numericSelected=new WeakSet();
const editNumber=(e,text,back=false)=>{if(e.disabled||e.readOnly)return;let current=numericText.get(e)??e.value;if(numericSelected.delete(e))current='';current=back?current.slice(0,-1):(current+text).slice(0,64);if(!/^[+\-]?(?:[0-9]*(?:\.[0-9]*)?)(?:[eE][+\-]?[0-9]*)?$/.test(current))return;numericText.set(e,current);e.value=current;e.dispatchEvent(new Event('input',{bubbles:true}));};
const key=(name,shift=false)=>{
 const e=document.activeElement;if(name==='SelectAll'){if(e.matches('input[type=number]'))numericSelected.add(e);else if(typeof e.select==='function')e.select();return;}const event=new KeyboardEvent('keydown',{key:name,bubbles:true,cancelable:true,shiftKey:shift});
 if(!e.dispatchEvent(event))return;
 if(name==='Tab'){const list=controls();let i=list.indexOf(e);focus(list[(i+(shift?-1:1)+list.length)%list.length]);}
 else if(e.matches('select')&&!e.disabled&&['ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(name)){e.selectedIndex=Math.max(0,Math.min(e.options.length-1,e.selectedIndex+(['ArrowUp','ArrowLeft'].includes(name)?-1:1)));e.dispatchEvent(new Event('change',{bubbles:true}));}
 else if(name.startsWith('Arrow'))window.__skateInput({type:'navigate',direction:name.slice(5).toLowerCase()});
 else if(name==='Enter'||name===' '){if(e.matches('button,a,[role=button],input[type=checkbox],input[type=radio]'))e.click();}
 else if(e.matches('input[type=number]')&&['Backspace','Delete'].includes(name))editNumber(e,'',true);
 else if(e.matches('input:not([type=checkbox]):not([type=radio]),textarea')&&!e.disabled&&!e.readOnly&&['Backspace','Delete'].includes(name)){
   try{let a=e.selectionStart,b=e.selectionEnd;if(a===b){if(name==='Backspace')a=Math.max(0,a-1);else b++;}e.setRangeText('',a,b,'end');e.dispatchEvent(new Event('input',{bubbles:true}));}catch(_){}}
};
Object.defineProperty(window,'__skateInput',{value:Object.freeze(v=>{
 if(v.type==='pointer'){
   const e=document.elementFromPoint(v.x,v.y);if(!e)return;
   e.dispatchEvent(new MouseEvent('mousemove',{clientX:v.x,clientY:v.y,bubbles:true}));
   if(v.click){const c=e.closest('button,input,select,textarea,a,[tabindex]');if(c)focus(c);const target=c||e;if(typeof target.click==='function')target.click();else target.dispatchEvent(new MouseEvent('click',{clientX:v.x,clientY:v.y,bubbles:true,cancelable:true}));}
 }else if(v.type==='wheel'){
   let e=document.activeElement;while(e&&e!==document.body&&e.scrollHeight<=e.clientHeight)e=e.parentElement;
   (e||document.scrollingElement).scrollBy({top:v.delta,behavior:'auto'});
 }else if(v.type==='key')key(v.key,v.shift);
 else if(v.type==='text'){
   const e=document.activeElement;if(!e.matches('input,textarea')||e.disabled||e.readOnly)return;
   if(e.type==='number'){editNumber(e,v.text);return;}
   try{e.setRangeText(v.text,e.selectionStart,e.selectionEnd,'end');e.dispatchEvent(new Event('input',{bubbles:true}));}catch(_){}
 }else if(v.type==='navigate'){
   if(v.direction==='accept')return key('Enter');if(v.direction==='back')return key('Escape');
   const list=controls();const active=document.activeElement;
   if(['left','right'].includes(v.direction)){
     if(active.matches('select'))return key(v.direction==='left'?'ArrowLeft':'ArrowRight');
     if(active.matches('input[type=number],input[type=range]')&&!active.disabled&&!active.readOnly){try{if(v.direction==='left')active.stepDown();else active.stepUp();numericText.delete(active);active.dispatchEvent(new Event('input',{bubbles:true}));active.dispatchEvent(new Event('change',{bubbles:true}));}catch(_){}return;}
     if(active.matches('input[type=checkbox]')&&!active.disabled){active.checked=v.direction==='right';active.dispatchEvent(new Event('input',{bubbles:true}));active.dispatchEvent(new Event('change',{bubbles:true}));return;}
   }
   if(!list.includes(active))return focus(list[0]);
   const r=active.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;
   const horizontal=['left','right'].includes(v.direction),sign=['left','up'].includes(v.direction)?-1:1;
   const choices=list.filter(e=>e!==active).map(e=>{const q=e.getBoundingClientRect(),dx=q.x+q.width/2-x,dy=q.y+q.height/2-y;return {e,main:(horizontal?dx:dy)*sign,cross:Math.abs(horizontal?dy:dx)};}).filter(q=>q.main>1).sort((a,b)=>(a.main+a.cross*3)-(b.main+b.cross*3));
   if(choices.length)focus(choices[0].e);
 }
})});
})();
