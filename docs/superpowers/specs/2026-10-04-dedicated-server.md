# Dedicated multiplayer baseline

Implement the requested community-hosted dedicated server using the existing
Hybrid Authority model in `CONTEXT.md`. No plugins, resource distribution or
client mod downloads are part of this version.

## Behavior

- A headless Rust executable accepts direct IPv4 UDP clients, without Bevy,
  Steam, a GPU or extracted character assets. A `.skate` file supplies a map
  fingerprint; `--test-world` selects the existing procedural test-world identity.
- Dedicated membership uses a distinct handshake. Require the configured map
  and compatible rig/physics definitions; preserve permissive legacy peer lobbies.
  Target 16 players by default, subject to user steering.
- Reuse bounded BODY/POSE datagrams, acknowledged delta baselines, existing
  interpolation and liveness. Preserve native high-speed skating behavior.
- Replicate built-in current trick, landed trick, score presentation, bail,
  movement mode and ragdoll/body state independently of Lua mods.
- The server computes shared player collision responses from fresh conservative
  proxies and validates shove requests by membership, state, distance, facing,
  action sequence and cooldown. Exactly-once effects use bounded retained queues
  and explicit acknowledgements. Stale/disconnected actors cease interacting.
- Dedicated clients suppress the old independently solved player-player response
  and apply accepted effects in fixed simulation. A shove triggers the stock
  attacker animation and the victim's existing wipeout lifecycle.
- Reject blob transfers and arbitrary mod application records in dedicated mode.
  Client mod execution/replication must not bypass this basic-session policy.

## Authority boundary

This version synchronizes accepted owner snapshots and authorizes shared player
interactions. Detailed skating, world collision and skeletal constraints remain
in the owning client's recovered simulation. Conservative server proxies do not
claim retail collider parity or competitive anti-cheat. Trick/score presentation
is replicated; it is not an independently replayed scoring authority.

## Research

FiveM's [OneSync](https://docs.fivem.net/docs/scripting-reference/onesync/) uses
server-visible state, network ownership and relevance-based replication.
[Migration documentation](https://docs.fivem.net/docs/scripting-manual/migrating-from-other-platforms/)
distinguishes server creation from client simulation. Its
[event guidance](https://docs.fivem.net/docs/developers/server-security/) requires
validating client requests, and
[weaponDamageEvent](https://docs.fivem.net/docs/scripting-reference/events/server-events/#weaponDamageEvent)
can be canceled by the server. These inform this design; the wire protocol and
collision policy are specific to this engine.

## Acceptance evidence

Real loopback UDP tests must demonstrate headless admission, multiple clients,
movement/body/pose/gameplay replication and disconnect cleanup. Deterministic
protocol tests must cover loss, duplicates, reordering, spoofed identity, wrong
map/rig, unsupported payloads, stale state, shove rejection and one-time effect
application. Preserve the current networking tests. Check the game target and
exercise available synthetic integration tests; explicitly report hardware and
owned-asset validation that could not be performed.
