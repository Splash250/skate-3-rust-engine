-- Redistributable primitive geometry. No downloaded model or retail data.
local cooldown = {}
local destinations = {
    ["0"] = {position = {-5, 1, 0}, heading = 0, velocity = {0, 0, 0}, instance = 0},
    ["7"] = {position = {-5, 1, 0}, heading = 0, velocity = {0, 0, 0}, instance = 7}
}
local function room(instance, color)
    resource.entity({op = "spawn", key = "floor_" .. instance, instance = instance,
        shape = {type = "box", half_extents = {12, 0.5, 12}}, body_type = "static",
        -- The top sits below the community park's floor (0), but above the
        -- native test world's floor (-0.035), avoiding coplanar rendering.
        position = {0, -0.51, 0}, color = {0.16, 0.18, 0.22, 1}})
    resource.entity({op = "spawn", key = "crate_" .. instance, instance = instance,
        shape = {type = "box", half_extents = {0.6, 0.6, 0.6}}, mass = 12,
        position = {2, 0.65, 0}, color = color})
    resource.entity({op = "spawn", key = "portal_" .. instance, instance = instance,
        shape = {type = "sphere", radius = 0.3}, body_type = "static",
        position = {8, 1, 0}, color = {0.25, 0.9, 0.7, 1}})
end

-- Console: command objects_room PLAYER_ID 0|7. Only predefined destinations
-- are accepted; client payload coordinates never become travel targets.
resource.command("objects_room", "objects.admin", function(args)
    local player, destination = args[1], destinations[args[2]]
    if not player or not destination then sdk.log("objects_room PLAYER_ID 0|7"); return end
    for _, observed in ipairs(resource.players()) do
        if observed.id == player then resource.teleport(player, destination); return end
    end
    sdk.log("Player is not admitted")
end)
resource.command("objects_push", "objects.admin", function(args)
    if not destinations[args[1]] then sdk.log("objects_push 0|7"); return end
    resource.entity({op = "impulse", key = "crate_" .. args[1], impulse = {30, 10, 0}})
end)

return {
    on_load = function()
        room(0, {0.9, 0.45, 0.15, 1})
        room(7, {0.35, 0.55, 0.95, 1})
        resource.state.set("room", {label = "Public practice"}, {kind = "instance", id = "0"})
        resource.state.set("room", {label = "Private practice"}, {kind = "instance", id = "7"})
    end,
    on_fixed_update = function(context)
        local live = {}
        for _, player in ipairs(resource.players()) do
            live[player.id] = true
            cooldown[player.id] = math.max(0, (cooldown[player.id] or 0) - context.dt)
            local p = player.position
            if p and destinations[player.instance] and cooldown[player.id] == 0
                and (p[1] - 8)^2 + p[3]^2 < 1.8^2 and p[2] > -1 and p[2] < 3 then
                local target = player.instance == "0" and "7" or "0"
                resource.teleport(player.id, destinations[target])
                cooldown[player.id] = 2
            end
        end
        for id in pairs(cooldown) do if not live[id] then cooldown[id] = nil end end
    end
}
