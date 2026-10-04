-- The only score ingestion path is a non-network host completion owned by this VM.
local ready,serial=false,resource.storage.get('serial') or 0
local active,done,top,done_order=nil,{},{},{}
local outbox=resource.storage.get('outbox')
local clock,retry,pending=0,0,false
local function text(v)return {type='text',value=v}end
local function number(v)return {type='real',value=v}end
local function sql(q,p)return {sql=q,params=p}end
local function account(actor)return resource.call('platform-profiles','identity',actor)end
-- Separate bounded categories: course participation must not hide native ranks.
local top_query="SELECT account,rules,score FROM (SELECT account,rules,score FROM best WHERE season=?1 AND rules='course-v1' ORDER BY score DESC,account LIMIT 10) UNION ALL SELECT account,rules,score FROM (SELECT account,rules,score FROM best WHERE season=?1 AND rules='native-input-v1' ORDER BY score DESC,account LIMIT 10) ORDER BY rules,score DESC,account"
local function remember(id,value)
    if not done[id] then done_order[#done_order+1]=id end
    done[id]=value
    while #done_order>64 do done[table.remove(done_order,1)]=nil end
    resource.state.set('last_result',value)
end
local function persist_result()
    if not outbox or pending or not ready then return end
    pending=true;retry=clock+5
    resource.services.submit('commit',{kind='transaction',statements={
        sql('INSERT INTO results(id,account,season,rules,score,tournament) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING',{text(outbox.id),text(outbox.account),text(outbox.season),text(outbox.rules),number(outbox.score),text(outbox.tournament)}),
        sql('INSERT INTO best(account,season,rules,score) VALUES(?1,?2,?3,?4) ON CONFLICT(account,season,rules) DO UPDATE SET score=MAX(score,excluded.score)',{text(outbox.account),text(outbox.season),text(outbox.rules),number(outbox.score)}),
        sql(top_query,{text(outbox.season)})
    }},5000)
end
resource.export('start',function(p)
    if not ready or active or outbox then return {ok=false,error='leaderboard busy or recovering'}end
    if type(p)~='table' or (p.rules~='course-v1' and p.rules~='native-input-v1') then return {ok=false,error='unsupported verifier'}end
    local identity=account(p.player);if not identity then return {ok=false,error='verified account required'}end
    local player;for _,v in ipairs(resource.players())do if v.id==p.player then player=v end end
    if not player then return {ok=false,error='not admitted'}end
    if p.rules=='native-input-v1' and (type(p.ticks)~='number' or p.ticks%1~=0 or p.ticks<60 or p.ticks>3600)then return {ok=false,error='ticks outside 60..3600'}end
    serial=serial+1;resource.storage.set('serial',serial)
    local id=resource.settings.get('season')..':'..serial
    active={id=id,player=p.player,account=identity,rules=p.rules,instance=player.instance,tournament=type(p.tournament)=='string' and p.tournament:sub(1,64) or '',started=clock}
    if p.rules=='course-v1' then
        resource.competition.submit({kind='define',course={name='ranked',instance=player.instance,
            checkpoints={{position={-8,1,0},radius=1.5,points=50},{position={-4,1,0},radius=1.5,points=50},{position={0,1,0},radius=1.5,points=75}},
            limits={max_speed=15,max_acceleration=60,max_gap_ms=500,max_airborne_ms=2500}}})
        resource.competition.submit({kind='start',name='ranked',player=p.player})
    else resource.competition.submit({kind='native_start',player=p.player,ticks=p.ticks})end
    return {ok=true,id=id}
end)
resource.export('cancel',function(id)
    if active and active.id==id then
        resource.competition.submit({kind=active.rules=='native-input-v1' and 'native_cancel' or 'cancel',player=active.player})
        remember(active.id,{id=active.id,status='cancelled'});active=nil
    end
    return true
end)
resource.export('result',function(id)return done[id]end)
resource.export('top',function()return top end)
resource.on('competition_result',function(p,sender)
    if sender~='0' or not active then return end
    if p.ok==false and (p.operation=='start' or p.operation=='native_start' or p.operation=='define') then
        remember(active.id,{id=active.id,status='rejected',reason=p.error});active=nil;return
    end
    if p.player~=active.player then return end
    if p.kind=='completed' and p.verified_rules==active.rules and account(active.player)==active.account then
        local score=p.verified_rules=='native-input-v1' and type(p.score)=='table' and p.score.awarded or p.score
        if type(score)~='number' or score~=score or score<0 or score>1000000000 then return end
        outbox={id=active.id,account=active.account,season=resource.settings.get('season'),rules=p.verified_rules,score=score,tournament=active.tournament}
        resource.storage.set('outbox',outbox) -- durable before submitting; replay is idempotent
        active=nil;persist_result()
    elseif p.kind=='cancelled' or p.kind=='rejected' then
        remember(active.id,{id=active.id,status=p.kind,reason=p.reason});active=nil
    end
end)
resource.on('service_result',function(p,sender)
    if sender~='0' then return end
    if p.key=='schema' then ready=p.result.ok;if ready then
        persist_result();resource.services.submit('top',{kind='query',statement=sql(top_query,{text(resource.settings.get('season'))})},5000)
    end;return end
    if p.key=='commit' then
        pending=false
        if not p.result.ok then return end
        local finished=outbox;outbox=nil;resource.storage.set('outbox',nil)
        if finished then remember(finished.id,{id=finished.id,status='committed',account=finished.account,score=finished.score,rules=finished.rules,tournament=finished.tournament})end
    end
    if (p.key=='commit' or p.key=='top') and p.result.ok then
        local results=p.result.value.results;local rows=results[#results].rows;top={}
        for _,r in ipairs(rows)do top[#top+1]={account=r[1].value,rules=r[2].value,score=r[3].value}end
        resource.state.set('top',top)
    end
end)
return {on_load=function()
    resource.services.submit('schema',{kind='migrate',migrations={{version=1,statements={
        sql('CREATE TABLE results(id TEXT PRIMARY KEY,account TEXT NOT NULL,season TEXT NOT NULL,rules TEXT NOT NULL CHECK(rules IN (\'course-v1\',\'native-input-v1\')),score REAL NOT NULL CHECK(score>=0 AND score<=1000000000),tournament TEXT NOT NULL)'),
        sql('CREATE TABLE best(account TEXT NOT NULL,season TEXT NOT NULL,rules TEXT NOT NULL,score REAL NOT NULL CHECK(score>=0 AND score<=1000000000),PRIMARY KEY(account,season,rules))')
    }}}},5000)
end,on_update=function(p)
    clock=clock+p.dt
    if outbox and not pending and clock>=retry then persist_result()end
    if active then
        local instance;for _,v in ipairs(resource.players())do if v.id==active.player then instance=v.instance end end
        if account(active.player)~=active.account or instance~=active.instance or clock-active.started>85 then
            resource.competition.submit({kind=active.rules=='native-input-v1' and 'native_cancel' or 'cancel',player=active.player})
            remember(active.id,{id=active.id,status='cancelled',reason='session retired or deadline'});active=nil
        end
    end
end}
