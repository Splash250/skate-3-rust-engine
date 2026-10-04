-- All durable identity comes from the host's admitted player snapshot.
local ready, serial = false, 0
local pending, busy, profiles, seen = {}, {}, {}, {}
local function identity(actor)
    for _, p in ipairs(resource.players()) do
        if p.id == actor and type(p.account_id) == 'string' then return p.account_id end
    end
end
local function text(v) return {type='text',value=v} end
local function sql(q,p) return {sql=q,params=p} end
resource.export('identity', function(actor) return identity(actor) end)
resource.export('profile', function(actor) local id=identity(actor); return id and profiles[id] end)
local function request(actor,name,visit)
    local account=identity(actor)
    if not ready or not account or busy[account] then return end
    local n=0;for _ in pairs(pending) do n=n+1 end;if n>=64 then return end
    serial=serial+1;local key='profile_'..serial
    local statements={sql('INSERT INTO profiles(account,name,visits) VALUES(?1,?2,0) ON CONFLICT(account) DO NOTHING',{text(account),text(resource.settings.get('welcome_name'))})}
    if name then statements[#statements+1]=sql('UPDATE profiles SET name=?2 WHERE account=?1',{text(account),text(name)}) end
    if visit then statements[#statements+1]=sql('UPDATE profiles SET visits=MIN(visits+1,1000000000) WHERE account=?1',{text(account)}) end
    statements[#statements+1]=sql('SELECT name,visits FROM profiles WHERE account=?1',{text(account)})
    pending[key]={actor=actor,account=account};busy[account]=true
    resource.services.submit(key,{kind='transaction',statements=statements},5000)
    return true
end
resource.on_net('rename',function(p,sender)
    if type(p)~='table' or type(p.name)~='string' or #p.name<1 or #p.name>24 or not p.name:match('^[%w _-]+$') then return end
    request(sender,p.name,false)
end)
resource.on('service_result',function(p,sender)
    if sender~='0' then return end
    if p.key=='schema' then ready=p.result.ok;if not ready then sdk.log('Profile schema unavailable') end;return end
    local req=pending[p.key];if not req then return end;pending[p.key]=nil;busy[req.account]=nil
    if not p.result.ok then seen[req.actor]=nil;return end
    local rows=p.result.value.results;local row=rows[#rows].rows[1]
    profiles[req.account]={name=row[1].value,visits=row[2].value}
    if identity(req.actor)==req.account then resource.state.set('profile',profiles[req.account],{kind='player',id=req.actor}) end
end)
return {on_load=function()
    resource.services.submit('schema',{kind='migrate',migrations={{version=1,statements={sql('CREATE TABLE profiles(account TEXT PRIMARY KEY,name TEXT NOT NULL CHECK(length(name)<=24),visits INTEGER NOT NULL CHECK(visits>=0 AND visits<=1000000000))')}}}},5000)
end,on_update=function()
    local live,accounts={},{}
    for _,p in ipairs(resource.players()) do
        live[p.id]=true
        if type(p.account_id)=='string' then accounts[p.account_id]=true;if ready and not seen[p.id] and not busy[p.account_id] then if request(p.id,nil,true) then seen[p.id]=true end end end
    end
    for actor in pairs(seen) do if not live[actor] then seen[actor]=nil end end
    for account in pairs(profiles) do if not accounts[account] then profiles[account]=nil end end
end}
