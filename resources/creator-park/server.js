// Exported markers remain the only authored coordinate source. Entry is based
// on server observations and is practice guidance, never competitive proof.
const markers = JSON.parse(sdk.readText('markers.json'));
const inside = new Map(), assigned = new Map();
resource.lifecycle({on_fixed_update() {
    const players = resource.players(), live = new Set(players.map(player => player.id));
    for (const id of assigned.keys()) if (!live.has(id)) { assigned.delete(id); inside.delete(id); }
    const occupied = new Set(assigned.values());
    for (const player of players) {
        if (player.instance !== '0') { inside.delete(player.id); continue; }
        if (!assigned.has(player.id)) {
            for (let slot=0; slot<64; slot++) if (!occupied.has(slot)) {
                assigned.set(player.id, slot); occupied.add(slot);
                resource.teleport(player.id, {position:[-10 - slot%8*2, 1, -8 + Math.floor(slot/8)*2], heading:0, velocity:[0,0,0], instance:0});
                break;
            }
        }
        const p=player.position;
        if (!p) continue;
        const marker=markers.find(m => (p[0]-m.position[0])**2+(p[2]-m.position[2])**2 < m.radius**2 && Math.abs(p[1]-m.position[1])<2);
        if (marker && inside.get(player.id)!==marker.id) {
            resource.state.set('marker', {label:marker.label,type:marker.type,order:marker.order}, {kind:'player',id:player.id});
        }
        inside.set(player.id, marker?.id);
    }
}});
