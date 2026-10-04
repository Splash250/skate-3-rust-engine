-- The server owns the competition. Landings are owner-reported observations,
-- so this is a cooperative example, not an anti-cheat verified leaderboard.
local members, next_enrollment = {}, {}
local round = {number = 0, phase = "waiting", target = challenge_rules.target, participants = 0}
local total = {rounds = 0, landings = 0}
local deadline, last_publish = 0, -1

local function now() return sdk.time.elapsed end
local function find_player(id)
    for _, player in ipairs(resource.players()) do
        if tostring(player.id) == tostring(id) then return player end
    end
end

local function publish()
    local count = 0
    for _ in pairs(members) do count = count + 1 end
    round.participants = count
    round.remaining = round.phase == "running" and math.max(0, math.ceil(deadline - now())) or 0
    resource.state.set("round", round)
    last_publish = math.floor(now())
end

local function start_round()
    local has_member = next(members) ~= nil
    round = {
        number = round.number + 1,
        phase = has_member and "running" or "waiting",
        target = challenge_rules.target,
        leader = "",
        leader_points = 0,
        completed_rounds = total.rounds,
    }
    deadline = now() + challenge_rules.duration_seconds
    for id, member in pairs(members) do
        local player = find_player(id)
        member.cursor = player and player.gameplay and player.gameplay.landed_seq or 0
        member.points, member.last_point = 0, -1000
    end
    publish()
end

local function finish(winner)
    if round.phase ~= "running" then return end
    round.phase, round.winner = "finished", tostring(winner or "")
    resource.emit("completed", {winner = round.winner, number = round.number})
    sdk.time.after("next_round", challenge_rules.next_round_delay, start_round)
    publish()
end

resource.on("completed", function()
    total.rounds = total.rounds + 1
    resource.storage.set("totals", total)
    round.completed_rounds = total.rounds
end)

resource.on_net("enroll", function(payload, sender)
    if sender == "0" or type(payload) ~= "table" then return end
    for key in pairs(payload) do if key ~= "action" then return end end
    if payload.action ~= "join" and payload.action ~= "leave" then return end
    local player = find_player(sender)
    if not player then return end
    local id = tostring(sender)
    if now() < (next_enrollment[id] or -1) then return end
    next_enrollment[id] = now() + 1
    if payload.action == "leave" then
        members[id] = nil
        resource.send("enrollment", {joined = false}, sender)
        publish()
        return
    end
    if not members[id] then
        members[id] = {cursor = player.gameplay and player.gameplay.landed_seq or 0,
            points = 0, last_point = -1000}
        if round.phase == "waiting" then start_round() end
    end
    resource.send("enrollment", {joined = true, points = members[id].points, round = round.number}, sender)
    publish()
end)

resource.command("challenge_reset", "challenge.admin", function()
    sdk.time.cancel("next_round")
    start_round()
end)

return {
    on_load = function()
        local saved = resource.storage.get("totals")
        if type(saved) == "table" then
            total.rounds = math.max(0, math.min(1000000000, tonumber(saved.rounds) or 0))
            total.landings = math.max(0, math.min(1000000000, tonumber(saved.landings) or 0))
        end
        start_round()
        sdk.log("Landing challenge ready; enroll to start a cooperative round")
    end,
    on_update = function()
        local connected = {}
        for _, player in ipairs(resource.players()) do connected[tostring(player.id)] = player end
        for id in pairs(next_enrollment) do if not connected[id] then next_enrollment[id] = nil end end
        for id, member in pairs(members) do
            local player = connected[id]
            if not player then
                members[id] = nil
            elseif round.phase == "running" then
                local gameplay = player.gameplay or {}
                local cursor = tonumber(gameplay.landed_seq) or 0
                if cursor < member.cursor then member.cursor = cursor end
                if cursor > member.cursor then
                    member.cursor = cursor -- Consume even rejected observations; no later replay.
                    if now() - member.last_point >= challenge_rules.minimum_landing_interval then
                        local point = resource.call("skate-rules", "landing_point", {
                            mode = gameplay.mode, landed_trick = gameplay.landed_trick,
                        })
                        if point == 1 then
                            member.points = math.min(challenge_rules.target, member.points + point)
                            member.last_point = now()
                            total.landings = total.landings + 1
                            if member.points > round.leader_points then
                                round.leader, round.leader_points = id, member.points
                            end
                            resource.send("point", {points = member.points, round = round.number}, player.id)
                            if member.points >= challenge_rules.target then finish(id) end
                        end
                    end
                end
            end
        end
        if round.phase == "running" and now() >= deadline then finish(round.leader) end
        if last_publish ~= math.floor(now()) then publish() end
    end,
    on_unload = function()
        resource.storage.set("totals", total)
        sdk.log("Landing challenge stopped")
    end,
}
