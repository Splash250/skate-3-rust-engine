local clock,stamp=0,''
local function refresh()
    local items={}
    local vote=resource.state.get('vote') or {}
    for _,map in ipairs(vote.options or {})do items[#items+1]={id=map,label=map,description='Vote for this required world',enabled=vote.open==true}end
    if #items==0 then items[1]={id='waiting',label='Waiting for vote options',enabled=false}end
    sdk.ui.menu('platform',{section='Server',title='Map vote',items=items})
end
return {on_load=refresh,on_ui_update=function(p)
    clock=clock+p.dt;if clock>=1 then clock=0;refresh()end
end,on_event=function(e)
    if e.name~='menu_action' or e.menu~='platform' then return end
    resource.send('vote',{map=e.item})
end,on_unload=function()sdk.ui.remove('platform');sdk.ui.remove_menu('platform')end}
