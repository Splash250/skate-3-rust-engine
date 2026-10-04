local clock,phase,serial=0,'enrollment',resource.storage.get('serial') or 0
local entrants,order,current,results={},{},nil,{}
local function account(actor)return resource.call('platform-profiles','identity',actor)end
local function actor(id)for _,p in ipairs(resource.players())do if p.account_id==id then return p end end end
local function publish()resource.state.set('tournament',{id=tostring(serial),phase=phase,entrants=order,current=current and current.account,results=results,rules=resource.settings.get('rules')})end
local function restore()
    if current then resource.teleport(current.actor,{restore_previous=true})end
end
local function cancel()
    if current and current.attempt then resource.call('platform-leaderboards','cancel',current.attempt)end
    restore();current=nil;phase='enrollment';entrants={};order={};results={};publish()
end
resource.export('inspect',function()return {phase=phase,entrants=order,results=results}end)
resource.on_net('enroll',function(_,sender)
    local id=account(sender);if not id or phase~='enrollment' or entrants[id] or #order>=resource.settings.get('max_entrants')then return end
    entrants[id]=true;order[#order+1]=id;publish()
end)
resource.on_net('leave',function(_,sender)
    local id=account(sender);if not id then return end
    if phase=='enrollment' then
        entrants[id]=nil;for i,v in ipairs(order)do if v==id then table.remove(order,i);break end end;publish()
    elseif phase=='running' and entrants[id] and not results[id] then
        results[id]={status='forfeit',reason='withdrawn'}
        if current and current.account==id then
            if current.attempt then resource.call('platform-leaderboards','cancel',current.attempt)end
            restore();current=nil
        end
        publish()
    end
end)
resource.command('tournament_start','tournaments.manage',function()
    if phase~='enrollment' or #order<1 then sdk.log('Enroll verified players before starting');return end
    serial=serial+1;resource.storage.set('serial',serial);phase='running';publish()
end)
resource.command('tournament_cancel','tournaments.manage',cancel)
resource.command('tournament_reset','tournaments.manage',function()if phase=='finished'then cancel()end end)
return {on_load=publish,on_unload=function()if current and current.attempt then resource.call('platform-leaderboards','cancel',current.attempt)end end,on_update=function(p)
    clock=clock+p.dt;if phase~='running'then return end
    if not current then
        local id;for _,v in ipairs(order)do if not results[v]then id=v;break end end
        if not id then phase='finished';publish();return end
        local player=actor(id)
        if not player then results[id]={status='forfeit',reason='account offline'};publish();return end
        -- Choose a vacant static-world instance. Host still enforces all native guards.
        local occupied={};for _,v in ipairs(resource.players())do occupied[v.instance]=true end
        local instance=1000;while occupied[tostring(instance)] and instance<1064 do instance=instance+1 end
        if instance==1064 then results[id]={status='forfeit',reason='no private instance'};publish();return end
        current={account=id,actor=player.id,instance=player.instance,position=player.position or {-8,1,0},room=tostring(instance),deadline=clock+15}
        resource.teleport(player.id,{position={-8,1,0},heading=0,velocity={0,0,0},instance=instance,restore_on_stop=true});publish();return
    end
    local player=actor(current.account)
    if not player then
        -- The host hides players during content readmission. Give the original
        -- connection a bounded opportunity to finish that normal transition.
        current.missing_since=current.missing_since or clock
        if clock-current.missing_since>=15 then
            if current.attempt then resource.call('platform-leaderboards','cancel',current.attempt)end
            results[current.account]={status='forfeit',reason='admission or connection timeout'};restore();current=nil;publish()
        end
        return
    end
    current.missing_since=nil
    if player.id~=current.actor then
        if current.attempt then resource.call('platform-leaderboards','cancel',current.attempt)end
        results[current.account]={status='forfeit',reason='connection changed'};restore();current=nil;publish();return
    end
    if not current.attempt then
        if player.instance==current.room then
            local attempt=resource.call('platform-leaderboards','start',{player=player.id,rules=resource.settings.get('rules'),ticks=resource.settings.get('ticks'),tournament=tostring(serial)})
            if attempt.ok then current.attempt=attempt.id;current.deadline=clock+90
            elseif clock>=current.deadline then results[current.account]={status='rejected',reason=attempt.error};restore();current=nil;publish()end
        elseif clock>=current.deadline then results[current.account]={status='rejected',reason='private admission timeout'};restore();current=nil;publish()end
    else
        local result=resource.call('platform-leaderboards','result',current.attempt)
        if result then results[current.account]=result;restore();current=nil;publish()
        elseif clock>=current.deadline then resource.call('platform-leaderboards','cancel',current.attempt);results[current.account]={status='cancelled',reason='outcome persistence timeout'};restore();current=nil;publish()end
    end
end}
