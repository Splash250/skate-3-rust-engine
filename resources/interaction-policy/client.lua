-- One policy for every consumer. Private visibility expires if the authority is lost.
local now, elapsed, serial, latest = 0, 0, 0, ""
local permissions, expires = {}, 0
local checks={}
local function csv(value)
    local result={}
    for item in string.gmatch(value or "", "[^,]+") do result[#result+1]=item end
    return result
end
local function map(value)
    local result={}
    for _, item in ipairs(csv(value)) do
        local id, text=string.match(item,"^([^=]+)=(.+)$")
        if id then result[id]=text end
    end
    return result
end
local function valid_permission(name) return type(name)=="string" and #name>0 and #name<=64 and string.match(name,"^[%w_.:%-*]+$")~=nil end
local function allowed(permission) return permission==nil or permission=="" or (now<expires and permissions[permission]==true) end
resource.on_net("__host_admin_result", function(payload,sender)
    if sender~="0" or type(payload)~="table" or payload.seq~=latest then return end
    permissions={}; expires=0
    if payload.ok and type(payload.value)=="table" and type(payload.value.permissions)=="table" then
        for _,permission in ipairs(payload.value.permissions) do permissions[permission]=true end
        expires=now+2
    end
end)
resource.export("filter", function(payload)
    local enabled=resource.settings.get("enabled")
    local include={}; for _,id in ipairs(csv(enabled)) do include[id]=true end
    local order={}; for index,id in ipairs(csv(resource.settings.get("order"))) do order[id]=index end
    local labels=map(resource.settings.get("labels")); local categories=map(resource.settings.get("categories")); local access=map(resource.settings.get("access"))
    local result={}
    local next_checks={}
    for _,entry in ipairs(type(payload)=="table" and payload.entries or {}) do
        if type(entry)=="table" and entry.version==1 and (enabled=="*" or include[entry.id]) then
            if valid_permission(access[entry.id]) then next_checks[access[entry.id]]=true end
            for _,permission in ipairs(entry.permissions or {}) do if valid_permission(permission) then next_checks[permission]=true end end
            local permit=allowed(access[entry.id])
            for _,permission in ipairs(entry.permissions or {}) do permit=permit and allowed(permission) end
            if permit then
                local label,category=labels[entry.id],categories[entry.id]
                if label and #label<=96 and not string.find(label,"%c") then entry.label=label end
                if category and #category<=32 and not string.find(category,"%c") then entry.category=category end
                result[#result+1]=entry
            end
        end
    end
    checks={};for permission in pairs(next_checks) do checks[#checks+1]=permission end;table.sort(checks)
    while #checks>128 do table.remove(checks) end
    table.sort(result,function(a,b)
        local ai,bi=order[a.id] or 1000,order[b.id] or 1000
        if ai~=bi then return ai<bi end
        return a.id<b.id
    end)
    return result
end)
local function policy() sdk.ui.interaction_policy(resource.settings.get("manual_markers")) end
return {
    on_load=policy,
    on_settings=function() policy() end,
    on_ui_update=function(frame)
        now=now+frame.dt; elapsed=elapsed+frame.dt
        if elapsed>=1 then
            elapsed=0; serial=serial+1; latest=tostring(serial)
            resource.send("__host_admin",{seq=latest,action={kind="permissions",check=#checks>0 and checks or nil}})
        end
    end
}
