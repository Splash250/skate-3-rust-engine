local channels, dispatch_members, elapsed = {}, {}, 0
local DISPATCH = "pizza_dispatch"
local DISPATCH_LEASE_SECONDS = 15
local function admitted()
    local live = {}
    for _, player in ipairs(resource.players()) do live[player.id] = true end
    return live
end
local function name(value)
    return type(value) == "string" and #value > 0 and #value <= 64
        and value:match("^[%w_.%-]+$")
end
resource.command("voice_players", "voice.admin", function()
    for _, player in ipairs(resource.players()) do sdk.log(player.id .. " instance=" .. player.instance) end
end)
resource.command("voice_radio", "voice.admin", function(args)
    if not name(args[1]) or #args < 3 or #args > 65 then
        sdk.log("voice_radio CHANNEL PLAYER_ID PLAYER_ID [PLAYER_ID...]")
        return
    end
    if args[1] == DISPATCH then sdk.log("pizza_dispatch is reserved for the pizza job"); return end
    local live, members, seen = admitted(), {}, {}
    for i = 2, #args do
        if not live[args[i]] or seen[args[i]] then sdk.log("Choose distinct admitted players"); return end
        members[#members+1], seen[args[i]] = args[i], true
    end
    if not channels[args[1]] then
        local count=0; for _ in pairs(channels) do count=count+1 end
        if count >= 16 then sdk.log("Remove a channel before creating another"); return end
    end
    resource.voice.submit({kind="channel", name=args[1], members=members})
    for _, previous in ipairs(channels[args[1]] or {}) do
        if not seen[previous] and live[previous] then resource.send("select_channel", {channel=""}, previous) end
    end
    channels[args[1]] = members
    for _, player in ipairs(members) do
        resource.send("select_channel", {channel="voice-room/" .. args[1]}, player)
    end
end)
resource.command("voice_clear", "voice.admin", function(args)
    if not name(args[1]) then sdk.log("voice_clear CHANNEL"); return end
    if args[1] == DISPATCH then sdk.log("pizza_dispatch is reserved for the pizza job"); return end
    resource.voice.submit({kind="remove_channel", name=args[1]})
    local live=admitted()
    for _, player in ipairs(channels[args[1]] or {}) do
        if live[player] then resource.send("select_channel", {channel=""}, player) end
    end
    channels[args[1]]=nil
end)
resource.command("voice_mute", "voice.admin", function(args)
    if not admitted()[args[1]] or (args[2] ~= "true" and args[2] ~= "false") then
        sdk.log("voice_mute PLAYER_ID true|false"); return
    end
    resource.voice.submit({kind="mute", player=args[1], muted=args[2]=="true"})
end)
resource.on("voice_result", function(result)
    if result.ok == false then sdk.log("Voice policy: " .. tostring(result.error)) end
end)

local function apply_dispatch(live)
    local members = {}
    for actor,expires in pairs(dispatch_members) do if live[actor] and expires>elapsed then members[#members + 1] = actor end end
    table.sort(members)
    channels[DISPATCH] = members
    if #members == 0 then
        resource.voice.submit({kind="remove_channel", name=DISPATCH})
    else
        resource.voice.submit({kind="channel", name=DISPATCH, members=members})
    end
end

resource.export("dispatch_member", function(request)
    if type(request) ~= "table" or type(request.actor) ~= "string"
        or not request.actor:match("^[1-9][0-9]*$") or type(request.enabled) ~= "boolean" then
        return {ok=false,error="invalid_request"}
    end
    local live = admitted()
    local actor = request.actor
    if not live[actor] then return {ok=false,error="unadmitted_actor"} end
    local already = type(dispatch_members[actor])=="number" and dispatch_members[actor]>elapsed
    if request.enabled and already then
        dispatch_members[actor]=elapsed+DISPATCH_LEASE_SECONDS
        return {ok=true,changed=false}
    elseif not request.enabled and not already then return {ok=true,changed=false} end
    if request.enabled then
        local count=0; for _ in pairs(dispatch_members) do count=count+1 end
        if count >= 64 then return {ok=false,error="dispatch_full"} end
        dispatch_members[actor] = elapsed+DISPATCH_LEASE_SECONDS
        apply_dispatch(live)
        resource.send("select_channel", {channel="voice-room/" .. DISPATCH}, actor)
    else
        dispatch_members[actor] = nil
        apply_dispatch(live)
        resource.send("select_channel", {channel=""}, actor)
    end
    return {ok=true,changed=true}
end)

return {
    on_load=function()
        resource.voice.submit({kind="proximity",meters=resource.settings.get("proximity_meters")})
    end,
    on_settings=function(change)
        if type(change) == "table" and change.key == "proximity_meters" then
            resource.voice.submit({kind="proximity",meters=resource.settings.get("proximity_meters")})
        end
    end,
    on_update=function(frame)
        elapsed=elapsed+(type(frame)=="table" and type(frame.dt)=="number" and math.max(0,math.min(frame.dt,10)) or 0)
        local live = admitted()
        local changed = false
        for actor,expires in pairs(dispatch_members) do
            if not live[actor] or expires<=elapsed then dispatch_members[actor]=nil; changed=true end
        end
        if changed then apply_dispatch(live) end
    end
}
