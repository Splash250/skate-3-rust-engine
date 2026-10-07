// Marker positions are generated from placements.json by resource_park.py.
// They provide guidance from the server's current observations; they are not
// authoritative proof of movement or job completion.
const markers = JSON.parse(sdk.readText('markers.json'));
resource.export('markers', () => markers.map(marker => ({...marker, position:[...marker.position]})));
resource.export('marker', actor => {
    if (typeof actor !== 'string' || !/^[1-9][0-9]*$/.test(actor)) return null;
    const player = resource.players().find(candidate => candidate.id === actor);
    if (!player || player.instance !== '0') return null;
    return resource.state.get('marker', {kind:'player', id:actor}) || null;
});
const inside = new Map();
const assigned = new Map();
const pads = [];
for (let row = 0; row < 8; row++) {
    for (let column = 0; column < 8; column++) {
        pads.push([-18 + column * 2.5, 1, -20 + row * 2.5]);
    }
}

resource.lifecycle({on_fixed_update() {
    const players = resource.players();
    const live = new Set(players.map(player => player.id));
    const occupied = new Set(assigned.values());
    for (const id of assigned.keys()) {
        if (!live.has(id)) { assigned.delete(id); inside.delete(id); }
    }
    for (const player of players) {
        if (player.instance !== '0') {
            if (inside.get(player.id) != null) {
                resource.state.set('marker', false, {kind:'player', id:player.id});
            }
            inside.delete(player.id);
            continue;
        }
        if (!assigned.has(player.id)) {
            const slot = pads.findIndex((_, index) => !occupied.has(index));
            if (slot >= 0) {
                assigned.set(player.id, slot);
                occupied.add(slot);
                resource.teleport(player.id, {position:pads[slot], heading:0, velocity:[0,0,0], instance:0});
            }
        }
        const p = player.position;
        let marker = null;
        if (Array.isArray(p)) {
            marker = markers.find(m => (p[0]-m.position[0])**2 + (p[2]-m.position[2])**2 < m.radius**2 && Math.abs(p[1]-m.position[1]) < 2) || null;
        }
        const previous = inside.get(player.id);
        if (marker && previous !== marker.id) {
            resource.state.set('marker', {id:marker.id,label:marker.label,type:marker.type,order:marker.order}, {kind:'player', id:player.id});
        } else if (!marker && previous != null) {
            resource.state.set('marker', false, {kind:'player', id:player.id});
        }
        inside.set(player.id, marker ? marker.id : null);
    }
}});
