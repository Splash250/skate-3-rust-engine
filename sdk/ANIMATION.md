# Resource animation and presentation, version 1

Client resources require `engine.animation`. Lua and JavaScript call
`sdk.animation.submit(operation)`; C# calls `Resource.Animation(operation)`.
The common command is `{kind:"animation",version:1,operation:...}`. Server
resources choose policy and replicate descriptors through resource events/state;
they do not issue renderer commands. There are no special resource IDs.

| Operation | Fields and behavior |
| --- | --- |
| `load` | `key,path`: load a packaged relative `.json` bank after rig validation. |
| `unload` | `key`: remove the bank and its owned active layers. |
| `play` | `key,bank,clip,target?`: play one named clip. `target` is `local` or a canonical decimal visible actor ID. Options: `speed` 0.05–4 (default 1), `looped` (false), `weight` 0–1 (1), `offset` seconds (0), `fade_in`/`fade_out` 0–5 seconds (0.15). |
| `stop` | `key,fade_out?`: fade an active layer to its unmodified base pose. |
| `appearance` | `key,target?,path`: prepare a packaged embedded `.glb` skin; reveal it only after loading and compatible rig binding. |
| `attach` | `key,target?,bone,path`: attach a packaged `.glb` to a named bone. Optional `translation`, normalized `rotation` xyzw and `scale` give the bone-local offset. |
| `remove` | `key`: remove the owned layer, appearance or attachment immediately. |

Keys are resource owned. An active appearance target cannot be claimed by a
second owner. Use different keys for simultaneous layers; start the new layer
and stop the previous layer to blend a transition. Transforms are applied in
stable resource/key order over the current native local or remote pose. A
previous frame's deltas are restored before native presentation so a stalled
network sample cannot accumulate drift. Imported skins copy compatible live
bone-local poses; stock physics and animation graphs keep running underneath.

Skin compatibility requires the expected named hierarchy and compatible inverse
bind frames; matching bone names alone does not retarget an arbitrary skeleton.
The renderer's bone-local basis differs from the native frames by a −90° X
rotation. The supplied procedural robot uses original identity bind frames and
joint-weighted geometry; its hat is authored along bone-local Z. Resource skins
use live pose rendering without static bind-pose frustum bounds, on both local
and remote actors. Instance visibility and resource retirement still remove
their owned scenes. Attachments remain visible over a replacement skin even
though its stock ancestor is hidden.

Banks are an explicit portable JSON format, not FBX/BVH/Cfx animation archives:

```json
{"version":1,"bones":[{"name":"HEAD","parent":"NECK1"}],"clips":[
  {"name":"nod","duration":1,"tracks":[{"bone":"HEAD","keys":[
    {"time":0},
    {"time":0.5,"rotation":[0.17364818,0,0,0.98480775]},
    {"time":1}
  ]}],"markers":[{"time":0.5,"name":"middle","payload":{"effect":"spark"}}]}
]}
```

Bone names are case insensitive and must exist in the native rig. `parent` is
the exact expected parent name, or null for a root. Each track references one
declared bone and has strictly increasing timestamps in 0–duration. Keys default
to zero translation, identity rotation and unit scale. Translation components
are bounded to ±2 metres, scale to 0.25–4, and quaternion squared norm must be
within 0.001 of one. Quaternions interpolate along the shortest hemisphere and
are normalized; translations/scales interpolate linearly. Duplicate bone/clip/
track names, unknown fields, invalid parents and invalid numeric values fail
before a bank replaces the current one. Stop active layers before replacing
their bank. Each bank has at most 256 declared bones, 64 clips, and 256 sorted
markers per clip; clip duration is at most 600 seconds and marker payloads 1 KiB.

The host emits lifecycle `on_event` values with `type:"animation"`, `version:1`,
`key` and `event`. Event kinds include `loaded`, `ready`, `error {message}` and
`marker {name,payload,target}`. Markers describe local presentation only. They
never award points or establish a verified landing, trick or competition result.
Existing `sdk.graphs.set_enabled` remains the reversible graph eligibility
extension. It does not bypass native conditions or force an arbitrary state.

The host may set `SKATE_RESOURCE_ANIMATION_LIMITS` to a JSON limits object before
launch. Invalid configuration disables resource animation. Defaults and hard
ceilings are:

| Field | Default | Hard maximum |
| --- | ---: | ---: |
| `max_bytes` per bank | 4MiB | 16MiB |
| `max_resident_bytes` all bank source bytes | 16MiB | 64MiB |
| `max_banks` | 16 | 64 |
| `max_layers` | 64 | 256 |
| `max_attachments` appearances and attachments combined | 64 | 256 |
| `max_keyframes` across resident banks | 65,536 | 262,144 |
| `max_events_per_frame` queued cosmetic notifications | 128 | 1,024 |

Event overflow drops cosmetic notifications; it does not enqueue unbounded work.
Targets can wait up to 10 seconds for their render rig, and appearance candidates
have a 10-second load deadline. A failed appearance leaves the previous skin
visible. Resource retirement removes banks/layers/skins/attachments and restores
stock visibility; observed target disappearance removes its active visuals.
External GLB URLs and sidecar assets are rejected by the existing package loader.

[`presentation-demo`](../resources/presentation-demo/) demonstrates instance
scoped replication, late-join selection replay and sender-authorized presets.
Its original articulated robot and brimmed hat are generated by
`tools/make_presentation_demo.py`. Run
`python3 -m unittest tools.test_presentation_demo -v` to verify limb articulation,
normalized skin weights and the hat's authored basis.
For an actual two-client visual check with locally owned prepared assets, run
`python3 tools/verify_resource_presentation.py --assets /path/to/assets --output /tmp/presentation-proof`.
It copies the public resources, holds a close camera, selects two imported poses
through ordinary resource commands, and stops the presentation resource. Native
reports include owned bank/layer/appearance/attachment counts and pending work;
the helper requires zero remaining animation state after stop. Inspect both
clients' PNGs as well: counts and marker callbacks cannot establish visual quality.
At 64 players with both visuals enabled, configure `max_attachments` to 128 or
higher; the example batches updates to respect callback command budgets.
Late joiners replay the current selection from its beginning; this version does
not promise frame-exact phase synchronization across machines.
