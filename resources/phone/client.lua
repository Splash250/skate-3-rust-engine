local opened, ready, camera, route = false, false, false, "home"
local preferences={theme="twilight",scale=1}
local elapsed, refresh, ringing, last_call = 0, -1, false, ""
local entries={}
local pending_binding=nil
local function send(value) if opened and ready then sdk.ui.browser.send("phone",value) end end
local filter_failed=false
local function filter(values)
    local ok,result=pcall(resource.call,"interaction-policy","filter",{entries=values})
    if ok and type(result)=="table" then filter_failed=false;return result end
    if not filter_failed then send({kind="error",message="Interface policy is unavailable or exceeds its limits. Ask an administrator to review the installed interfaces."}) end
    filter_failed=true;return {}
end
local function snapshot() return resource.call("phone-calls","snapshot",{}) end
local function stop_camera() if camera then camera=false; sdk.photos.mode(false) end end
local function open()
    if opened then sdk.ui.browser.focus("phone",true); return end
    sdk.ui.browser.open("phone",{entry="index.html",files={"index.html","phone.css","phone.js","phone-sans.ttf"},width=390,height=760,focus=true,
        surface={anchor="bottom_right",scale=preferences.scale,offset={24,24},fps=20}})
    opened=true; ready=false; refresh=-1
end
local function close()
    stop_camera(); sdk.ui.browser.close("phone"); opened=false; ready=false; route="home"
end
local function update(dt)
    elapsed=elapsed+(type(dt)=="number" and math.max(0,math.min(dt,10)) or 0.016)
    if pending_binding then
        local result=sdk.commands.result("binding")
        if result then
            pending_binding=nil;send({kind="binding_result",ok=result.ok,error=result.error});sdk.ui.interfaces.list()
        elseif elapsed-pending_binding>4 then pending_binding=nil;send({kind="binding_result",ok=false,error="Binding save timed out. Reopen Settings to inspect it."}) end
    end
    if elapsed-refresh<0.35 then return end
    refresh=elapsed
    local state=snapshot()
    local active=state.call and state.call.state=="ringing" and state.call.incoming==true
    if active and not ringing then
        sdk.audio.play("ring",{path="ring.wav",spatial=false,loop=true,volume=0.15})
    elseif not active and ringing then sdk.audio.stop("ring") end
    ringing=active
    if active and not opened then sdk.ui.text("call","Incoming call · Open Phone to answer")
    elseif last_call~="" then sdk.ui.remove("call") end
    last_call=active and "ringing" or ""
    if opened and ready then
        send({kind="calls",value=state})
        sdk.ui.interfaces.list()
    end
end
resource.export("open",function() open(); return {ok=true} end)
resource.on("photo",function(value)
    if type(value)~="table" then return end
    if value.kind=="mode" then
        camera=value.enabled==true
        if not camera and opened then route="gallery"; sdk.ui.browser.focus("phone",true); sdk.photos.gallery() end
    end
    send({kind="photo",value=value})
end)
return {
    on_load=function()
        local saved=resource.storage.get("preferences")
        if type(saved)=="table" then
            if saved.theme=="twilight" or saved.theme=="paper" then preferences.theme=saved.theme end
            if saved.scale==0.75 or saved.scale==0.85 or saved.scale==1 then preferences.scale=saved.scale end
        end
        sdk.ui.interfaces.register("open",{version=1,label="Phone",icon="phone",category="Player",destination="phone",phone=false,quick=true,
            binding={key="KeyP"}})
        sdk.ui.interfaces.register("voice",{version=1,label="Voice",icon="voice",category="Player",destination="phone",phone=true,quick=true})
        sdk.audio.preload("ring.wav")
    end,
    on_ui_update=function(value) update(value.dt) end,
    on_event=function(payload)
        if payload.type=="interface" and payload.key=="open" then open(); return end
        if payload.type=="interface" and payload.key=="voice" then route="voice";open();send({kind="route",route="voice"});return end
        if payload.type=="interfaces" then
            entries=filter(payload.entries)
            send({kind="apps",entries=entries});return
        end
        if payload.type~="browser" or payload.key~="phone" then return end
        local event=payload.event
        if event.kind=="ready" then
            ready=true;send({kind="boot",preferences=preferences,route=route,calls=snapshot()});sdk.ui.interfaces.list()
        elseif event.kind=="closed" then opened=false; ready=false; stop_camera(); route="home"
        elseif event.kind=="message" and type(event.value)=="table" then
            local value=event.value
            if value.action=="close" then close()
            elseif value.action=="route" then
                if value.route=="home" or value.route=="contacts" or value.route=="gallery" or value.route=="settings" or value.route=="voice" then
                    stop_camera();route=value.route
                    if route=="contacts" then resource.call("phone-calls","request",{action="contacts"}) end
                    if route=="gallery" then sdk.photos.gallery() end
                    if route=="voice" then resource.call("phone-calls","request",{action="devices"}) end
                elseif value.route=="camera" then
                    route="camera";sdk.photos.mode(true)
                end
            elseif value.action=="call" then
                local result=resource.call("phone-calls","request",{action=value.operation,target=value.target,id=value.id,muted=value.muted,deafened=value.deafened,input_device=value.input_device,output_device=value.output_device})
                if not result.ok then send({kind="error",message=result.error}) end
            elseif value.action=="refresh_contacts" then resource.call("phone-calls","request",{action="contacts"})
            elseif value.action=="thumbnail" and type(value.id)=="string" and #value.id<=64 then sdk.photos.thumbnail(value.id)
            elseif value.action=="preferences" then
                if value.theme=="twilight" or value.theme=="paper" then preferences.theme=value.theme end
                local resize=false
                if value.scale==0.75 or value.scale==0.85 or value.scale==1 then resize=preferences.scale~=value.scale;preferences.scale=value.scale end
                resource.storage.set("preferences",preferences)
                if resize then local previous=route;close();route=previous;open() else send({kind="preferences",value=preferences}) end
            elseif value.action=="binding" and not pending_binding then
                local allowed=filter(entries)
                local known=false;for _,entry in ipairs(allowed) do if entry.id==value.id then known=true end end
                local keys={F2=true,F3=true,F4=true,F7=true,F8=true,KeyI=true,KeyP=true,KeyO=true}
                local pad=value.button
                local hold=value.hold_ms
                if known and keys[value.key] and (pad==nil or pad==32 or pad==64 or pad==128 or pad==32768)
                    and type(hold)=="number" and hold>=0 and hold<=2000 and hold%1==0 then
                    sdk.commands.request("binding",{kind="ui_interfaces",operation={kind="bind",id=value.id,binding={key=value.key,button=pad,hold_ms=hold}}})
                    pending_binding=elapsed
                else send({kind="binding_result",ok=false,error="Choose a registered action and supported binding."}) end
            elseif value.action=="invoke" then
                -- Resolve only a currently authorized descriptor; never execute page commands.
                local allowed=filter(entries)
                for _, entry in ipairs(allowed) do
                    if entry.id==value.id and entry.generation==value.generation and entry.phone and (entry.disabled_reason==nil or entry.disabled_reason=="") then
                        close();sdk.ui.interfaces.invoke(entry.id,entry.generation);break
                    end
                end
            end
        end
    end,
    on_unload=function() close();sdk.audio.stop("ring");sdk.ui.remove("call") end
}
