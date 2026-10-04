local saved=resource.storage.get('crews') or {serial=0,groups={}}
local invites,clock,last_publish={},0,0
local function account(actor) return resource.call('platform-profiles','identity',actor) end
local function find(id) for key,crew in pairs(saved.groups) do if crew.members[id] then return key,crew end end end
local function count(t) local n=0;for _ in pairs(t) do n=n+1 end;return n end
local function persist() resource.storage.set('crews',saved) end
local function publish()
    for _,p in ipairs(resource.players()) do
        local id=account(p.id);local key,crew;if id then key,crew=find(id) end
        resource.state.set('crew',crew and {id=key,name=crew.name,leader=crew.leader,members=crew.members} or false,{kind='player',id=p.id})
    end
end
resource.export('crew',function(actor)local id=account(actor);if id then local key,crew=find(id);return crew and {id=key,name=crew.name,leader=crew.leader,members=crew.members} end end)
resource.on_net('crew',function(p,sender)
    local id=account(sender);if not id or type(p)~='table' then return end
    local key,crew=find(id)
    if p.action=='create' and not crew and type(p.name)=='string' and #p.name>0 and #p.name<=24 and p.name:match('^[%w _-]+$') and count(saved.groups)<128 then
        saved.serial=saved.serial+1;saved.groups[tostring(saved.serial)]={name=p.name,leader=id,members={[id]=true}};persist()
    elseif p.action=='invite' and crew and crew.leader==id and count(crew.members)<resource.settings.get('max_members') then
        local target=account(p.actor);if target and target~=id and not find(target) and count(invites)<128 then invites[target]={crew=key,expires=clock+60};resource.send('invitation',{crew=key,name=crew.name},p.actor) end
    elseif p.action=='accept' and not crew then
        local invite=invites[id];local invited=invite and saved.groups[invite.crew]
        if invite and invite.expires>=clock and invited and count(invited.members)<resource.settings.get('max_members') then invited.members[id]=true;invites[id]=nil;persist() end
    elseif p.action=='leave' and crew then
        crew.members[id]=nil
        if next(crew.members)==nil then saved.groups[key]=nil elseif crew.leader==id then local keys={};for member in pairs(crew.members) do keys[#keys+1]=member end;table.sort(keys);crew.leader=keys[1] end
        persist()
    elseif p.action=='kick' and crew and crew.leader==id then
        local target=account(p.actor);if target and target~=id then crew.members[target]=nil;persist() end
    end
    publish()
end)
return {on_update=function(p)
    clock=clock+p.dt
    for id,invite in pairs(invites) do if invite.expires<clock then invites[id]=nil end end
    -- Republish private membership on late join and instance admission; host state coalesces.
    if clock-last_publish>=1 then last_publish=clock;publish()end
end}
