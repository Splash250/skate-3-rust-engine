local opened,ready,serial,pending,elapsed=false,false,0,{},0
local function request(action,client_key)
    local count=0;for _ in pairs(pending) do count=count+1 end
    if count>=8 then return end
    serial=serial+1;local seq=tostring(serial);pending[seq]={key=client_key,at=elapsed}
    resource.send("__host_admin",{seq=seq,action=action})
end
resource.on_net("__host_admin_result",function(value,sender)
    if sender~="0" or type(value)~="table" or not pending[value.seq] then return end
    local key=pending[value.seq].key;pending[value.seq]=nil
    if ready then sdk.ui.browser.send("admin",{kind="result",key=key,ok=value.ok,value=value.value,error=value.error}) end
end)
return {
    on_ui_update=function(frame)
        elapsed=elapsed+frame.dt
        for seq,ticket in pairs(pending) do
            if elapsed-ticket.at>=5 then
                pending[seq]=nil
                if ready then sdk.ui.browser.send("admin",{kind="result",key=ticket.key,ok=false,error={code="timeout",message="Server response timed out. Retry the operation."}}) end
            end
        end
    end,
    on_load=function() sdk.ui.interfaces.register("open",{version=1,label="Administration",icon="admin",category="Administration",destination="dashboard",quick=true,permissions={"status.read"}}) end,
    on_event=function(event)
        if event.type=="interface" and event.key=="open" and not opened then
            sdk.ui.browser.open("admin",{entry="index.html",files={"index.html","admin.css","admin.js"},width=1120,height=760,focus=true,surface={anchor="center",scale=1,offset={0,0},fps=20}});opened=true
        elseif event.type=="browser" and event.key=="admin" then
            local e=event.event
            if e.kind=="ready" then ready=true;request({kind="permissions"},"permissions");request({kind="status"},"status")
            elseif e.kind=="closed" then opened=false;ready=false;pending={}
            elseif e.kind=="message" and type(e.value)=="table" then
                if e.value.action=="close" then sdk.ui.browser.close("admin");opened=false;ready=false;pending={}
                elseif type(e.value.request)=="table" and type(e.value.key)=="string" and #e.value.key<=32 then request(e.value.request,e.value.key) end
            end
        end
    end
}
