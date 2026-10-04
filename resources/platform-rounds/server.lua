local rounds,clock={},0
local serial=resource.storage.get('serial') or 0
local function valid(instance)return type(instance)=='string' and instance:match('^%d+$') and #instance<=10 and tostring(tonumber(instance))==instance and tonumber(instance)<=4294967295 end
local function publish(id) resource.state.set('round',rounds[id],{kind='instance',id=id}) end
local function begin(p)
    if type(p)~='table' or not valid(p.instance) then return {ok=false,error='invalid instance'} end
    local n=0;for _ in pairs(rounds) do n=n+1 end;if not rounds[p.instance] and n>=64 then return {ok=false,error='session capacity'} end
    if rounds[p.instance] and rounds[p.instance].phase=='active' then return {ok=false,error='round already active'} end
    serial=serial+1;resource.storage.set('serial',serial)
    rounds[p.instance]={id=tostring(serial),phase='active',mode=resource.settings.get('mode'),duration=resource.settings.get('duration_seconds'),elapsed=0,participants={}}
    publish(p.instance);return {ok=true,id=tostring(serial)}
end
resource.export('begin',begin)
resource.export('inspect',function(instance)return rounds[instance] end)
resource.export('finish',function(instance)local r=rounds[instance];if r then r.phase='ended';publish(instance) end;return r end)
resource.command('round_start','rounds.manage',function(args)begin({instance=args[1] or '0'})end)
resource.on_net('join',function(_,sender)
    for _,p in ipairs(resource.players()) do if p.id==sender and rounds[p.instance] then rounds[p.instance].participants[sender]=true;publish(p.instance) end end
end)
local published=0
return {on_load=function()begin({instance='0'})end,on_update=function(p)
    clock=clock+p.dt;local live={};for _,player in ipairs(resource.players()) do live[player.id]=player.instance end
    for id,r in pairs(rounds) do
        if r.phase=='active' then r.elapsed=math.min(r.duration,r.elapsed+p.dt);if r.elapsed>=r.duration then r.phase='ended' end end
        for actor in pairs(r.participants) do if live[actor]~=id then r.participants[actor]=nil end end
        for actor,instance in pairs(live) do if instance==id then r.participants[actor]=true end end
    end
    if clock-published>=1 then published=clock;for id in pairs(rounds) do publish(id) end end
end}
