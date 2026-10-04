local clock,visible=0,true
local function refresh()
    sdk.ui.menu('platform',{section='Server',title='Profile',items={{id='toggle',label=visible and 'Hide profile overlay' or 'Show profile overlay'}}})
end
return {on_load=refresh,on_ui_update=function(p)
    clock=clock+p.dt;if clock>=1 then clock=0;refresh()end
    local value=(type(sdk.net.info().local_id)=='string' and sdk.net.info().local_id~='0' and resource.state.get('profile',{kind='player',id=sdk.net.info().local_id}))
    if visible then sdk.ui.text('platform',value and (value.name..' · Visits '..value.visits) or 'Sign in for persistent profiles and crews')else sdk.ui.remove('platform')end
end,on_event=function(e)
    if e.name=='menu_action' and e.menu=='platform' and e.item=='toggle' then visible=not visible;refresh()end
end,on_unload=function()sdk.ui.remove('platform');sdk.ui.remove_menu('platform')end}
