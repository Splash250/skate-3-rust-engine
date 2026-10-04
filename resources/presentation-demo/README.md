# Shared presentation example

This resource lets each authenticated sender choose their own server-approved
mascot/nod/hat preset. Instance-scoped state replays selections for late joiners;
room changes and departed players remove the corresponding visuals. The sample
starts the preset automatically on each client. It has no score or physics API.

Before the server runs `ensure presentation-demo`, each client must permit this
resource's `engine.animation` capability for that server source. Merge the
following entry into `grants.json` inside the client resource cache, preserving
existing grants:

```json
{
  "udp://127.0.0.1:31030/48031030": {
    "presentation-demo": ["engine.animation"]
  }
}
```

Replace the example source with the exact endpoint/session namespace shown by
your connection's resource inventory. The cache is `SKATE3_RESOURCE_CACHE` when
set, otherwise `settings/resources` under the prepared asset directory's parent.
Reconnect after changing the policy if activation has already failed. See
[persistent cache and grants](../../docs/multiplayer/resources.md#persistent-cache-and-grants)
for source isolation and the full policy. The server grant alone does not grant
client animation control.

`clips.json` is original local-delta animation targeting the named `HEAD` bone
with parent `NECK1`. The two small GLBs are generated original primitive geometry;
no retail vertices, textures, poses or animation samples are included. The
mascot is a complete articulated robot with rounded joint pieces and weighted
connecting segments across the torso, both arms and both legs. Its 25 compatible
named joints have original identity bind frames; every piece receives the live
native pose. The separate hat has a brim, crown and band, authored along its local
Z axis; the attachment places it relative to the live head bone. Other rigs must supply a compatible
bank and skin; incompatible parents fail validation and preserve the old skin.

For this rig, the live head's local +X points upward. The hat attachment translates
0.24m along +X and rotates its authored Z axis by +90° around Y. This was checked
on two native clients with the close-view capture helper below. The horizontal
visor and eyes use this same head basis; the nod clip pitches around local Z.

Use `tools/make_presentation_demo.py` to reproduce the GLBs and
`python3 -m unittest tools.test_presentation_demo -v` for articulation/hat checks. See
[`sdk/ANIMATION.md`](../../sdk/ANIMATION.md) for the versioned contract, budgets and
the distinction between cosmetic markers and verified competition events.

With built game/server binaries and locally owned prepared assets,
`python3 tools/verify_resource_presentation.py --assets /path/to/assets --output /tmp/presentation-proof`
captures three attachment axes, two distinct held imported poses, and resource
retirement. Inspect the PNGs as well as the metadata; callback logs alone do not
prove correct appearance or movement.
