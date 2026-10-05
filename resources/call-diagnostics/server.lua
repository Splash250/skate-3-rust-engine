local after={}
local function reply(actor,seq,value)
    value.seq=seq
    resource.send("result",value,actor,{kind="player",id=actor})
end
resource.on_net("request",function(value,sender)
    if sender=="0" or type(value)~="table" or type(value.seq)~="string" or #value.seq>32 or not value.seq:match("^[1-9]%d*$") then return end
    if value.action~="inspect" and value.action~="test" then return end
    -- Never accept an identity or permission claim from the payload.
    local allowed=resource.authorized(sender,"calls.diagnostics")
    if not allowed or (value.action=="test" and not resource.authorized(sender,"calls.test")) then
        reply(sender,value.seq,{ok=false,denied=true,error="Your current roles do not permit this operation."});return
    end
    local time=sdk.time.elapsed
    if (after[sender] or 0)>time then reply(sender,value.seq,{ok=false,error="Wait one second before another diagnostics request."});return end
    after[sender]=time+1
    local export=value.action=="inspect" and "diagnostics" or "invariant_test"
    local ok,result=pcall(resource.call,"phone-calls",export,{})
    if not ok then reply(sender,value.seq,{ok=false,error="Call diagnostics are unavailable. Reopen after the resource recovers."});return end
    -- Export work must not let an authorization change expose a stale result.
    local can_read=resource.authorized(sender,"calls.diagnostics")
    local can_test=resource.authorized(sender,"calls.test")
    if not can_read or (value.action=="test" and not can_test) then
        reply(sender,value.seq,{ok=false,denied=true,error="Your permissions changed before this request completed."});return
    end
    reply(sender,value.seq,{ok=true,kind=value.action,value=result,can_test=can_test})
end)
return {on_update=function()
    local live={};for _,player in ipairs(resource.players()) do live[player.id]=true end
    for actor in pairs(after) do if not live[actor] then after[actor]=nil end end
end}
