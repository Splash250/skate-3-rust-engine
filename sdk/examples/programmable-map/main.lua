local previous_generation
return {
    on_update = function()
        local generation=sdk.map.status().generation
        if not generation or generation==previous_generation then return end
        local p = sdk.player.read().position
        if not p then return end
        sdk.map.set({layers={{key="private",items={
            {kind="marker",key="saved",position={p[1]+10,p[2],p[3]},label="My spot",style={color={0.8,0.5,1,1},size=8}}
        }}}})
        previous_generation = generation
    end,
    on_unload = function() sdk.map.clear() end
}
