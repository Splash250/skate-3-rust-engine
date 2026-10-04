# Resource worlds and park placement

A selected resource can declare one required embedded SKATE package:

```json
{
  "files": ["park.skate"],
  "world": {"map": "park.skate", "required": true, "max_decoded_bytes": 268435456}
}
```

The world file must be a portable public path listed in `files`. Only one world
may be selected across the resolved dependency set. Assets embedded in the SKATE
package and ordinary resource dependencies use the existing verified download
cache and content digest. Existing manifests without a world retain their
canonical serialization and identity.

Activation proceeds through download, hash verification, bounded decode and
validation, construction of collision/native grind data and rendering, atomic
world publication, and script activation. Resource readiness remains false
until these stages finish. A failed build retains the previous world and reports
a resource activation error. A newer offer or disconnect cancels publication of
an obsolete prepared world. Unload restores the prior local map through the same
loader and retires old visuals, collision and native rail references together.

The original dedicated wire-map admission identity stays stable during a
resource map transition. The server-selected content revision and activation
epoch identify the required world. This lets connected players mount a new park
without accidentally switching to peer hosting or leaving the server. The
server parses the same package and installs its triangle terrain in every
shared-object instance. It does not infer terrain from client observations.

The downloaded-world subset supports SKATE01..15 static triangle rendering,
embedded materials/textures, and authored or verified native grind rails. It
rejects native doors, NPC routes and unsupported extensions with an error rather
than silently dropping gameplay. Resource worlds currently require explicit
triangle collision; retail-only collision archives are outside this subset.
The declared decoded budget defaults to 256 MiB and may be set from 1 to 512 MiB.
Compressed blocks are charged before decode. Independent geometry limits bound
vertices, triangles, materials, textures, rails and rail points. Existing content
file/set quotas also apply.

The renderer partitions geometry into spatial leaves of at most 16384 triangles
and applies frustum culling. Required resource worlds additionally stream mesh
residency by distance: CPU source meshes remain resident, derived from the bounded decoded world input,
while distant mesh handles are dropped so Bevy can release their GPU assets.
Returning to a cell creates a fresh mesh handle on the same world-owned entity.
A 20m retention margin avoids repeated uploads at the edge. Uploads are bounded
per frame; one legal leaf may exceed the byte quantum so it cannot starve.
Collision and native rail data remain resident and spatially indexed, avoiding
holes in gameplay when presentation cells unload. Materials/textures stay
resident under the import budgets. This is mesh residency streaming, not disk
paging or texture residency streaming.

A world may provide up to four independently authored render-only SKATE LODs:

```json
"lods": [{"map": "park-low.skate", "distance": 200}]
```

LOD paths must be distinct public files with increasing integer distances from
10 to 10000 metres. The base supplies collision/native rails; LODs must contain
only presentation geometry/materials. All layers debit one decoded world
budget. Distance to each spatial cell selects its layer. Author corresponding
near/far bounds consistently to avoid visible changes at layer boundaries.
No automatic destructive mesh simplification is performed.

Local `world-streaming.json` in the resource cache configures `distance`
(default600m,100..10000), `uploads_per_frame` (default2,1..16), and
`upload_bytes_per_frame` (default4194304,65536..67108864). It is a regular local
file limited to16KiB. Initial gameplay admission also waits until all nearby
required render cells have entered residency.

`tools/resource_park.py` is a focused placement CLI using portable JSON as its
save/load format. It creates and edits boxes, ramps, native grind polylines and
interaction markers, and exports a complete redistributable world resource.
Run `python3 tools/resource_park.py --help` for commands; the example in
`resources/community-park` includes an editable scene and usage instructions.
All coordinates are Y-up metres. Render-only markers remain separate from
collision; native rail metadata remains separate from its visible mesh.

Graphical two-client park traversal and grind acceptance must be reported
separately from decoder, provider, terrain and construction tests.

Server resources granted `resource.world` can register, replace and remove
native grind rails at runtime:

```lua
resource.world.command({op="rail_upsert", key="practice", instance="0",
  points={{0,1,0},{0,1,8}}, closed=false})
resource.world.command({op="rail_remove", key="practice", instance="0"})
```

Each key belongs to its resource generation and instance. Registration state
uses the scoped reliable state channel, so a late join receives the current
definition and an instance switch retires the previous native query references.
An update rebuilds the resource overlay while preserving the immutable base
provider and independent native owner handles. Cached acquisition/grind state
is retired through the native physical lifecycle before replacing its provider.
Stop/restart removes registrations from the previous generation.

Rails need 2–1024 finite, noncoincident adjacent points. Budgets allow 64 rails
and 4096 points per resource, and 16384 points globally. Native rail metadata
provides grind queries; author a visible rail mesh and ordinary collision in the
park or a shared object when those are also needed. The placement exporter does
this for its rail primitive. Supported moving skating surfaces use the shared
box/sphere/capsule entity API; arbitrary runtime terrain triangle editing is not
part of the current surface API.


