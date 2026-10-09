-- This layer belongs to this client resource instance only. Press K to toggle it.
local previous, shown = false, false
return {
    on_update = function()
        local down = sdk.input.down("KeyK")
        if down and not previous then
            shown = not shown
            if shown then
                local p = sdk.player.read().position
                if p then sdk.map.set({layers={{key="private",items={
                    {kind="marker",key="saved",position={p[1]+10,p[2],p[3]},label="My spot",style={color={0.8,0.5,1,1},size=8}}
                }}}}) end
            else sdk.map.clear() end
        end
        previous = down
    end
}
