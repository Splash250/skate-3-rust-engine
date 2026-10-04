-- The server chooses membership; the user still enables --voice and holds V.
resource.on_net("select_channel", function(value)
    if type(value) ~= "table" or type(value.channel) ~= "string" then return end
    if value.channel ~= "" and not value.channel:match("^voice%-room/[%w_.%-]+$") then return end
    resource.voice.submit({kind="transmit", pressed=true, channel=value.channel})
end)
resource.on("voice_result", function(result)
    if result.ok == false then sdk.log("Voice: " .. tostring(result.error)) end
    if result.kind == "error" then sdk.log("Voice device: " .. tostring(result.value.message)) end
end)
