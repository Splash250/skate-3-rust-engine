-- Signaling belongs to this resource. The existing host remains the voice authority.
local calls, occupied, presence, dial_after, ring_after, directory_after = {}, {}, {}, {}, {}, {}
local serial = 0
local retiring = {}
local function now() return sdk.time.elapsed end
local function players()
    local result = {}
    for _, player in ipairs(resource.players()) do result[player.id] = player end
    return result
end
local function send(actor, name, value)
    resource.send(name, value, actor, {kind="player", id=actor})
end
local function failure(actor, code, message) send(actor, "error", {code=code, message=message}) end
local function enabled(actor)
    local value = presence[actor]
    return value and value.enabled and now()-value.at < 15
end
local function describe(call, actor, reason)
    return {id=call.id, state=call.state, incoming=actor==call.to,
        peer=actor==call.from and call.to or call.from, reason=reason,
        channel=call.state=="active" and (resource.id.."/"..call.id) or nil}
end
local function publish(call, reason)
    local live=players()
    for _, actor in ipairs({call.from,call.to}) do
        if live[actor] then send(actor,"call",describe(call,actor,reason)) end
    end
end
local function finish(call, reason)
    if calls[call.id] ~= call then return end
    -- Removing a connecting channel is ordered after its queued creation.
    if call.state=="connecting" or call.state=="active" then
        retiring[call.id]=now()
        resource.voice.submit({kind="remove_channel",name=call.id})
    end
    call.state="ended"
    calls[call.id]=nil; occupied[call.from]=nil; occupied[call.to]=nil
    publish(call,reason)
end
local function valid(call, live)
    return live[call.from] and live[call.to]
        and live[call.from].instance==call.instance and live[call.to].instance==call.instance
        and enabled(call.from) and enabled(call.to)
