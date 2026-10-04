local clock,visible=0,true
local function refresh()
    sdk.ui.menu('platform',{section='Server',title='Leaderboards',items={{id='toggle',label=visible and 'Hide standings overlay' or 'Show verified standings'}}})
end
return {on_load=refresh,on_ui_update=function(p)
    clock=clock+p.dt;if clock>=1 then clock=0;refresh()end
    if visible then
        local top=resource.state.get('top') or {};local lines={'VERIFIED STANDINGS'}
        for _,rules in ipairs({'course-v1','native-input-v1'})do
            local entries={}
            for _,r in ipairs(top)do if r.rules==rules and #entries<5 then
                entries[#entries+1]=(#entries+1)..'. '..r.score..' · '..r.account:sub(1,8)
            end end
            if #entries>0 then lines[#lines+1]=rules..': '..table.concat(entries,' | ')end
        end
        if #top==0 then lines[#lines+1]='No verified finishes yet'end
        sdk.ui.text('platform',table.concat(lines,'\n'))
    else sdk.ui.remove('platform')end
end,on_event=function(e)
    if e.name=='menu_action' and e.menu=='platform' and e.item=='toggle' then visible=not visible;refresh()end
end,on_unload=function()sdk.ui.remove('platform');sdk.ui.remove_menu('platform')end}
