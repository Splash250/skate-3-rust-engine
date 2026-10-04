local room, applied, reset = nil, {}, false
local function remove(id)
    for _, suffix in ipairs({"move","skin","hat"}) do sdk.animation.submit({op="remove",key=id.."_"..suffix}) end
end
resource.on_net("room",function(value,sender)
    if sender~="0" then return end
    if room~=value.id then reset=true;room=value.id end
end)
return {
    on_load=function()
        sdk.animation.submit({op="load",key="moves",path="clips.json"})
        resource.send("choose",{mode="nod",skin=true,hat=true})
        sdk.ui.text("presentation","Resource presentation: shared mascot, nod and hat; cosmetic markers only")
    end,
    on_update=function()
        if not room then return end
        local work=0
        if reset then
            for id in pairs(applied) do remove(id);applied[id]=nil;work=work+1;if work==8 then return end end
            reset=false
        end
        local values=resource.state.get("actors",{kind="instance",id=room}) or {}
        for id, value in pairs(values) do
            if applied[id]~=value.revision then
                remove(id);applied[id]=value.revision
                if value.mode=="nod" then sdk.animation.submit({op="play",key=id.."_move",bank="moves",clip="nod",target=id,looped=true,fade_in=0.2}) end
                if value.skin then sdk.animation.submit({op="appearance",key=id.."_skin",target=id,path="mascot.glb"}) end
                if value.hat then sdk.animation.submit({op="attach",key=id.."_hat",target=id,bone="HEAD",path="hat.glb",translation={0.24,0,0},rotation={0,0.70710678,0,0.70710678},scale={1,1,1}}) end
                work=work+1;if work==8 then return end
            end
        end
        for id in pairs(applied) do if not values[id] then remove(id);applied[id]=nil;work=work+1;if work==8 then return end end end
    end,
    on_event=function(event)
        if event.type=="animation" and event.event.kind=="error" then sdk.log(event.event.message) end
    end
}