end
resource.on("voice_result", function(result, sender)
    if sender~="0" or type(result)~="table" then return end
    if result.operation=="remove_channel" then
        if result.ok==true then retiring[result.name]=nil end
        return
    end
    if result.operation~="channel" then return end
    local call=calls[result.name]
    if not call or call.state~="connecting" then return end
    if result.ok~=true then finish(call,"Voice channel unavailable. Try again later.")
    elseif not valid(call,players()) then finish(call,"Player left the call or changed instance.")
    else call.state="active"; publish(call) end
end)
resource.on_net("request", function(value, sender)
    local live=players()
    if sender=="0" or not live[sender] or type(value)~="table" then return end
    local action=value.action
    if action=="presence" then
        if type(value.enabled)~="boolean" then return end
        presence[sender]={enabled=value.enabled,at=now()}
        if not value.enabled and occupied[sender] then finish(calls[occupied[sender]],"Phone or voice became unavailable.") end
        return
    end
    if action=="contacts" then
        if (directory_after[sender] or 0)>now() then return end
        directory_after[sender]=now()+0.5
        local result={}
        for _, player in ipairs(resource.players()) do
            if player.id~=sender and player.instance==live[sender].instance and #result<63 then
                local available=enabled(player.id) and not occupied[player.id]
                result[#result+1]={id=player.id,label="Player "..player.id,available=available==true,
                    reason=not enabled(player.id) and "Phone or voice is unavailable." or (occupied[player.id] and "Busy" or nil)}
            end
        end
        send(sender,"contacts",result); return
    end
    if action=="dial" then
        if occupied[sender] then failure(sender,"busy","Finish your current call first."); return end
        if (dial_after[sender] or 0)>now() then failure(sender,"cooldown","Wait a few seconds before calling again."); return end
        local target=type(value.target)=="string" and live[value.target] or nil
        if not target or target.id==sender or target.instance~=live[sender].instance then
            failure(sender,"unavailable","Choose another player in your instance."); return
        end
        if not enabled(sender) or not enabled(target.id) then failure(sender,"unavailable","Both players need a working Phone interface and --voice enabled."); return end
        if occupied[target.id] then failure(sender,"busy","That player is already in a call."); return end
        if (ring_after[target.id] or 0)>now() then failure(sender,"cooldown","That player was just called. Try again shortly."); return end
        local count=0; for _ in pairs(calls) do count=count+1 end
        for _ in pairs(retiring) do count=count+1 end
        if count>=16 or serial>=1000000000 then failure(sender,"capacity","All call lines are busy. Try again later."); return end
        serial=serial+1
        local id="g"..resource.generation.."-"..serial
        local call={id=id,from=sender,to=target.id,instance=live[sender].instance,state="ringing",expires=now()+resource.settings.get("ring_seconds")}
        calls[id]=call; occupied[sender]=id; occupied[target.id]=id
        dial_after[sender]=now()+resource.settings.get("dial_cooldown_seconds")
        ring_after[target.id]=now()+3
        publish(call); return
    end
    if action~="accept" and action~="decline" and action~="cancel" and action~="hangup" then return end
    if type(value.id)~="string" or #value.id>64 then return end
    local call=calls[value.id]
    if not call or occupied[sender]~=call.id then failure(sender,"stale","This call is no longer available."); return end
    if not valid(call,live) then finish(call,"Player left the call or changed instance."); return end
    if call.state=="ringing" and now()>=call.expires then finish(call,"No answer."); return end
    if action=="accept" and sender==call.to and call.state=="ringing" then
        call.state="connecting"; call.expires=now()+5
        resource.voice.submit({kind="channel",name=call.id,members={call.from,call.to}})
        publish(call)
    elseif action=="decline" and sender==call.to and call.state=="ringing" then finish(call,"Call declined.")
    elseif action=="cancel" and sender==call.from and call.state=="ringing" then finish(call,"Call cancelled.")
    elseif action=="hangup" and (call.state=="active" or call.state=="connecting") then finish(call,"Call ended.")
    else failure(sender,"stale","That action is unavailable for this call.") end
end)
-- Read-only server exports. The calling dashboard separately authorizes its real sender.
local function diagnostics()
    local result={ringing=0,connecting=0,active=0,retiring=0,capacity=16,used=0}
    for _,call in pairs(calls) do result[call.state]=result[call.state]+1;result.used=result.used+1 end
    for _ in pairs(retiring) do result.retiring=result.retiring+1;result.used=result.used+1 end
    return result
end
resource.export("diagnostics",diagnostics)
resource.export("invariant_test",function()
    local errors,checked={},0
    local function check(condition,message)
        checked=checked+1
        if not condition and #errors<16 then errors[#errors+1]=message end
    end
    local count=0
    for id,call in pairs(calls) do
        count=count+1
        check(id==call.id,"A call record has an inconsistent index.")
        check(call.from~=call.to,"A call has identical participants.")
        check(occupied[call.from]==id and occupied[call.to]==id,"A call is missing a participant index.")
        check(call.state=="ringing" or call.state=="connecting" or call.state=="active","A call has an invalid live phase.")
        check(retiring[id]==nil,"A live call is also retiring.")
    end
    for actor,id in pairs(occupied) do
        local call=calls[id]
        check(call and (call.from==actor or call.to==actor),"A participant index has no matching live call.")
    end
    for id in pairs(retiring) do
        count=count+1;check(calls[id]==nil,"A retiring channel still has a live call.")
    end
    check(count<=16,"Calls and retiring channels exceed capacity.")
    return {ok=#errors==0,checked=checked,errors=errors}
end)
return {on_update=function()
    local live=players()
    for id, at in pairs(retiring) do
        if now()-at>=1 then
            retiring[id]=now()
            resource.voice.submit({kind="remove_channel",name=id})
        end
    end
    for _, call in pairs(calls) do
        if not valid(call,live) then finish(call,"Player left the call or changed instance.")
        elseif call.state~="active" and now()>=call.expires then finish(call,call.state=="ringing" and "No answer." or "Voice connection timed out.") end
    end
    for _, records in ipairs({presence,dial_after,ring_after,directory_after}) do
        for actor in pairs(records) do if not live[actor] then records[actor]=nil end end
    end
end}
