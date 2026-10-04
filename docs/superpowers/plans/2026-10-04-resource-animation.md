# Resource animation and presentation implementation

This extends capability 9 of the approved platform request. The existing master
plan authorizes local implementation without another approval or a commit.

## Contract

The shared client command is `animation`, guarded by `engine.animation`. Lua,
JavaScript and C# submit the same version 1 operation. Resource IDs and actor IDs
come from the host; no particular example resource or animation name is special.

An imported bank is bounded JSON with version 1, named bones, expected parent
names, clips, timestamped local-delta translation/rotation/scale keys and marker
events. The parser rejects unknown fields, duplicate names, nonfinite values,
bad quaternion norms, incompatible bone parents, unordered timestamps, excess
duration/key counts and data above the byte budget before installation. This is
an explicitly documented portable authoring format, not a claim of FBX/BVH
support. It is safe to distribute with synthetic, redistributable fixtures.

Operations load/unload a bank, play/stop a named clip on `local` or a canonical
decimal actor ID, and set/clear an appearance or bone attachment from an embedded
package GLB. Every key belongs to its resource generation. Appearance candidates
remain hidden until asset loading and rig binding succeed; failure preserves the
current character. The existing GLB validator rejects external asset reads.

Playback interpolates translations/scales and normalized shortest-path
quaternions, supports rate, loop, offset and fade, and blends cosmetic local-bone
deltas over the existing native pose. Physics, collision, root movement and
server competition results stay with their existing authorities. Marker events
are local presentation notifications. Existing graph eligibility gates remain
the skating extension surface; they cannot fabricate verified trick outcomes.

Replication uses the existing reliable resource state/event lane. An example
server validates the requesting sender and publishes player/instance-scoped
presentation descriptors; clients apply those descriptors to the corresponding
visible actor. Persistent scoped state supplies late joins. Disconnect,
visibility loss, generation replacement, failure and resource stop remove owned
layers, candidates and attachments, restore hidden stock visuals and free assets.

## Integration and scheduling

- Add a renderer-independent parser, operation schema and sampling helpers in
  `crates/skate-mods/src/animation.rs`.
- Add client ownership, binding, asset loading, events and cleanup in
  `crates/skate-game/src/modding/animation.rs`.
- Expose only current local/remote render roots and named bone bindings. Reuse
  `AnimationStatus::for_scene`, package-contained `graphics::asset_path` and the
  existing native hierarchy; do not replace the native animation graph.
- Restore the previous frame's overlay before native animation presentation,
  then apply the new overlay after local and remote base poses. This prevents
  additive drift when a remote has no new network sample.
- Route commands and retirement through the modding owner lifecycle. Deliver
  bounded marker/ready/failure events through `on_event`.

## Verification sequence

1. Parser regression tests first: valid synthetic two-bone bank; missing or
   mismatched parent; duplicate tracks/markers; nonfinite/out-of-range values;
   ordering, size and count limits; quaternion interpolation and looping events.
2. Runtime tests: common command shape and grants in Lua/JS/C#, wrong-side denial,
   reserved owner IDs and unsupported versions, no score mutation from markers.
3. Synthetic Bevy tests: local and remote bindings receive the same layer;
   repeated frames do not accumulate deltas; cross-fades and stop restore base;
   candidate validation failure leaves the previous appearance visible;
   owner stop/actor removal frees all owned presentation.
4. Resource example tests: sender-authorized replication, snapshot replay for
   late joins and cleanup. Build/check all affected consumers after focused
   tests; coordinate Cargo with the other implementation agents.
5. Actual Linux visual verification remains distinct from synthetic ECS tests.
   Native Windows presentation is unverified until run on Windows. Record both
   honestly in the platform evidence ledger.

## Existing limitations observed

`animation_pose/authored_clips.rs` accepts only a private file and seven fixed
stock slots; it cannot serve as the new resource API. The old dedicated
`multiplayer/appearance.rs` path returns early, so its peer appearance exchange
does not establish dedicated resource presentation. `ScoreHolder::credit_trick`
is an accumulator; animation marker claims are not independent verification.
