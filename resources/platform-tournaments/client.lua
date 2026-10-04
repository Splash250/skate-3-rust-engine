local clock,stamp=0,''
local function refresh()
    local items={}
    items[#items+1]={id='enroll',label='Enroll'}
    items[#items+1]={id='leave',label='Withdraw'}
    sdk.ui.menu('platform',{section='Server',title='Tournament',items=items})
end
return {on_load=refresh,on_ui_update=function(p)
    clock=clock+p.dt;if clock>=1 then clock=0;refresh()end
    local t=resource.state.get('tournament');sdk.ui.text('platform',t and ('Tournament · '..t.phase..' · '..#t.entrants..' entrants') or 'Tournament starting')
end,on_event=function(e)
    if e.name~='menu_action' or e.menu~='platform' then return end
    resource.send(e.item,{})
end,on_unload=function()sdk.ui.remove('platform');sdk.ui.remove_menu('platform')end}
