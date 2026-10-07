local ready, clock, sequence, cursor = false, 0, resource.storage.get('order_sequence') or 0, 1
local jobs = resource.storage.get('jobs') or {}
local published, timers, checked, dispatch, dispatch_refresh = {}, {}, {}, {}, {}
local PLAYERS_PER_TICK = 12
local markers, routes = {}, {}
local function valid_actor(actor) return type(actor)=='string' and #actor<=20 and actor:match('^[1-9][0-9]*$')~=nil end
local function identity(actor)
    if not valid_actor(actor) then return nil end
    local account=resource.call('platform-profiles','identity',actor)
    return type(account)=='string' and #account>0 and #account<=160 and account or nil
end
local function player_for(actor)
    for _,player in ipairs(resource.players()) do if player.id==actor then return player end end
end
local function marker_for(actor)
    local marker=resource.call('boardwalk-borough','marker',actor)
    return type(marker)=='table' and marker.id or nil
end
local function live_street_player(actor)
    local player=player_for(actor)
    if not player then return nil,'not_admitted' end
    if player.instance~='0' then return nil,'wrong_instance' end
    if identity(actor)==nil then return nil,'unverified_actor' end
    return player
end
local function view(job,message,error)
    if not job then return {ready=ready,phase='offered',message=message,error=error} end
    return {ready=ready,order_id=job.order_id,phase=job.phase,target=job.target,
        target_label=job.target and (markers[job.target] and markers[job.target].label),reward=job.reward,
        message=message,error=error}
end
local function publish(actor,account,message,error)
    if identity(actor)~=account then return end
    local value=view(jobs[account],message,error)
    resource.state.set('job',value,{kind='player',id=actor})
    resource.send('response',value,actor)
    return value
end
local function persist()
    resource.storage.set('jobs',jobs)
    resource.storage.set('order_sequence',sequence)
end
local function dispatch_member(actor,enabled)
    local result=resource.call('voice-room','dispatch_member',{actor=actor,enabled=enabled})
    if type(result)~='table' or result.ok~=true then return false end
    dispatch[actor]=enabled or nil
    dispatch_refresh[actor]=enabled and (clock+5) or nil
    return true
end
local function is_active(job)
    return job and (job.phase=='offered' or job.phase=='picked_up' or job.phase=='payout_pending')
end
local function finish(actor,account,job,phase,message,error)
    job.phase=phase;job.error=nil;timers[account]=nil;checked[account]=nil
    persist()
    dispatch_member(actor,false)
    return publish(actor,account,message,error)
end
local function submit_payout(actor,job)
    return resource.call('rp-economy','submit',{actor=actor,operation_id=job.payout_id,
        kind='credit',amount=job.reward,reason='Boardwalk pizza delivery'})
end
local function reconcile_payout(actor,account,job)
    if job.phase~='payout_pending' or clock<(checked[account] or 0) then return end
    checked[account]=clock+0.5
    local status=resource.call('rp-economy','operation',{actor=actor,operation_id=job.payout_id})
    if type(status)~='table' then return end
    if status.status=='applied' then finish(actor,account,job,'complete','Delivery paid: $'..tostring(job.reward)..'.')
    elseif status.status=='rejected' then finish(actor,account,job,'failed','The wallet could not accept this payout.',status.error)
    elseif status.status=='unknown' then
        local submitted=submit_payout(actor,job)
        if type(submitted)=='table' and submitted.ok==false and submitted.error~='not_ready' then
            publish(actor,account,'Payout is waiting for the wallet service.',submitted.error)
        end
    end
end
local function reject(actor,account,message,error)
    return publish(actor,account,message,error)
