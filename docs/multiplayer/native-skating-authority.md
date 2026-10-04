# Native skating authority: input version 1

`native-input-v1` runs the engine's recovered skating graphs, controls, articulated
physics, terrain/rail queries, landing classifier and native scorer in a trusted
headless game companion. It is an opt-in competition mode; regular dedicated
movement and `Gameplay` scores retain their existing owner-reported semantics.
[Course-v1](verified-competitions.md) remains a separate bounded trajectory verifier
with its unchanged swept capsule and terrain-reference policy.

## Operator configuration and resource API

Build matching binaries and supply locally prepared, legally owned native assets:

```sh
cargo build --locked -p skate-game --bin skate3rust
cargo build --locked -p skate-server
```

Add this field to the existing server resource JSON (paths resolve against that
JSON's directory):

```json
{"native_authority":{"executable":"/path/to/skate3rust","assets":"/path/to/prepared/assets","max_workers":1}}
```

The companion is a trusted operator-selected executable, never downloaded resource
code. The server remains usable without it. Its own asset directory is not exposed
through the resource HTTP inventory and must not be redistributed. Development
builds using dynamic Bevy linking need the same local library search path used by
`cargo run`; deployed builds must include their runtime libraries.

A granted server resource uses its existing `resource.competition` capability:

```lua
resource.competition.submit({kind="native_start",player=actor_id,ticks=300})
resource.competition.submit({kind="native_cancel",player=actor_id})
resource.on("competition_result",function(result)
  -- The start/cancel command has operation, ok and value/error.
  -- A later verified outcome has kind, player, verified_rules, ticks and score.
  if result.kind=="completed" and result.verified_rules=="native-input-v1" then
    local awarded = result.score.awarded
    -- Apply this resource's own reward/ranking policy here.
  end
end)
```

JavaScript uses the same operation; C# uses `Resource.Competition`. `player` is a
canonical decimal string. No command accepts score, trick descriptor, position,
resource owner or generation from the caller. The host assigns the requesting
resource generation. The requested duration is 1–3600 physical ticks (at 60 Hz).
At most 1–4 companions are configured; a slot remains occupied until its process is
actually reaped, even after cancellation. Course/native attempts exclude each other.

The actor must be admitted into a **solitary instance** in the verified required
resource world. Shared entities and resource-added rails make that instance
ineligible, including when added during the attempt. Static rails authored in the
immutable world are available to native physics. Version 1 does not simulate other
players, moving shared objects, arbitrary mod forces or resource-defined physical
behavior. Admission uses the world's trusted spawn/heading, zero initial velocity,
a fresh movement epoch, stock 60 Hz stepping and normal difficulty.

## Trusted inputs and results

The server trusts its executable, owned assets, required world's published bytes,
resource generation, movement epoch and monotonic clock. From the client it accepts
only 18 conditioned controller action channels: four finite stick axes in [-1,1],
two triggers in [0,1], and digital 0/1 buttons. A tick cannot choose a timestep or
submit poses, velocities, contacts, graph state, trick names or scores.

Packets carry 1–32 contiguous samples; exact resends are idempotent. Compact
framing uses a 12-byte header and 26 bytes per sample: six exact f32 analog values
and twelve digital bits, at most 844 bytes within the existing application budget.
The window tolerates normal acknowledgement delay without serializing four ticks
per roundtrip. Accepted inputs drain through a separate four-in-flight worker
limit; temporary pipe backpressure retains the bounded journal. Changed history,
gaps, invalid domains, ticks beyond the attempt and advancing beyond 60 ticks per
host second plus a 3-tick initial allowance are rejected. Stale epoch samples cannot
advance a new attempt. Journals retain at most 3600 samples, and their BLAKE3 digest
binds version, epoch, instance and resource generation. Input packets, pipe frames,
queues and worker counts are bounded. Worker startup has a 30 s deadline; first
input allows 15 s for client initialization, then missing input or worker progress
has a 2 s deadline. Newly pending work starts a fresh processing deadline after an
idle worker; client loading does not consume that progress allowance. An absolute
host-clock deadline of the nominal attempt duration plus 20 s (initial loading allowance and 5 s delivery grace) prevents slow inputs from
retaining scarce workers indefinitely.

The companion receives the same canonical input stream and published map as the
prediction client. It uses the existing native solver/scorer, without renderer,
input-device polling or resource callbacks. The host reads its bounded snapshots,
checks admission, contiguous ticks and input digest, and publishes a server-owned
acknowledgement. Client `Gameplay`, score claims and cosmetic animation events are
never consulted for native outcomes.

`score` contains native trick/landing/bail counters and names, clean/sketchy flags,
current multiplier, current `sequence`, current `line`, lifetime
`completed_lines`, and cumulative `awarded`/`publications` for this attempt.
`sequence` can be a live or latest settled sequence and resets under the native rules; it is **not** an attempt total.
`awarded` sums each finalized native reward exactly once at its publication
boundary, including the native multiplier and bail penalty. It includes isolated
tricks below the combo threshold that never enter a line. The sum uses f64 over
actual f32 native rewards; it does not feed back into native scoring math.
`publications` counts those publications, including zero or penalized rewards.
Line expiry and native reset preserve these attempt-wide observations.
`completed_lines + line` remains the native combo-line ledger, including expired
lines, and is a diagnostic subset rather than the complete awarded total. A resource
can rank `awarded` or apply its own explicit verified-outcome policy; intermediate
previews are not an independent bank. Cancelled/rejected attempts do
not produce a successful score.

## Prediction, reconciliation and retirement

Unsupported local marker returns, debug camera control, difficulty/tuning edits
and physical SDK/graph mutations are gated during the attempt. Existing local
physical mod state must be cleared before admission. Command-result wrappers
report denial; ordinary mapped input, cosmetic presentation and cleanup remain
available. These gates preserve prediction parity without changing ordinary
local-mod or peer-session behavior.

The graphical client asynchronously loads a canonical native simulation and
verifies the mounted world digest before admitting its controller stream. Normal
local physics pauses while that state loads. Thereafter the ordinary conditioned
controls drive the same native pipeline immediately, with a maximum 120 unacknowledged
ticks. The client compares each acknowledged native output digest with its retained
prediction and resends the oldest unacknowledged inputs.

A mismatch rebuilds **all** native physics, skater, controls, camera and graph state
from canonical initialization plus the bounded input history. It does not patch a
BODY pose into unknown hidden graph/solver state. The acknowledged replay digest
must match before replacement; persistent asset/settings/platform disagreement
ends the connection with an explicit error. A completed attempt stops at the
server's terminal tick, replaying/truncating any prediction lead if necessary.
This fails closed on cross-platform numerical disagreement; native Windows parity
has not been established.

Teleport, instance/readmission, disconnect, world replacement and resource
retirement cancel the old admission before queued worker results can award points.
A new co-occupant or shared geometry also cancels. Stale replay jobs cannot replace
client state after retirement. Dropping the worker kills/reaps it and retires its
scratch copy of the map. The new approved destination is never overwritten by an
old attempt's correction.

## Repeatable validation and current boundary

```sh
cargo test --locked -p skate-net --test native_authority
cargo test --locked -p skate-server --lib native_authority
SKATE3_ASSET_ROOT=/absolute/path/to/prepared/assets cargo test --locked -p skate-game --bin skate3rust physics::native_authority -- --ignored --nocapture
SKATE_NATIVE_EXE=/absolute/path/to/skate3rust SKATE3_ASSET_ROOT=/absolute/path/to/prepared/assets cargo test --locked -p skate-server --lib actual_native_worker -- --ignored --nocapture
python3 tools/verify_resource_native.py --assets /absolute/path/to/prepared/assets --bin-dir target/debug --output /tmp/native-authority-proof
python3 tools/verify_resource_native.py --impaired --assets /absolute/path/to/prepared/assets --bin-dir target/debug --output /tmp/native-authority-impaired-proof
python3 tools/verify_native_actions.py --scenario combo --assets /absolute/path/to/prepared/assets --executable target/debug/skate3rust --output /tmp/native-combo-proof
```

The graphical helper uses ordinary mapped stick inputs in a private instance and
requires a server-derived heelflip landing, matched prediction acknowledgements,
terminal reconciliation and public-instance return. Keep its assets and captures
outside source/packages; inspect its screenshots in addition to protocol assertions.
The optional `--impaired` run forwards both real clients through bounded loopback
UDP proxies with 25–75 ms delay per direction, 1% seeded loss and reordering.
HTTP resource downloads remain direct loopback. Its result records packet/byte
counts, losses, queue peaks and cleanup, without retaining packet payloads.
This is a local impairment experiment, not a WAN measurement.
The [evidence ledger](platform-extension-evidence.md) records which runs actually
passed. Reusing the complete native pipeline is not proof that every stock
flip/grab/grind variant or combo has been exercised. Full trick-family acceptance,
long native-attempt soak, Windows parity and broader multiplayer interaction
remain separate until evidenced. This mode does not expand course-v1's 0.20 m
animation-origin accommodation or claim arbitrary-rig collision proof.
