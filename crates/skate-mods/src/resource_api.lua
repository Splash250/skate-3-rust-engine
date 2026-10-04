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
