-- Reusable rules, evaluated in the caller's side through a declared dependency.
resource.export("landing_point", function(observation)
    if type(observation) ~= "table" then return 0 end
    local label = observation.landed_trick
    if observation.mode == "Ragdoll" or type(label) ~= "string" then return 0 end
    if #label == 0 or #label > 128 then return 0 end
    -- Count a confirmed landing once. Native score values are deliberately unused.
    return 1
end)

resource.export("progress_label", function(value)
    if type(value) ~= "table" then return "Waiting for the server" end
    return tostring(value.points or 0) .. " / " .. tostring(value.target or 3) .. " landings"
end)
