local opened,ready,elapsed=false,false,0
local invocation,notice=nil,nil
local function open()
    if opened then return end
    sdk.ui.browser.open("menu",{entry="index.html",files={"index.html","menu.css","menu.js"},width=520,height=680,focus=true,surface={anchor="center",scale=1,offset={0,0},fps=20}})
    opened=true
end
return {
    on_load=function()
        sdk.ui.interfaces.register("open",{version=1,label="Interaction menu",icon="menu",category="Player",destination="dashboard",binding={key="F2",button=32,hold_ms=600}})
    end,
    on_ui_update=function(frame)
        elapsed=elapsed+frame.dt
        if invocation then
            local result=sdk.commands.result("invoke")
            if result or elapsed-invocation>4 then
                invocation=nil
                if not result or not result.ok then notice=result and result.error or "Interface did not respond. Choose another interface or close this menu.";open() end
            end
        end
        if ready and elapsed>=0.5 then elapsed=0;sdk.ui.interfaces.list() end
    end,
    on_event=function(event)
        if event.type=="interface" and event.key=="open" then open()
        elseif event.type=="interfaces" and ready then
            local ok,entries=pcall(resource.call,"interaction-policy","filter",{entries=event.entries})
            if not ok then entries={};notice="Menu policy could not be applied. Ask the operator to check interface metadata and policy limits." end
            local visible={}
            for _,entry in ipairs(entries) do if entry.id~="master-menu/open" then visible[#visible+1]=entry end end
            sdk.ui.browser.send("menu",{kind="entries",entries=visible})
            if notice then sdk.ui.browser.send("menu",{kind="error",message=notice});notice=nil end
        elseif event.type=="browser" and event.key=="menu" then
            local e=event.event
            if e.kind=="ready" then ready=true;sdk.ui.interfaces.list()
            elseif e.kind=="closed" then opened=false;ready=false
            elseif e.kind=="message" and type(e.value)=="table" then
                local v=e.value
                if v.action=="close" then sdk.ui.browser.close("menu");opened=false;ready=false
                elseif v.action=="invoke" and type(v.id)=="string" and type(v.generation)=="string" then
                    sdk.ui.browser.close("menu");opened=false;ready=false
                    invocation=elapsed
                    sdk.commands.request("invoke",{kind="ui_interfaces",operation={kind="invoke",id=v.id,generation=v.generation}})
                end
            end
        end
    end
}
