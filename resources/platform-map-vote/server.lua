local clock,deadline,last_round=0,nil,nil
local votes,cooldowns={},{}
local function options()
    local out={};for id in resource.settings.get('rotation'):gmatch('[^,]+') do if #out<16 and id:match('^[a-z0-9_-]+$') then out[#out+1]=id end end
    return out
end
local function tally()
    local counts={};for _,id in ipairs(options()) do counts[id]=0 end
    for _,vote in pairs(votes) do if counts[vote] then counts[vote]=counts[vote]+1 end end
    return counts
end
local function publish()resource.state.set('vote',{options=options(),counts=tally(),remaining=deadline and math.max(0,deadline-clock),open=deadline~=nil})end
resource.export('options',options)
resource.on_net('vote',function(p,sender)
    if not deadline or type(p)~='table' or type(p.map)~='string' then return end
    local valid=false;for _,player in ipairs(resource.players()) do if player.id==sender then valid=true end end;if not valid then return end
    local identity=resource.call('platform-profiles','identity',sender) or ('guest:'..sender)
    if cooldowns[identity] and cooldowns[identity]>clock then return end
    if not votes[identity] then local n=0;for _ in pairs(votes)do n=n+1 end;if n>=256 then return end end
    for _,id in ipairs(options()) do if id==p.map then votes[identity]=id;cooldowns[identity]=clock+1;publish();return end end
end)
resource.command('map_vote','maps.manage',function()votes={};cooldowns={};deadline=clock+resource.settings.get('vote_seconds');publish()end)
resource.on('world_result',function(p,sender)if sender=='0' then resource.state.set('rotation_result',p);resource.call('platform-rounds','begin',{instance='0'}) end end)
return {on_load=publish,on_update=function(p)
    clock=clock+p.dt;local r=resource.call('platform-rounds','inspect','0')
    if r and r.phase=='ended' and r.id~=last_round and not deadline then last_round=r.id;votes={};cooldowns={};deadline=clock+resource.settings.get('vote_seconds');publish() end
    if deadline and clock>=deadline then
        deadline=nil;local candidates=options();local counts=tally();local winner=candidates[1]
        for _,id in ipairs(candidates) do if not winner or counts[id]>counts[winner] then winner=id end end
        if winner then resource.world.command({op='select',resource=winner}) else resource.call('platform-rounds','begin',{instance='0'}) end
        publish()
    end
end}
