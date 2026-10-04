# Verified course competitions

The `resource.competition` server capability defines resource-owned checkpoint,
pickup, timing and shared-entity-contact competitions. The host derives outcomes
from server-owned definitions, server time and admitted movement observations.
Clients cannot submit scores, finish times, checkpoint indexes or contact claims.
Lua/JavaScript use `resource.competition.submit`; C# uses `Resource.Competition`.
Client-side calls are rejected. The reusable
[verified-course example](../../resources/verified-course/README.md) runs in the
redistributable community park.

```lua
resource.competition.submit({kind="define",course={
  name="sprint",instance="0",
  checkpoints={{position={5,1,0},radius=1,points=50}},
  pickups={{position={3,1,0},radius=0.5,points=10}},
  contacts={{entity="finish_bell",points=20}},
  limits={max_speed=15,max_acceleration=40,max_gap_ms=500,max_airborne_ms=2500}
}})
resource.competition.submit({kind="start",name="sprint",player=actor_id})
resource.competition.submit({kind="cancel",player=actor_id})
resource.competition.submit({kind="remove",name="sprint"})
```

`instance` and `player` are canonical decimal strings. Only an admitted player
already in the course instance may start. Starting teleports that player to the
verified resource world's approved spawn with zero velocity and a fresh movement
epoch. Courses require that world; they cannot choose an arbitrary client
position as a trusted starting point. Commands complete through
`competition_result` with `{operation,ok,value/error}`. An attempt later emits
`{kind="completed",player,course,score,elapsed_ms,checkpoints,pickups,contacts,
verified_rules="course-v1"}` or `{kind="rejected",player,course,reason,score=0}`.
A later trusted teleport, instance change, readmission or disconnect cancels the
attempt with `{kind="cancelled",player,course,reason="movement_epoch_changed"}`;
it does not award a score or override the new destination.
Resource state, backend persistence, or rewards can consume that host callback.
There is no generic server score setter or magic resource ID.

Checkpoints are ordered, at most one per fresh accepted observation. Pickups and
contact targets award once per attempt. All declared targets must be reached to
complete; partial points never become a successful result. A contact target is
the named shared entity belonging to the same resource generation and instance.
Static and server-simulated dynamic shapes qualify; kinematic shapes do not.
The host intersects the validated player capsule segment with their current
authoritative shapes/poses. These awards mean a geometric target intersection;
they are not native solver-contact events or a historical sweep of a moving
object. Actual shared-object solver contacts have separate diagnostics.
Completion is emitted once and removes the active attempt.

## Verified rules and authority boundary

The input remains client-owned character movement, but the competition host
independently validates every accepted segment against a restrictive envelope.
It reads the actual admitted BODY sample sequence and its server receipt time;
repeated snapshots cannot create extra scoring samples. Intervals below20ms
coalesce before evaluation. A sample gap over the configured100–500ms rejects
the attempt. Host wall-clock delivery time, never a client tick/time field,
determines speed, acceleration, the displacement window and elapsed finish time.

The default speed limit is15m/s (configurable1–25), acceleration40m/s²
(1–80). A segment permits0.08m positional tolerance; the rolling1s path-distance
window permits only0.15m total tolerance, so a packet flood cannot repeatedly
mint displacement allowance. Derived velocity change permits1.5m/s jitter
tolerance. These values are a strict game-mode policy, not a recovered retail
anti-cheat model. Changed movement epochs/instances cancel scoring, including
otherwise authorized teleports during a run. Trusted epoch/instance changes are
checked even while readmission hides BODY observations. Missing/stale observations
within an unchanged movement epoch time out.

Collision validation uses Rapier/Parry's continuous capsule-versus-triangle-mesh
cast against the server-selected world, including initial overlap checks.
The upright capsule has a 0.3 m radius and 0.5 m half-height, with its center 0.85 m
above its collision reference and 0.05 m floor clearance. Native BODY positions are
animation origins, which can lie slightly below the floor when riding. If trusted
terrain support lies above that origin by at most 0.20 m, the verifier raises only
the collision reference to the support height. Deeper roots fail collision
validation. It keeps the full capsule, wall/ceiling sweeps and raw-root movement,
speed, acceleration and checkpoint measurements; it does not ignore floor
triangles. Entity-target intersections use the same collision reference. This
bounded course-v1 policy is not a guarantee for every rig, bail pose or native
trick. A downward support ray checks up to 1.8 m from 0.3 m above the root;
uninterrupted unsupported travel
defaults to2500ms and may be configured100–4000ms. Invalid motion rejects all
points for that attempt and issues another server-approved spawn correction.

