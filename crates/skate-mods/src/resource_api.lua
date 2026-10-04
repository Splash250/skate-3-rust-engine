-- Resource teardown is engine-owned. User finalizers would run outside callback
-- budgets and can reenter Rust while Lua closes, including during failed startup.
local raw_setmetatable, raw_get, raise = setmetatable, rawget, error
function setmetatable(value, meta)
    if meta ~= nil and raw_get(meta, '__gc') ~= nil then
        raise('resource finalizers are unsupported; use on_unload', 2)
    end
    return raw_setmetatable(value, meta)
end

-- Resource API 1; familiar aliases retain explicit resource namespaces and grants.
function RegisterNetEvent(name, callback)
    assert(type(callback)=='function', 'RegisterNetEvent requires a callback')
    resource.on_net(name, callback)
end
function AddEventHandler(name, callback) resource.on(name, callback) end
function RemoveEventHandler(name) resource.off(name) end
function TriggerEvent(name, payload) resource.emit(name, payload) end
function TriggerServerEvent(name, payload)
    assert(resource.side=='client','TriggerServerEvent is client-only')
    resource.send(name, payload)
end
function TriggerClientEvent(name, recipient, payload)
    assert(resource.side=='server','TriggerClientEvent is server-only')
    if recipient == -1 then recipient = nil end
    resource.send(name, payload, recipient)
end
function RegisterCommand(name, callback, permission)
    resource.command(name, permission or ('command.'..name), callback)
end
function SetTimeout(milliseconds, callback)
    assert(type(milliseconds)=='number' and milliseconds>=0,'invalid timeout')
    local key='timeout.'..tostring((resource._timer_id or 0)+1)
    resource._timer_id=(resource._timer_id or 0)+1
    sdk.time.after(key,milliseconds/1000,callback)
    return key
end
function ClearTimeout(key) sdk.time.cancel(key) end
function GetCurrentResourceName() return resource.id end
function IsDuplicityVersion() return resource.side=='server' end
exports = setmetatable({}, {
    __call=function(_,name,callback) resource.export(name,callback) end,
    __index=function(_,owner)
        return setmetatable({}, {__index=function(_,name)
            return function(_,payload) return resource.call(owner,name,payload) end
        end})
    end
})

-- Protected calls may catch ordinary script errors, but never resource budget
-- exhaustion. Capture natives before exposing wrappers; debug/load are absent.
local check_budget = sdk._budget_check
sdk._budget_check = nil
local native_pcall, native_xpcall = pcall, xpcall
local pack, unpack = table.pack, table.unpack
function pcall(callback, ...)
    local result = pack(native_pcall(callback, ...))
    check_budget()
    return unpack(result, 1, result.n)
end
function xpcall(callback, handler, ...)
    local result = pack(native_xpcall(callback, handler, ...))
    check_budget()
    return unpack(result, 1, result.n)
end
local create, resume, status, yield = coroutine.create, coroutine.resume, coroutine.status, coroutine.yield
local native_close=coroutine.close
function coroutine.close(thread)
    local result=pack(native_close(thread))
    check_budget()
    return unpack(result,1,result.n)
end
function coroutine.resume(thread, ...)
    local result = pack(resume(thread, ...))
    check_budget()
    return unpack(result, 1, result.n)
end
function coroutine.wrap(callback)
    local thread = create(callback)
    return function(...)
        local result = pack(coroutine.resume(thread, ...))
        if not result[1] then error(result[2], 2) end
        return unpack(result, 2, result.n)
    end
end
local checked_resume = coroutine.resume
local threads, clock, next_thread = {}, 0, 0
local max_threads = sdk._max_threads
sdk._max_threads = nil
function CreateThread(callback)
    assert(type(callback)=='function', 'CreateThread requires a function')
    local count=0; for _ in pairs(threads) do count=count+1 end
    assert(count<max_threads, 'resource thread limit reached')
    next_thread=next_thread+1
    threads[next_thread]={thread=create(callback), at=clock}
    return next_thread
end
function Wait(milliseconds)
    assert(type(milliseconds)=='number' and milliseconds>=0 and milliseconds<=86400000, 'Wait requires 0..86400000 milliseconds')
    return yield(milliseconds)
end
Citizen={CreateThread=CreateThread, Wait=Wait, SetTimeout=SetTimeout}
function GetGameTimer() return math.floor(clock*1000) end
function sdk._resource_due(dt)
    for _,task in pairs(threads) do if task.at<=clock+dt then return true end end
    return false
end
function sdk._resource_advance(dt)
    clock=clock+dt
    local due={}
    for id,task in pairs(threads) do if task.at<=clock then due[#due+1]=id end end
    table.sort(due)
    -- Snapshot runnable tasks: Wait(0), new tasks and catch-up never spin inside
    -- this tick. All resumptions share the VM callback's instruction budget.
    for _,id in ipairs(due) do
        local task=threads[id]
        local ok, milliseconds=checked_resume(task.thread)
        if not ok then error(milliseconds, 0) end
        if status(task.thread)=='dead' then threads[id]=nil
        else
            assert(type(milliseconds)=='number' and milliseconds>=0 and milliseconds<=86400000, 'thread must yield a bounded Wait delay')
            task.at=clock+milliseconds/1000
        end
    end
end

-- Portable finite vector values. They are Lua tables, serialized as x/y/z/w
-- objects, not Cfx's patched-Lua vector type. Metatables contain no host handles.
local axes={'x','y','z','w'}
local function vector_type(dim)
    local meta={}
    local function make(...)
        local values=pack(...)
        assert(values.n==dim, 'wrong vector dimension')
        local v={}
        for i=1,dim do
            local n=values[i]
            assert(type(n)=='number' and n==n and math.abs(n)<math.huge, 'vector components must be finite')
            v[axes[i]]=n
        end
        return setmetatable(v,meta)
    end
    local function operation(a,b,op)
        local values={}
        for i=1,dim do
            local key=axes[i]
            values[i]=op(type(a)=='number' and a or a[key], type(b)=='number' and b or b[key])
        end
        return make(unpack(values))
    end
    meta.__add=function(a,b) return operation(a,b,function(x,y)return x+y end) end
    meta.__sub=function(a,b) return operation(a,b,function(x,y)return x-y end) end
    meta.__mul=function(a,b) return operation(a,b,function(x,y)return x*y end) end
    meta.__div=function(a,b) return operation(a,b,function(x,y)return x/y end) end
    meta.__unm=function(a) return -1*a end
    meta.__len=function(a) local n=0;for i=1,dim do n=n+a[axes[i]]^2 end;return math.sqrt(n) end
    meta.__eq=function(a,b) for i=1,dim do if a[axes[i]]~=b[axes[i]] then return false end end;return true end
    meta.__tostring=function(a) local t={};for i=1,dim do t[i]=tostring(a[axes[i]]) end;return 'vector'..dim..'('..table.concat(t,', ')..')' end
    return make
end
vector2=vector_type(2)
vector3=vector_type(3)
vector4=vector_type(4)
vec2,vec3,vec4=vector2,vector3,vector4
