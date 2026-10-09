-- Pure server Lua: clients render this data using their native live map.
local timer, published = 0, false
return {
    on_update = function(frame)
        timer = timer + frame.dt
        if timer < 2 or published then return end
        local players = resource.players()
        local player = players and players[1]
        if not player or not player.position then return end
        local p = player.position
        local x, y, z = p[1], p[2], p[3]
        resource.map.set({
            settings = {title = "Community map", opacity = 0.92},
            layers = {{key = "session", items = {
                {kind="marker",key="meet",position={x+20,y,z},label="Meet here",style={color={1,0.7,0.2,1},size=8}},
                {kind="path",key="route",points={{x,y,z},{x+20,y,z},{x+20,y,z+30}},style={color={0.2,0.8,1,1},size=3}},
                {kind="region",key="zone",points={{x-15,y,z-15},{x+15,y,z-15},{x+15,y,z+15},{x-15,y,z+15}},style={color={1,0.7,0.2,0.8},size=2}}
            }}}
        })
        published = true
    end
}
