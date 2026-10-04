-- The authenticated event sender can choose only their own presentation preset.
local choices, rooms, revisions, previous = {}, {}, {}, {}
resource.on_net("choose", function(value, sender)
    if type(value) ~= "table" or (value.mode ~= "nod" and value.mode ~= "off") then return end
    for _, player in ipairs(resource.players()) do
        if player.id == sender then
            revisions[sender] = (revisions[sender] or 0) + 1
            choices[sender] = {mode=value.mode, skin=value.skin==true, hat=value.hat==true, revision=revisions[sender]}
            return
        end
    end
end)
return {on_update=function()
    local next_rooms, live = {}, {}
    for _, player in ipairs(resource.players()) do
        live[player.id] = true
        local instance = player.instance or "0"
        next_rooms[instance] = next_rooms[instance] or {}
        if choices[player.id] then next_rooms[instance][player.id] = choices[player.id] end
        if rooms[player.id] ~= instance then
            rooms[player.id] = instance
            resource.send("room", {id=instance}, player.id)
        end
    end
    for id in pairs(rooms) do if not live[id] then choices[id]=nil;revisions[id]=nil;rooms[id]=nil end end
    -- Fixed small example: each descriptor is bounded and at most64 admitted
    -- players contribute. Publish on change to avoid needless reliable traffic.
    for instance, values in pairs(next_rooms) do
        local changed=false
        for id, value in pairs(values) do if not previous[instance] or previous[instance][id] ~= value then changed=true end end
        for id in pairs(previous[instance] or {}) do if not values[id] then changed=true end end
        if changed then resource.state.set("actors", values, {kind="instance",id=instance}) end
    end
    for instance in pairs(previous) do if not next_rooms[instance] then resource.state.set("actors",nil,{kind="instance",id=instance}) end end
    previous = next_rooms
end}
