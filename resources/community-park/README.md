# Community Practice Park

This original 9 KiB park contains a ramp, an 8 m grind rail, a floor and two
interaction markers. No retail assets are included. The rail has native grind
metadata as well as visible/collidable geometry. Markers are visible but do not
create solid barriers; their positions and radii are in `markers.json`.
Server-observed marker entry updates a player-scoped HUD message; clients never
submit a marker result or competitive score. Grant `resource.state`,
`resource.commands`, `resource.world`, `resource.teleport` and `engine.ui` when selecting this example.
The console command `park_rail add|move|remove` demonstrates runtime native rail
metadata registration (a separate practice rail, without a visible model).

Select this resource in the server's `ensure` list. Exactly one resource in the
resolved set may select a required world. Clients download and validate the
package, build collision and native rails, publish the scene, then report
resource readiness. Resource unload restores the client's prior local world.

The source of truth is `placements.json`. To edit and export it from the
repository root:

```sh
python3 tools/resource_park.py list resources/community-park/placements.json
python3 tools/resource_park.py update resources/community-park/placements.json --id ramp --position 0 1 6
python3 tools/resource_park.py place resources/community-park/placements.json --id bench --kind box --position -4 0.5 3 --size 3 1 1
python3 tools/resource_park.py place resources/community-park/placements.json --id short_rail --kind rail --position -5 1 0 --point 0 0 -2 --point 0 0 2
python3 tools/resource_park.py remove resources/community-park/placements.json --id bench
python3 tools/resource_park.py export resources/community-park/placements.json --root resources/community-park --resource-id community-park --lod-scene resources/community-park/placements-low.json --lod-distance 200
```

The independently authored far LOD simplifies the distant park to its floor
and ramp silhouette. It has no collision or native rail data; the base park
remains authoritative at every viewing distance. Local mesh streaming uses
bounded uploads, and admission waits for nearby required cells.

Reload/restart the resource after exporting. Export rewrites `park.skate`,
`markers.json`, `placements.json`, `resource.json`, and, with `--lod-scene`,
`park-low.skate` and `placements-low.json`. The exporter does not preserve custom
script/capability declarations. For this scripted example, copy generated
world/files fields into the existing manifest or restore its client/server script
and capability fields after exporting; keep the two scripts as separate files.

On first admission the server script allocates one of 64 predefined clear-floor
spawn pads, two metres apart, so joins do not stack players on one spot. Slots
remain stable during a connection and are released when the player leaves.