end
local function handle(value,sender)
    if sender=='0' or type(value)~='table' then return end
    local account=identity(sender)
    if not account then return end
    if not ready then return reject(sender,account,'Pizza service is starting. Try again shortly.','not_ready') end
    local action=value.action
    if action=='snapshot' or action=='list' then return publish(sender,account) end
    local player,error=live_street_player(sender)
    if not player then return reject(sender,account,'Return to the public Boardwalk world to use this job.',error) end
    if action=='go_to_counter' then
        local counter=markers.pizza_counter
        local position=type(counter)=='table' and counter.position or nil
        if type(position)~='table' then return reject(sender,account,'Pizza counter location is unavailable.','marker_unavailable') end
        resource.teleport(sender,{position={position[1],position[2]+0.75,position[3]},heading=0,velocity={0,0,0},instance=0})
        return publish(sender,account,'Heading to Slice of Life Pizza. Start your shift, then pick up the order.')
    end
    local job=jobs[account]
    if action=='start' then
        if marker_for(sender)~='pizza_counter' then return reject(sender,account,'Start a shift at Slice of Life Pizza.','wrong_marker') end
        if is_active(job) then return publish(sender,account,'Your current order is still active.') end
        if not dispatch_member(sender,true) then return reject(sender,account,'Dispatch radio is unavailable. Try again shortly.','dispatch_unavailable') end
        sequence=sequence+1;resource.storage.set('order_sequence',sequence)
        local route_index=((sequence-1)%#routes)+1
        job={account=account,order_id='order_'..tostring(sequence),phase='offered',target=routes[route_index],reward=resource.settings.get('delivery_reward')}
        jobs[account]=job;timers[account]=clock;persist()
        return publish(sender,account,'Shift started. Pick up the order at the pizza counter.')
    elseif action=='pickup' then
        if not job or job.phase~='offered' then return reject(sender,account,'There is no order ready for pickup.','wrong_phase') end
        if marker_for(sender)~='pizza_counter' then return reject(sender,account,'Pick up the order at Slice of Life Pizza.','wrong_marker') end
        job.phase='picked_up';job.error=nil;timers[account]=clock;persist()
        return publish(sender,account,'Order collected. Deliver it to '..markers[job.target].label..'.')
    elseif action=='deliver' then
        if not job or job.phase~='picked_up' then return reject(sender,account,'Pick up an order before delivering it.','wrong_phase') end
        if marker_for(sender)~=job.target then return reject(sender,account,'Deliver to '..markers[job.target].label..'.','wrong_marker') end
        if clock-(timers[account] or clock)<resource.settings.get('minimum_delivery_seconds') then
            return reject(sender,account,'Keep moving; the delivery needs more travel time.','too_soon')
        end
        job.phase='payout_pending';job.payout_id='pizza_delivery_'..job.order_id;job.error=nil;persist()
        checked[account]=clock
        dispatch_member(sender,false)
        local submitted=submit_payout(sender,job)
        if type(submitted)=='table' and submitted.ok==false and submitted.error~='not_ready' then
            return publish(sender,account,'Delivery reached. Payout is waiting for the wallet service.',submitted.error)
        end
        return publish(sender,account,'Delivery confirmed. Payment is processing.')
    elseif action=='cancel' then
        if not job or not is_active(job) or job.phase=='payout_pending' then return reject(sender,account,'This order cannot be cancelled now.','wrong_phase') end
        return finish(sender,account,job,'cancelled','Shift cancelled.')
    end
end

resource.on_net('request',handle)
resource.export('snapshot',function(actor)
    local account=identity(actor)
    if not account then return {ok=false,error='unverified_actor'} end
    return view(jobs[account])
end)
return {
    on_load=function()
        local ok,value=pcall(resource.call,'boardwalk-borough','markers',{})
        if not ok or type(value)~='table' then sdk.log('Boardwalk marker export unavailable');return end
        for _,marker in ipairs(value) do if type(marker)=='table' and type(marker.id)=='string' and type(marker.label)=='string' then markers[marker.id]=marker end end
        for _,id in ipairs({'drop_01','drop_02','drop_03'}) do if markers[id] then routes[#routes+1]=id end end
        ready=markers.pizza_counter~=nil and #routes==3
        if not ready then sdk.log('Pizza route markers are incomplete');return end
        for _,job in pairs(jobs) do if job.phase=='picked_up' then timers[job.account]=clock end end
        resource.state.set('ready',true)
    end,
    on_update=function(frame)
        clock=clock+(type(frame)=='table' and type(frame.dt)=='number' and math.max(0,math.min(frame.dt,10)) or 0)
        if not ready then return end
        local players=resource.players();local live={}
        for _,player in ipairs(players) do live[player.id]=true end
        for actor in pairs(dispatch) do if not live[actor] then dispatch[actor]=nil end end
        if #players==0 then return end
        local count=math.min(PLAYERS_PER_TICK,#players)
        for _=1,count do
            if cursor>#players then cursor=1 end
            local player=players[cursor];cursor=cursor+1
            local account=identity(player.id)
            if account then
                local job=jobs[account]
                if not published[player.id] then publish(player.id,account);published[player.id]=true end
                if is_active(job) then
                    if job.phase~='payout_pending' then
                        if not dispatch[player.id] or clock>=(dispatch_refresh[player.id] or 0) then dispatch_member(player.id,true) end
                    end
                    if job.phase=='picked_up' and not timers[account] then timers[account]=clock end
                    if job.phase=='payout_pending' then reconcile_payout(player.id,account,job)
                    elseif timers[account] and clock-timers[account]>=resource.settings.get('shift_timeout_seconds') then finish(player.id,account,job,'cancelled','The shift expired. Start again at the pizza counter.','expired') end
                end
            end
        end
        for actor in pairs(published) do if not live[actor] then published[actor]=nil end end
    end,
    on_unload=function()
        for actor,enabled in pairs(dispatch) do if enabled then dispatch_member(actor,false) end end
    end
}
