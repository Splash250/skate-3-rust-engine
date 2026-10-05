local opened,ready,elapsed=false,false,0
local function update()
    local players={}
    for _,p in ipairs(resource.players()) do players[#players+1]={name=p.name or "Skater",id=p.id,local_player=p["local"]==true} end
    sdk.ui.browser.send("guide",{players=players})
end
return {
    on_load=function() sdk.ui.interfaces.register("open",{version=1,label="People nearby",icon="map",category="Player",destination="dashboard",phone=true,quick=true}) end,
    on_event=function(e)
        if e.type=="interface" and e.key=="open" and not opened then
            sdk.ui.browser.open("guide",{entry="index.html",files={"index.html","guide.css","guide.js"},width=420,height=600,focus=true,surface={anchor="bottom_right",scale=1,offset={24,24},fps=15}});opened=true
        elseif e.type=="browser" and e.key=="guide" then
            if e.event.kind=="ready" then ready=true;update()
            elseif e.event.kind=="closed" then opened=false;ready=false
            elseif e.event.kind=="message" and e.event.value.action=="close" then sdk.ui.browser.close("guide");opened=false;ready=false end
        end
    end,
    on_ui_update=function(frame) elapsed=elapsed+frame.dt;if ready and elapsed>=1 then elapsed=0;update() end end
}