These rules block direct score/time claims, movement warps, excess speed or
acceleration, sustained unsupported flight and crossing a wall between samples.
They cannot prove that a plausible trajectory came from an unmodified skating
client. They do not derive flip/grab/grind trick identity, native contact state,
combo multipliers, or the full articulated character simulation. The stock
`Gameplay` trick/score fields remain owner-reported metadata and must not be used
as verified rankings. The opt-in [native-input-v1 companion](native-skating-authority.md) independently
simulates bounded controller streams with the recovered engine in a solitary
instance. Its guarantees and acceptance are separate; `course-v1` identifies this
narrower trajectory rule set explicitly.

## Bounds and lifecycle

Commands are limited to16KiB serialized JSON. A resource may define4 courses;
the host caps32 courses and64 active attempts, one per actor. Each course permits
32 ordered checkpoints,32 pickups and16 distinct contact targets. Target radii
are0.5–5m, each award0–10000 points, and positions are finite below100km per axis.
The movement window retains at most64 samples per attempt; callbacks retain at
most128 events. Collision is built once per verified world revision with at most
262144 triangles. It reuses the existing Rapier dependency rather than introducing
another physics implementation. Larger valid worlds, or spawns incompatible with
the simplified capsule, still load normally; starting a course returns an
explicit verification-unavailable error instead of allocating an oversized
collision verifier or weakening its rules.

Resource stop/restart removes that generation's rules, attempts and pending
events. World replacement cancels attempts before another world can award points.
Rules cannot be replaced while their attempts are active. Reconnects receive new
actor identities in account-required mode; use the
[account identity snapshot](accounts-and-administration.md) to attach verified
results to persistent accounts, rather than trusting account names in payloads.

Run `cargo test --locked -p skate-server --test competition`. The deterministic
protocol fixture uses admitted client/server sessions and synthetic terrain to
compare a legitimate trajectory with forged score metadata, a jump and a wall
crossing, including server correction, lifecycle and contact awards. This is
protocol/rules evidence; it does not claim a complete native graphical skating
race or Internet-latency playtest.

## Native course acceptance

`tools/verify_resource_course.py` runs two native clients and a dedicated server
against the redistributable community park. Supply locally owned prepared
assets and matching freshly built client/server executables:

```sh
python3 tools/verify_resource_course.py --assets /path/to/prepared/assets \
  --bin-dir target/debug --output /tmp/native-course-proof
```

The helper creates a temporary, capability-granted resource beneath the new
output directory. It mounts the selected player with mapped action79, starts a
normal server-owned course at the world's trusted spawn, and pulses action80
to push through three ordered checkpoints and a pickup along the clear floor.
After a terminal host result, action81 brakes. It never submits score, position,
velocity, or another teleport during the attempt. The other client observes
the same native movement.

The evidence validator requires exactly one authoritative `course-v1`
completion awarding175 points, positive server elapsed time, native onboard
observations spanning the route, and no native travel/reconciliation while
pushing. Rejection, cancellation, missing targets, stopped resources, a loading
menu, or missing screenshots fails the run. Results, unfiltered logs, and both
native screenshots remain under the output directory, including on failure.
Review those screenshots separately; successful protocol assertions alone do
not establish correct rendering or animation.

This narrow, ordinary-input acceptance still does not establish server-derived
flip/grab/grind scoring or resistance to every plausible forged trajectory.

The Linux two-client acceptance on2026-10-04 completed all three checkpoints
and one pickup for175 points in2545ms of server time. Ten distinct native
onboard observations covered5.6120m with no extra native reset during the run;
both clients exited successfully after their periodic and final captures.
The first run correctly exposed an incompatibility between the animation-root
height and the simplified collision capsule during landing. The support-aware
root correction described above passed its rejection regressions before the
fresh-server native rerun passed. The final capture also verifies the example
floor's one-centimetre separation from the park floor, with all three shared
objects still collidable and no coplanar rendering artifact.
This was local-loopback evidence with owned
native assets, not a Windows or Internet-latency playtest.
