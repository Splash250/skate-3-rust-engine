return {
    on_ui_update=function()
        local id=sdk.net.info().local_id
        local marker=id and id~="0" and resource.state.get("marker",{kind="player",id=id})
        sdk.ui.text("park-marker",marker and ("Community park — "..marker.name.." marker reached") or "Community park — ramp, native rail and markers")
    end,
    on_unload=function() sdk.ui.remove("park-marker") end
}
