local opened, ready, elapsed = false, false, 0
local latest = nil
local function send(value)
    if opened and ready then sdk.ui.browser.send("properties", value) end
end
resource.on_net("response", function(value, sender)
    if sender=="0" and type(value)=="table" then latest=value;send({kind="snapshot",value=latest}) end
end)
local function open()
    if opened then sdk.ui.browser.focus("properties", true); return end
    sdk.ui.browser.open("properties", {entry="index.html", files={"index.html", "properties.css", "properties.js"},
        width=430, height=620, focus=true, surface={anchor="bottom_right", scale=1, offset={24,24}, fps=15}})
    opened=true; ready=false
end
local function request(action, values)
    values=values or {}; values.action=action
    resource.send("request", values)
end
return {
    on_load=function()
        sdk.ui.interfaces.register("open", {version=1, label="Properties", icon="home", category="Player",
            destination="phone", phone=true, quick=true})
    end,
    on_event=function(e)
        if e.type=="interface" and e.key=="open" then open()
        elseif e.type=="browser" and e.key=="properties" then
            local event=e.event
            if event.kind=="ready" then ready=true;send({kind="snapshot",value=latest});request("list")
            elseif event.kind=="closed" then opened=false;ready=false
            elseif event.kind=="message" and type(event.value)=="table" then
                local v=event.value
                if v.action=="close" then sdk.ui.browser.close("properties");opened=false;ready=false
                elseif v.action=="rent" or v.action=="enter" or v.action=="exit" or v.action=="list" then request(v.action,{unit=v.unit,owner=v.owner})
                elseif v.action=="invite" then request("invite",{target=v.target})
                elseif v.action=="accept_invite" then request("accept_invite",{owner=v.owner}) end
            end
        end
    end,
    on_ui_update=function(frame) elapsed=elapsed+(frame.dt or 0);if opened and ready and elapsed>=1 then elapsed=0;request("list") end end,
    on_unload=function() if opened then sdk.ui.browser.close("properties") end;opened=false;ready=false end
}
