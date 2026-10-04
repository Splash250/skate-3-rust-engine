-- These positions match placements.json. Accepted server movement drives
-- marker entry; this demonstration does not award competitive score.
local markers = {
    {id="entry", position={-8,1,0}, radius=1.5},
    {id="finish", position={8,1,0}, radius=1.5}
}
local inside = {}
-- Fixed, server-owned pads occupy clear floor left of all authored obstacles.
-- Allocate once per admitted player; stable slots survive later joins/leaves.
local pads, occupied, assigned = {}, {}, {}
for row=0,7 do
    for column=0,7 do
        pads[#pads+1] = {-8 - column*2, 1, (row%2 == 0 and 1 or -1)*(math.floor(row/2)*2+1)}
    end
end
local rail_present = false
resource.command("park_rail", "park.admin", function(args)
    if args[1] == "remove" then
        if rail_present then resource.world.command({op="rail_remove",key="practice",instance="0"}); rail_present=false end
    elseif args[1] == "add" or args[1] == "move" then
        local x = args[1] == "move" and -6 or -5
        resource.world.command({op="rail_upsert",key="practice",instance="0",points={{x,0.75,-4},{x,0.75,4}}})
        rail_present=true
    else sdk.log("park_rail add|move|remove: updates native grind metadata for a practice rail") end
end)
return {
    on_fixed_update=function()
        local live={}
        for _,player in ipairs(resource.players()) do
            live[player.id]=true
            -- Private-room admission belongs to the resource that selected it.
            -- This public park allocates only public-room spawn pads.
            if player.instance == "0" and not assigned[player.id] then
                for slot,position in ipairs(pads) do
                    if not occupied[slot] then
                        assigned[player.id]=slot;occupied[slot]=player.id
                        resource.teleport(player.id,{position=position,heading=0,velocity={0,0,0},instance=0})
                        break
                    end
                end
            end
            local p=player.position
            if p and player.instance == "0" then
                local match=nil
                for _,marker in ipairs(markers) do
                    local q=marker.position
                    if (p[1]-q[1])^2+(p[3]-q[3])^2<marker.radius^2 and math.abs(p[2]-q[2])<2 then match=marker.id end
                end
                if match and inside[player.id]~=match then
                    resource.state.set("marker",{name=match},{kind="player",id=player.id})
                end
                inside[player.id]=match
            end
        end
        for id,slot in pairs(assigned) do
            if not live[id] then assigned[id]=nil;occupied[slot]=nil;inside[id]=nil end
        end
        for id in pairs(inside) do if not live[id] then inside[id]=nil end end
    end
}
