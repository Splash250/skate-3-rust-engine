local room=nil
resource.on_net("course_room",function(value,sender)
    if sender=="0" and type(value)=="table" and type(value.instance)=="string" then room=value.instance end
end)
return {
    on_ui_update=function()
        local result=room and resource.state.get("last_result",{kind="instance",id=room})
        local text="Course: checkpoints x=-4,0,4,8 / gold pickup (-2,1,-2). Console: course_start PLAYER_ID"
        if result then text="Verified course: " .. result.points .. " points / " .. result.elapsed_ms .. "ms" end
        sdk.ui.text("verified-course",text)
    end
}
