local channels = {}
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
