local clock,invitation=0,nil
resource.on_net('invitation',function(value)invitation=value.name end)
local function refresh()
    local items={}
    items[#items+1]={id='create',label='Create crew'}
    items[#items+1]={id='accept',label='Accept invitation'}
    items[#items+1]={id='leave',label='Leave crew'}
    for _,p in ipairs(resource.players())do if p.id~=sdk.net.info().local_id then items[#items+1]={id='invite_'..p.id,label='Invite '..p.id};items[#items+1]={id='kick_'..p.id,label='Remove '..p.id}end end
    sdk.ui.menu('platform',{section='Server',title='Crew',items=items})
end
return {on_load=refresh,on_ui_update=function(p)
    clock=clock+p.dt;if clock>=1 then clock=0;refresh()end
    local crew=(type(sdk.net.info().local_id)=='string' and sdk.net.info().local_id~='0' and resource.state.get('crew',{kind='player',id=sdk.net.info().local_id}))
    if crew then local n=0;for _ in pairs(crew.members)do n=n+1 end;sdk.ui.text('platform',crew.name..' · '..n..' members')
    elseif invitation then sdk.ui.text('platform','Invitation to '..invitation..' · Server > Crew > Accept invitation')
    else sdk.ui.remove('platform')end
end,on_event=function(e)
    if e.name~='menu_action' or e.menu~='platform' then return end
    if e.item:sub(1,7)=='invite_'then resource.send('crew',{action='invite',actor=e.item:sub(8)})elseif e.item:sub(1,5)=='kick_'then resource.send('crew',{action='kick',actor=e.item:sub(6)})else resource.send('crew',{action=e.item,name='Skate crew'})end
end,on_unload=function()sdk.ui.remove('platform');sdk.ui.remove_menu('platform')end}
