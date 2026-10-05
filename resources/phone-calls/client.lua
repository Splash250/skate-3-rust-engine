-- No microphone consent is granted here: native opt-in, physical PTT and mute/deafen always apply.
local current={state="idle"}
local contacts, message, last_presence, last_enabled = {}, "", -5, nil
local ui_time=0
local presenter_at=-10
local controls={muted=false,deafened=false}
local devices={inputs={},outputs={}}
local function voice() return type(sdk.snapshot.voice)=="table" and sdk.snapshot.voice or {enabled=false} end
local function available()
    local value=voice()
    return ui_time-presenter_at<3 and value.enabled==true and not (type(value.device)=="table" and value.device.state=="error")
end
local function select_channel(channel)
    if voice().enabled then resource.voice.submit({kind="transmit",pressed=true,channel=channel or ""}) end
end
resource.on_net("call",function(value,sender)
    if sender~="0" or type(value)~="table" or type(value.id)~="string" then return end
    if value.state~="ringing" and value.state~="connecting" and value.state~="active" and value.state~="ended" then return end
    -- Late terminal messages cannot end a newer call.
    if value.state=="ended" and current.id and value.id~=current.id then return end
    if value.state=="active" and value.channel~=resource.id.."/"..value.id then return end
    current=value; message=value.reason or ""
    if value.state=="active" then select_channel(value.channel)
    elseif value.state=="ended" then select_channel("") end
end)
resource.on_net("contacts",function(value,sender)
    if sender=="0" and type(value)=="table" then contacts=value end
end)
resource.on_net("error",function(value,sender)
    if sender=="0" and type(value)=="table" and type(value.message)=="string" then message=value.message end
end)
resource.on("voice_result",function(value)
    if value.kind=="devices" and type(value.value)=="table" then
        devices={inputs={},outputs={},truncated=false}
        for _,key in ipairs({"inputs","outputs"}) do
            local bytes=0
            for index,name in ipairs(value.value[key] or {}) do
                if index<=8 and type(name)=="string" and bytes+#name<=512 then
                    devices[key][#devices[key]+1]=name;bytes=bytes+#name
                else devices.truncated=true end
            end
        end
    end
    if value.ok==false then message="Voice: "..tostring(value.error)
    elseif value.kind=="error" then message="Audio device unavailable. Check Voice settings." end
end)
resource.export("request",function(value)
    if type(value)~="table" then return {ok=false,error="Invalid call request."} end
    local action=value.action
    if action=="devices" then
        if not voice().enabled then return {ok=false,error="Voice requires local --voice opt-in."} end
        resource.voice.submit({kind="devices"})
    elseif action=="voice" then
        if not voice().enabled then return {ok=false,error="Voice requires local --voice opt-in."} end
        local selected={input_device=controls.input_device,output_device=controls.output_device}
        for _,field in ipairs({"input_device","output_device"}) do
            if value[field]~=nil then
                local valid=value[field]==""
                for _,name in ipairs(devices[field=="input_device" and "inputs" or "outputs"] or {}) do if name==value[field] then valid=true end end
                if not valid then return {ok=false,error="Refresh devices and choose an available device."} end
                selected[field]=value[field]~="" and value[field] or nil
            end
        end
        controls.input_device=selected.input_device;controls.output_device=selected.output_device
        if type(value.muted)=="boolean" then controls.muted=value.muted end
        if type(value.deafened)=="boolean" then controls.deafened=value.deafened end
        resource.voice.submit({kind="configure",muted=controls.muted,deafened=controls.deafened,input_device=controls.input_device,output_device=controls.output_device})
    elseif action=="contacts" then resource.send("request",{action=action})
    elseif action=="dial" then
        if not available() then return {ok=false,error="Enable --voice and check your audio devices before calling."} end
        if type(value.target)~="string" or not value.target:match("^[1-9]%d*$") or #value.target>20 then return {ok=false,error="Select a player."} end
        resource.send("request",{action=action,target=value.target})
    elseif action=="accept" or action=="decline" or action=="cancel" or action=="hangup" then
        if type(value.id)~="string" or value.id~=current.id then return {ok=false,error="This call is no longer available."} end
        resource.send("request",{action=action,id=value.id})
    else return {ok=false,error="Unsupported call action."} end
    message=""; return {ok=true}
end)
resource.export("snapshot",function()
    presenter_at=ui_time
    return {version=1,call=current,contacts=contacts,message=message,voice=voice(),controls=controls,devices=devices}
end)
return {on_ui_update=function(frame)
    ui_time=ui_time+frame.dt
    local enabled=available()
    if enabled~=last_enabled or ui_time-last_presence>=5 then
        resource.send("request",{action="presence",enabled=enabled})
        last_enabled=enabled; last_presence=ui_time
    end
end}
