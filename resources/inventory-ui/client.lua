local opened, ready = false, false
local function open()
    if opened then return end
    sdk.ui.browser.open("inventory", {entry="index.html", files={"index.html","inventory.css","inventory.js"}, width=900, height=640, focus=true, surface={anchor="center",scale=1,offset={0,0},fps=20}})
    opened=true
end
resource.on_net("inventory", function(payload, sender)
    if sender == "0" and ready then
        sdk.ui.browser.send("inventory", payload)
        sdk.log(payload.ok and "Inventory browser synchronized" or "Inventory browser received an error")
    end
end)
return {
    on_load=function()
        sdk.ui.interfaces.register("open",{version=1,label="Inventory",icon="bag",category="Player",destination="dashboard",phone=true,quick=true,binding={key="KeyI",hold_ms=0}})
    end,
    on_event=function(payload)
        if payload.type=="interface" and payload.key=="open" then open();return end
        if payload.type ~= "browser" or payload.key ~= "inventory" then return end
        local event=payload.event
        if event.kind == "ready" then
            ready=true; resource.send("inventory_request", {action="inspect"})
        elseif event.kind == "closed" then opened=false; ready=false
        elseif event.kind == "message" and type(event.value) == "table" then
            local value=event.value
            if value.action == "close" then
                sdk.ui.browser.close("inventory"); opened=false; ready=false
            elseif value.action == "inspect" or value.action == "buy" then
                resource.send("inventory_request", {action=value.action, item=value.item})
            end
        end
    end,
    on_unload=function() sdk.ui.browser.close("inventory"); sdk.ui.remove("inventory-help") end
}
