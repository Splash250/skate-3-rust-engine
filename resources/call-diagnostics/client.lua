local opened,ready,serial=false,false,0
local time,last_sent,last_authorized=0,-2,-10
local pending,queued,expired={},nil,true
local function send(value) if ready then sdk.ui.browser.send("diagnostics",value) end end
local function close()
    sdk.ui.browser.close("diagnostics");opened=false;ready=false;pending={};queued=nil;expired=true
end
local function request(action)
    serial=serial+1
    if serial>1000000000 then close();return end
    local seq=tostring(serial);pending[seq]={at=time,kind=action}
    resource.send("request",{seq=seq,action=action});last_sent=time
end
resource.on_net("result",function(value,sender)
    if sender~="0" or type(value)~="table" or type(value.seq)~="string" then return end
    local ticket=pending[value.seq]
    if not ticket then return end
    pending[value.seq]=nil
    if not ready or time-ticket.at>2 then return end
    if value.ok then last_authorized=time;expired=false
    elseif value.denied then last_authorized=-10;expired=true;pending={};queued=nil end
    send({kind="result",operation=ticket.kind,ok=value.ok,value=value.value,error=value.error,denied=value.denied,can_test=value.can_test})
end)
return {
    on_load=function()
        sdk.ui.interfaces.register("open",{version=1,label="Call diagnostics",icon="admin",category="Administration",destination="dashboard",phone=true,quick=true,permissions={"calls.diagnostics"}})
    end,
    on_ui_update=function(frame)
        time=time+frame.dt
        if not ready then return end
        for seq,ticket in pairs(pending) do
            if time-ticket.at>2 then pending[seq]=nil;send({kind="error",message="The diagnostics request timed out."}) end
        end
        if not expired and time-last_authorized>2 then expired=true;send({kind="expired"}) end
        if next(pending)==nil and time-last_sent>=1.1 then
            local action=queued or "inspect";queued=nil;request(action)
        end
    end,
    on_event=function(event)
        if event.type=="interface" and event.key=="open" then
            if not opened then
                sdk.ui.browser.open("diagnostics",{entry="index.html",files={"index.html","diagnostics.css","diagnostics.js"},width=960,height=660,focus=true,surface={anchor="center",scale=1,offset={0,0},fps=10}})
                opened=true;ready=false;last_authorized=-10;expired=true
            else sdk.ui.browser.focus("diagnostics",true) end
        elseif event.type=="browser" and event.key=="diagnostics" then
            local value=event.event
            if value.kind=="ready" then ready=true;last_sent=-2;queued="inspect"
            elseif value.kind=="closed" then opened=false;ready=false;pending={};queued=nil;expired=true
            elseif value.kind=="message" and type(value.value)=="table" then
                if value.value.action=="close" then close()
                elseif value.value.action=="inspect" or value.value.action=="test" then queued=value.value.action end
            end
        end
    end,
    on_unload=close
}
