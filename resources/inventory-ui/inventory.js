'use strict';
const byId = id => document.getElementById(id);
let data = null;
let pending = true;
let purchaseUncertain = false;
let purchaseFocus = null;
let deadline;
function status(message, error = false) {
  byId('status').textContent = message;
  byId('status').classList.toggle('error', error);
}
function post(value) {
  if (!window.skate || typeof window.skate.postMessage !== 'function') {
    status('Open this inventory from a connected game resource.', true);
    pending = false;
    return false;
  }
  window.skate.postMessage(value);
  return true;
}
function updateDisabled() {
  for (const button of byId('catalog').querySelectorAll('button')) {
    button.disabled = pending || purchaseUncertain || !data || data.coins < Number(button.dataset.price);
  }
  byId('refresh').disabled = pending;
  byId('catalog').setAttribute('aria-busy', String(pending));
}
function request(value) {
  if (pending) return;
  if (!post(value)) { updateDisabled(); return; }
  purchaseFocus = document.activeElement?.dataset.item || null;
  pending = true;
  status(value.action === 'buy' ? 'Saving your purchase…' : 'Refreshing inventory…');
  updateDisabled();
  deadline = window.setTimeout(() => {
    pending = false;
    if (value.action === 'buy') purchaseUncertain = true;
    status('The server has not replied. Use Refresh to check the result before purchasing again.', true);
    updateDisabled();
    if (purchaseFocus && document.activeElement === document.body) byId('refresh').focus();
    purchaseFocus = null;
  }, 10000);
}
function element(tag, text) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  return node;
}
function render(value) {
  const catalog = byId('catalog');
  catalog.replaceChildren();
  const owned = byId('owned');
  owned.replaceChildren();
  for (const item of value.catalog) {
    const row = element('li');
    const detail = element('div');
    detail.append(element('h3', item.name), element('p', item.description));
    const button = element('button', `${item.price} coins`);
    button.type = 'button';
    button.dataset.price = String(item.price);
    button.dataset.item = item.id;
    button.setAttribute('aria-label', `Buy ${item.name} for ${item.price} coins`);
    button.addEventListener('click', () => request({action:'buy', item:item.id}));
    row.append(detail, button);
    catalog.append(row);
    const quantity = value.owned[item.id] || 0;
    if (quantity > 0) {
      const entry = element('li');
      entry.append(element('span', item.name), element('strong', `× ${quantity}`));
      owned.append(entry);
    }
  }
  byId('empty').hidden = owned.children.length > 0;
  byId('coins').textContent = String(value.coins);
}
window.addEventListener('message', event => {
  const value = event.data;
  if (!value || typeof value !== 'object' || typeof value.ok !== 'boolean') return;
  const focusedItem = document.activeElement?.dataset.item || purchaseFocus;
  const restoreFocus = focusedItem && (document.activeElement === document.body || byId('catalog').contains(document.activeElement));
  purchaseFocus = null;
  window.clearTimeout(deadline);
  pending = false;
  if (value.ok && Array.isArray(value.catalog) && Number.isSafeInteger(value.coins) && value.owned) {
    data = value;
    purchaseUncertain = false;
    render(value);
  } else if (!data) {
    byId('catalog').replaceChildren(element('li', 'Inventory is unavailable. Use Refresh after reconnecting.'));
  }
  status(typeof value.message === 'string' ? value.message : 'Unexpected inventory response. Refresh to try again.', !value.ok);
  updateDisabled();
  if (restoreFocus) {
    const button = [...byId('catalog').querySelectorAll('button')].find(node => node.dataset.item === focusedItem);
    (button && !button.disabled ? button : byId('refresh')).focus({preventScroll:true});
  }
});
byId('refresh').addEventListener('click', () => request({action:'inspect'}));
byId('close').addEventListener('click', () => post({action:'close'}));
window.addEventListener('keydown', event => {
  if (event.key === 'Escape') { event.preventDefault(); post({action:'close'}); }
});
deadline = window.setTimeout(() => {
  pending = false;
  status('Inventory has not connected. Use Refresh to try again.', true);
  updateDisabled();
}, 10000);
updateDisabled();
