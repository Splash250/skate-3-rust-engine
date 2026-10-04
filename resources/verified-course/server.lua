-- This resource selects policy; the host derives movement validity, checkpoint
-- crossings, points and elapsed time. No client score/event is accepted.
local courses = {["0"]="practice_public", ["7"]="practice_private"}
local course_instances = {practice_public="0", practice_private="7"}
local rooms = {}
for instance, name in pairs(courses) do
resource.entity({op="spawn", key="pickup_" .. instance, instance=tonumber(instance),
    shape={type="sphere",radius=0.2},body_type="static",
    position={-2,1,-2},color={1,0.8,0.1,1}})
resource.competition.submit({kind="define", course={name=name, instance=instance,
    checkpoints={
        {position={-4,1,0},radius=1.5,points=50},
        {position={0,1,0},radius=1.5,points=50},
        {position={4,1,0},radius=1.5,points=50},
        {position={8,1,0},radius=1.5,points=100}
    },
    pickups={{position={-2,1,-2},radius=1.25,points=25}},
    limits={max_speed=15,max_acceleration=60,max_gap_ms=500,max_airborne_ms=2500}
}})
end
resource.command("course_players", "course.admin", function()
    for _, player in ipairs(resource.players()) do sdk.log(player.id .. " instance=" .. player.instance) end
end)
resource.command("course_start", "course.admin", function(args)
    if not args[1] then sdk.log("course_start PLAYER_ID"); return end
    for _, player in ipairs(resource.players()) do
        if player.id == args[1] then
            local course=courses[player.instance]
            if not course then sdk.log("Choose course instance0 or7"); return end
            resource.competition.submit({kind="start",name=course,player=player.id})
            return
        end
    end
    sdk.log("Choose an admitted player in instance0 or7")
end)
resource.command("course_room", "course.admin", function(args)
    if not args[1] or not courses[args[2]] then sdk.log("course_room PLAYER_ID 0|7"); return end
    for _, player in ipairs(resource.players()) do
        if player.id==args[1] then
            -- Cancel before changing the baseline; a room move never finishes a run.
            resource.competition.submit({kind="cancel",player=player.id})
            resource.teleport(player.id,{position={-8,1,0},heading=0,velocity={0,0,0},instance=tonumber(args[2])})
            return
        end
    end
end)
resource.command("course_cancel", "course.admin", function(args)
    if args[1] then resource.competition.submit({kind="cancel",player=args[1]}) end
end)
resource.on("competition_result", function(result)
    if result.ok == false then
        -- Moving a player without an active attempt intentionally has nothing to cancel.
        if result.operation~="cancel" then sdk.log("Course: " .. tostring(result.error)) end
        return
    end
    if result.kind == "completed" then
        resource.state.set("last_result",{player=result.player,points=result.score,
            elapsed_ms=result.elapsed_ms,verification=result.verified_rules},
            {kind="instance",id=course_instances[result.course]})
        sdk.log("Verified course: " .. result.player .. " earned " .. result.score .. " in " .. result.elapsed_ms .. "ms")
    elseif result.kind == "rejected" then
        sdk.log("Course attempt rejected: " .. result.player .. " (" .. result.reason .. ")")
    end
end)
return {
    on_fixed_update=function()
        local live={}
        for _, player in ipairs(resource.players()) do
            live[player.id]=true
            if rooms[player.id]~=player.instance then
                rooms[player.id]=player.instance
                resource.send("course_room",{instance=player.instance},player.id)
            end
        end
        for player in pairs(rooms) do if not live[player] then rooms[player]=nil end end
    end
}
