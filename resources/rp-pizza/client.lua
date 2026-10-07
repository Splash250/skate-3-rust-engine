local opened,ready,elapsed=false,false,0
local latest=nil
local function send(value) if opened and ready then sdk.ui.browser.send('pizza',value) end end
resource.on_net('response',function(value,sender) if sender=='0' and type(value)=='table' then latest=value;send({kind='job',value=value}) end end)
local function open()
    if opened then sdk.ui.browser.focus('pizza',true);return end
    sdk.ui.browser.open('pizza',{entry='index.html',files={'index.html','pizza.css','pizza.js'},width=430,height=620,focus=true,
        surface={anchor='bottom_right',scale=1,offset={24,24},fps=15}})
    opened=true;ready=false
end
local function request(action) resource.send('request',{action=action}) end
return {
    on_load=function() sdk.ui.interfaces.register('open',{version=1,label='Pizza Shift',icon='delivery',category='Jobs',destination='phone',phone=true,quick=true}) end,
    on_ui_update=function(frame) elapsed=elapsed+(frame.dt or 0);if opened and ready and elapsed>=1 then elapsed=0;request('snapshot') end end,
    on_event=function(e)
        if e.type=='interface' and e.key=='open' then open()
        elseif e.type=='browser' and e.key=='pizza' then
            local event=e.event
            if event.kind=='ready' then ready=true;send({kind='job',value=latest});request('snapshot')
            elseif event.kind=='closed' then opened=false;ready=false
            elseif event.kind=='message' and type(event.value)=='table' then
                local action=event.value.action
                if action=='close' then sdk.ui.browser.close('pizza');opened=false;ready=false
                elseif action=='go_to_counter' or action=='start' or action=='pickup' or action=='deliver' or action=='cancel' then request(action) end
            end
        end
    end,
    on_unload=function() if opened then sdk.ui.browser.close('pizza') end;opened=false;ready=false end
}
