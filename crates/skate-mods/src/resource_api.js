// Original compatibility adapter. Each call reaches the shared permission and
// generation-aware resource API; no file/network/native module globals exist.
(function () {
    'use strict';
    const host = globalThis.__resource_host;
    const registerHost = globalThis.__resource_register;
    const metadata = JSON.parse(globalThis.__resource_metadata);
    delete globalThis.__resource_host;
    delete globalThis.__resource_register;
    delete globalThis.__resource_metadata;
    const stringify = JSON.stringify, parse = JSON.parse;
    const prototypeOf = Object.getPrototypeOf, plainPrototype = Object.prototype;
    const isArray = Array.isArray;
    // Capture methods before user code can mutate shared JS intrinsics.
    const mapGet=Function.call.bind(Map.prototype.get);
    const mapSet=Function.call.bind(Map.prototype.set);
    const mapDelete=Function.call.bind(Map.prototype.delete);
    const mapHas=Function.call.bind(Map.prototype.has);
    const mapEntries=Function.call.bind(Map.prototype.entries);
    const mapSize=Function.call.bind(Object.getOwnPropertyDescriptor(Map.prototype,'size').get);
    const callbacks = new Map();
    let nextCallback = 0;
    function serialize(value) {
        if (value === undefined) value = null;
        const active = new Set();
        let nodes = 0;
        function check(v, depth) {
            if (depth > 32 || ++nodes > metadata.maxValueBytes * 2 + 128) throw Error('payload nesting or node limit exceeded');
            if (v === null || typeof v === 'string' || typeof v === 'boolean') return;
            if (typeof v === 'number' && Number.isFinite(v)) return;
            if (typeof v !== 'object') throw Error('payload must contain only finite JSON values');
            const prototype = prototypeOf(v);
            if (!isArray(v) && prototype !== plainPrototype && prototype !== null) {
                throw Error('payload must use plain JSON objects; Promise and class results are unsupported');
            }
            if (active.has(v)) throw Error('cyclic payload');
            active.add(v);
            for (const key of Object.keys(v)) check(v[key], depth + 1);
            active.delete(v);
        }
        check(value, 0);
        const result = stringify(value);
        if (result.length > metadata.maxValueBytes * 2 + 1024) throw Error('payload exceeds byte limit');
        return result;
    }
    function call(method, ...args) { return parse(host(method, serialize(args))); }
    function register(kind, name, callback, permission = '') {
        if (typeof callback !== 'function') throw Error('callback must be a function');
        const replaced=[];
        if(kind==='export') for(const [id,entry] of mapEntries(callbacks)) {
            if(entry.kind==='export'&&entry.name===name) replaced.push(id);
        }
        if (mapSize(callbacks)-replaced.length >= metadata.maxCallbacks + 8) throw Error('JavaScript callback limit reached');
        const id = ++nextCallback;
        registerHost(kind, name, id, permission);
        for(const old of replaced) mapDelete(callbacks,old);
        mapSet(callbacks,id,{kind,name,callback});
    }
    Object.defineProperty(globalThis, '__resource_dispatch', {value(id, args) {
        const entry = mapGet(callbacks,id);
        if (!entry) throw Error('unknown JavaScript callback');
        return serialize(entry.callback(...parse(args)));
    }});
    const resource = {
        id: metadata.id, version: metadata.version, side: metadata.side,
        generation: metadata.generation, grants: Object.freeze(metadata.grants),
        on(name, fn) { register('on', name, fn); },
        onNet(name, fn) { register('on_net', name, fn); },
        emit(name, payload) { return call('resource.emit', name, payload === undefined ? null : payload); },
        send(name, payload, recipient = null, scope = null) { return call('resource.send', name, payload === undefined ? null : payload, recipient, scope); },
        off(name) {
            const result=call('resource.off', name);
            for(const [id,entry] of mapEntries(callbacks)) {
                if((entry.kind==='on'||entry.kind==='on_net')&&entry.name===name) mapDelete(callbacks,id);
            }
            return result;
        },
        export(name, fn) { register('export', name, fn); },
        call(dependency, name, payload) { return call('resource.call', dependency, name, payload === undefined ? null : payload); },
        command(name, permission, fn) { register('command', name, fn, permission); },
        // Lua's empty sequence crosses the shared adapter as an empty object.
        // Preserve the public list contract before the first player joins too.
        players() { const value=call('resource.players'); return isArray(value) ? value : []; },
        lifecycle(handlers) { for (const name of Object.keys(handlers)) register('lifecycle', name, handlers[name]); },
        entities: { command(value) { return call('resource.entities.command', value); }, all() { return call('resource.entities.all'); } },
        entity(value) { return call('resource.entities.command', value); },
        voice: { submit(operation) { return call('resource.voice.submit', operation); } },
        world: { command(operation) { return call('resource.world.command', operation); } },
        competition: { submit(operation) { return call('resource.competition.submit', operation); } },
        transfer: { start(key,name,payload,options=null) { return call('resource.transfer.start',key,name,payload,options); }, cancel(key) { return call('resource.transfer.cancel',key); } },
        teleport(player, destination) { return call('resource.teleport', player, destination); },
        state: {get(key, scope = null) { return call('resource.state.get', key, scope); }, set(key, value, scope = null) { return call('resource.state.set', key, value === undefined ? null : value, scope); }},
        settings: {get(key) { return call('resource.settings.get', key); }, all() { return call('resource.settings.all'); }},
        storage: {get(key) { return call('resource.storage.get', key); }, set(key, value) { return call('resource.storage.set', key, value === undefined ? null : value); }},
        services: {
            submit(key, operation, timeoutMs) { return call('resource.services.submit', key, operation, timeoutMs); },
            cancel(key) { return call('resource.services.cancel', key); }
        }
    };
    resource.on_net = resource.onNet;
    globalThis.resource = resource;
    globalThis.sdk = {
        resource,
        log(text) { return call('sdk.log', text); },
        readText(path) { return call('sdk.read_text', path); },
        submit(command) { return call('sdk.submit', command); },
        animation: { version: 1, submit(operation) { return call('sdk.submit', {kind:'animation',version:1,operation}); } },
        ui: {
            text(key, text) { return call('sdk.ui.text', key, text); },
            remove(key) { return call('sdk.ui.remove', key); },
            menu(key, options) { return call('sdk.ui.menu', key, options); },
            canvas(key, options) { return call('sdk.ui.canvas', key, options); }
        }
    };
    if (resource.side === 'server') delete sdk.ui;
    globalThis.on = resource.on;
    globalThis.onNet = resource.onNet;
    globalThis.emit = resource.emit;
    globalThis.emitNet = resource.side === 'server'
        ? (name, recipient, payload) => resource.send(name, payload, recipient === -1 ? null : recipient)
        : (name, payload) => resource.send(name, payload);
    globalThis.exports = new Proxy((name, callback) => resource.export(name, callback), {
        get(_, dependency) { return new Proxy({}, {get(_, name) { return payload => resource.call(dependency, name, payload); }}); }
    });
    let clock = 0, nextTimer = 0;
    const timers = new Map(), ticks = new Map();
    globalThis.setTimeout = (callback, milliseconds = 0) => {
        if (typeof callback !== 'function' || !Number.isFinite(milliseconds) || milliseconds < 0 || milliseconds > 86400000) throw Error('invalid timeout');
        if (mapSize(timers) >= metadata.maxThreads) throw Error('JavaScript timer limit reached');
        const id = ++nextTimer;
        mapSet(timers,id, {at: clock + milliseconds, callback}); return id;
    };
    globalThis.clearTimeout = id => mapDelete(timers,id);
    globalThis.setTick = callback => {
        if (typeof callback !== 'function' || mapSize(ticks) >= metadata.maxThreads) throw Error('invalid tick or tick limit reached');
        const id = ++nextTimer; mapSet(ticks,id, callback); return id;
    };
    globalThis.clearTick = id => mapDelete(ticks,id);
    globalThis.GetGameTimer = () => Math.floor(clock);
    resource.lifecycle({on_update(payload) {
        clock += payload.dt * 1000;
        const due = [...mapEntries(timers)].filter(([, timer]) => timer.at <= clock);
        const runnable = [...mapEntries(ticks)];
        for (const [id, timer] of due) { if (mapDelete(timers,id)) timer.callback(); }
        for (const [id, callback] of runnable) { if (mapHas(ticks,id)) callback(); }
    }});
})();
