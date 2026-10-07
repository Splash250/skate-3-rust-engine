# Boardwalk Borough

An original, compact skate-town world for the roleplay showcase. All scene
geometry uses the repository's redistributable primitive park format.

`placements.json` is the source of truth. Regenerate the required world and
render-only distant LOD with:

```sh
python3 tools/resource_park.py export resources/boardwalk-borough/placements.json \
  --root resources/boardwalk-borough --resource-id boardwalk-borough \
  --lod-scene resources/boardwalk-borough/placements-low.json
```

The stable marker contract is `plaza_spawn`, `pizza_counter`, `drop_01`,
`drop_02`, `drop_03`, `apt_entry`, and `apt_exit`. The apartment's private
interior geometry is authored around world position `[180, 0, 0]`; the property
resource selects a private instance and fixed destinations there. Entry marker
state is server-observed and player-scoped, but client-originated movement
observations are not cheat-proof and must not be treated as competitive proof.
