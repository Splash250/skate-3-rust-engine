local title = "Landing club"
local joined, opted_out, points, current_round = false, false, 0, -1
local next_attempt, last_ui = -1, ""

resource.on_net("enrollment", function(payload, sender)
    if sender ~= "0" or type(payload) ~= "table" then return end
    joined = payload.joined == true
    points = tonumber(payload.points) or 0
    if type(payload.round) == "number" then current_round = payload.round end
end)
resource.on_net("point", function(payload, sender)
    if sender ~= "0" or type(payload) ~= "table" then return end
    if type(payload.round) == "number" and payload.round >= current_round then
        current_round, points = payload.round, tonumber(payload.points) or points
    end
end)

local function enroll(action)
    resource.send("enroll", {action = action})
    next_attempt = sdk.time.elapsed + 2
end

local function show()
    local round = resource.state.get("round") or {phase = "loading", target = challenge_rules.target}
    if type(round.number) == "number" and round.number > current_round then
        current_round, points = round.number, 0
    end
    local status = opted_out and "Spectating" or joined and "Enrolled" or "Joining the next round"
    local progress = resource.call("skate-rules", "progress_label", {points = points, target = round.target})
    local detail = round.phase == "finished" and
        (round.winner ~= "" and "Winner: player " .. round.winner or "Round finished") or
        tostring(round.remaining or 0) .. "s remaining · " .. tostring(round.participants or 0) .. " skaters"
    local stamp = status .. progress .. detail .. tostring(round.number)
    if stamp == last_ui then return end
    last_ui = stamp
    sdk.ui.canvas("challenge", {anchor = "top_right", offset = {24, 24}, size = {350, 170}, items = {
        {key = "panel", type = "rect", position = {0, 0}, size = {350, 170}, color = {0.035, 0.06, 0.085, 0.94}},
        {key = "stripe", type = "rect", position = {0, 0}, size = {5, 170}, color = {0.2, 0.9, 0.65, 1}},
        {key = "title", position = {20, 14}, size = {310, 28}, text = title, font_size = 23},
        {key = "status", position = {20, 48}, size = {310, 22}, text = status, font_size = 15, color = {0.5, 0.9, 0.75, 1}},
        {key = "progress", position = {20, 76}, size = {310, 30}, text = progress, font_size = 23},
        {key = "detail", position = {20, 111}, size = {310, 22}, text = detail, font_size = 14},
        {key = "note", position = {20, 141}, size = {310, 18}, text = "Cooperative · server-managed rounds", font_size = 12, color = {0.65, 0.7, 0.75, 1}},
    }})
end

return {
    on_load = function()
        title = sdk.read_text("ui/title.txt"):gsub("%s+$", "")
        sdk.ui.menu("challenge", {section = "Challenges", title = title, items = {
            {id = "join", label = "Join landing challenge"},
            {id = "leave", label = "Leave landing challenge"},
        }})
        show()
    end,
    on_fixed_update = function()
        if not opted_out and not joined and sdk.time.elapsed >= next_attempt then enroll("join") end
    end,
    on_ui_update = show,
    on_event = function(event)
        if event.name ~= "menu_action" or event.menu ~= "challenge" then return end
        if event.item == "join" then opted_out = false; enroll("join") end
        if event.item == "leave" then opted_out = true; joined = false; enroll("leave") end
    end,
    on_unload = function()
        sdk.ui.remove("challenge")
        sdk.ui.remove_menu("challenge")
    end,
}