Local `asset-limits.json` in the resource cache controls downloadable GLB import.
Missing fields retain these defaults; a downloaded resource cannot raise them:

| Field | Default | Hard maximum |
| --- | ---: | ---: |
| `model_file_bytes` | 16 MiB | 128 MiB |
| `model_json_bytes` | 256 KiB | 2 MiB |
| `geometry_bytes` | 16 MiB | 256 MiB |
| `texture_bytes` | 16 MiB | 256 MiB |
| `image_dimension` | 2048 | 8192 |
| `set_decoded_bytes` | 256 MiB | 2 GiB |

`content` accepts the resource cache's bounded content limits, including
`max_file_bytes`, `max_set_bytes` and `max_cache_bytes`. For a model above64MiB,
raise `content.max_file_bytes` as well; the model budget cannot exceed it.
The local policy is at most16KiB and rejects unknown fields and symlinks.
Validation runs before resource readiness and again before graphics, appearance
or attachment loads. The set budget charges decoded geometry, accessor reads,
image/texture expansions and JSON/file storage across every declared GLB and
world layer. This is an allocation estimate, not a promise about total GPU or
process RSS; engine bookkeeping and upload staging consume additional memory.

Supported GLB2 features include embedded geometry, PNG/JPEG materials, standard
node transforms, skins, morph targets and animation data with bounded accessors,
scene instances and channel expansion. GLB geometry remains presentation data;
shared entities or the required world supply physical collision. External/data
URIs, sparse accessors, compressed/unknown extensions, animated PNG and
compressed PNG metadata are rejected with diagnostics. The existing limits of
256 nodes,32 meshes,64 primitives,16 skins and128 animation channels still bound
object-graph complexity. Custom animation playback uses its separate validated
clip/rig API; importing glTF animation data alone does not change skating logic.


## Repeatable graphical verification

Build the game and server first, then run this opt-in tool in a graphical session
with locally owned prepared assets:

```sh
python3 tools/verify_resource_park.py --assets "$SKATE3_ASSET_ROOT" \
  --bin-dir target/debug --output /tmp/park-verification --overview
python3 tools/verify_resource_park.py --assets "$SKATE3_ASSET_ROOT" \
  --bin-dir target/debug --output /tmp/park-lifecycle --overview --scenario lifecycle
python3 tools/verify_resource_park.py --assets "$SKATE3_ASSET_ROOT" \
  --bin-dir target/debug --output /tmp/park-interaction --overview --scenario interaction
```

The output directory must be empty or absent. The tool copies redistributable
examples into it, starts a dedicated loopback server and two native clients,
delays the second join, and pushes a shared object. It checks resource status,
late-join observation, the final required world and native rail, and all three
public shared colliders. A successful exit cannot hide a silently stopped park.
The lifecycle scenario stops/restores presentation, then stops/restores the park
while both clients remain connected. Intermediate PNGs and native input reports
are saved as `clientN.phaseNN.png` and `clientN.phaseNN.input.txt`; each JSON phase
names its captures and records server resource state. The final capture retains
`clientN.png` and `clientN.input.txt`. `--seconds` and `--interval` configure the
bounded generic verifier (default36 seconds and5-second snapshots).

Processes have bounded shutdown and are cleaned up even after failure; evidence
is retained. The tool does not build, extract, download or upload owned assets.
Inspect the images for park materials, remote presentation and cleanup: metadata
checks alone cannot prove visual correctness. `--scenario all` runs initial,
individual stop/restart and combined lifecycle cases as separate sessions.

The interaction scenario uses additional commands only in its copied resources.
Each real player receives an approved approach beside the current crate with a
velocity toward it, from opposite sides. It issues no object impulse command.
Acceptance requires a server solver contact for that exact player and object,
horizontal crate displacement, and native solved contact evidence for that
object on each client. It then moves one player through the example's ordinary
room command to instance7, emptied only in the verification copy. Both clients
must stop seeing each other; the isolated client must have no public replicated
objects or native shared colliders. Returning to instance0 must restore all
three objects and peer visibility. Server contact logs are opt-in through
`SKATE_RESOURCE_DIAGNOSTICS=1`; candidate proximity alone is not counted as a
solver contact. These checks exercise owner-reported player proxies against
server-owned object physics, not server simulation of the full player rig.

## Visual authoring

[Park Studio](visual-authoring.md) adds selection, transforms, snapping, undo/redo, rail and gameplay-marker editing, deterministic export and immediate native playtest over this placement format. [Creator Courtyard](../../resources/creator-park/README.md) is an original redistributable authored example. Exporting is local authoring; operators publish changes through the existing world resource lifecycle.
