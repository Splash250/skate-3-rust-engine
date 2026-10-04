# Composable dedicated gameplay

The six `platform-*` resources are original, replaceable Lua packages using the
existing resource lifecycle, scoped state, settings, commands and services.
Exact `1.0.0` dependencies are declared in their manifests. Native pause-menu
entries appear under **Server** after content and settings admission. They use
no retail content, external service or client-reported score authority.

## Install and run

Use the pinned [server packs](server-packs.md):

| Recipe | Initial world | Gameplay | Account prerequisite |
| --- | --- | --- | --- |
| `resources/packs/free-skate.recipe.json` | Community Practice Park | Profiles, crews, rounds, voting/rotation | Anonymous skating works; verified profiles/crews require account-enabled launch |
| `resources/packs/tournament.recipe.json` | Creator Courtyard | All six packages; course-v1 tournament mode | Initialize and launch the account authority |
| `resources/packs/community-park.recipe.json` | Creator Courtyard | All six packages; free-skate rounds, optional tournaments | Initialize and launch the account authority |

Both original parks are pinned in every recipe. Only one required world runs at
a time. The operator's `world_rotation` list is the authority for selection;
resource settings cannot authorize arbitrary filesystem worlds. Do not edit
installed immutable files. Review and install a newly pinned recipe for upgrades.
Account bootstrap stores the authority under the same private `data` root as
resource persistence, so a supervised stopped-store backup includes both.

```sh
cargo build --locked -p skate-server -p skate-accounts
python3 tools/server_pack.py plan resources/packs/tournament.recipe.json --source-root resources
python3 tools/server_pack.py apply resources/packs/tournament.recipe.json --source-root resources --root /tmp/skate-tournament --server-executable "$PWD/target/debug/skate-server"
python3 tools/server_pack.py init-accounts /tmp/skate-tournament administrator --account-executable "$PWD/target/debug/skate-account" < /private/admin-password
# Use the generated stable config pathname on every launch.
target/debug/skate-server --test-world --bind 127.0.0.1:31030 --resources /tmp/skate-tournament/server.json --accounts /tmp/skate-tournament/accounts.json
```

Create player accounts and local login profiles using
[accounts and administration](accounts-and-administration.md). The new dedicated
browser accepts each profile's local pathname; it never asks discovery metadata
where to send credentials. Profile JSON, passwords, certificates and stores are
private local prerequisites, never recipe artifacts.

In the game, open **Multiplayer → Dedicated servers**, enter/save the endpoint
and optional account profile pathname, refresh and join. Resource capability
approval and download progress use the existing client admission surface.
Open **Server** for crew controls, round entry, voting and tournament enrollment.
The host console starts an enrolled tournament with:

```text
command tournament_start
command tournament_cancel
command tournament_reset
command map_vote
command round_start 0
settings platform-rounds
setting platform-rounds duration_seconds 120
setting platform-map-vote vote_seconds 15
```

Commands use `tournaments.manage`, `maps.manage` and `rounds.manage`. Local host
console is trusted; resource command dispatch retains the host's permission
checks. These are not network events, and ordinary clients cannot invoke them by
sending a similarly named event. The HTTPS admin exposes typed settings and
resource lifecycle actions with live authorization/audit checks.

## Contracts

Export calls use `resource.call(resourceId, exportName, value)`. Network sender
identity is supplied by the host. A client field named `account`, `player` or
`score` never becomes an authenticated identity or verified result.

| Resource | Exports | Network actions and state | Persistence / settings |
| --- | --- | --- | --- |
| `platform-profiles` | `identity(actor)` → verified account ID or nil; `profile(actor)` → cached profile or nil | `rename {name}` changes only the sender's ASCII 1–24 character name; player-private `profile` state | SQLite account-keyed name and visits; private `welcome_name` default |
| `platform-crews` | `crew(actor)` → crew or nil | `crew {action,name?,actor?}` supports create/invite/accept/leave/kick; private invitation and membership | One atomic resource storage value; max 128 crews; live `max_members` (2–16) |
| `platform-rounds` | `begin {instance}` → `{ok,id/error}`; `finish(instance)`; `inspect(instance)` | `join` uses sender's current instance; instance-scoped `round` | Durable monotonic serial, transient rounds; max 64 instances; `duration_seconds` applies to subsequent rounds, `mode` requires restart |
| `platform-map-vote` | `options()` → ordered IDs | `vote {map}` only during the window; public `vote`, `rotation_result` | Transient votes; live comma-separated `rotation`, `vote_seconds`; bounded 256 voters |
| `platform-leaderboards` | `start {player,rules,ticks?,tournament?}` → `{ok,id/error}`; `cancel(id)`; `result(id)`; `top()` | No network score-submission endpoint; public verified `top` and latest result | SQLite results/best, durable replay outbox/serial; restart-only public `season` |
| `platform-tournaments` | `inspect()` → phase, entrants, results | Verified sender `enroll`/`leave`; public `tournament` | Durable tournament serial; transient enrollment/bracket; `rules` restart-only, `ticks` and `max_entrants` live |

Crews bind membership and invitations to verified account IDs. Reconnecting with
a new connection actor preserves membership; leader-only changes recheck that
account. Invitations expire after 60 seconds and do not survive resource restart.
When a leader leaves, the lexicographically first remaining account becomes
leader. Empty crews are removed. A crew is a social group; it does not grant
roles, reserved admission or authority over other players.

Rounds snapshot duration/mode at creation, auto-track admitted actors and remove
disconnected/moved actors. Resource restart creates a new round serial and clears
transient sessions. The map voter uses one vote per verified account, or per
unverified development actor on anonymous servers, with a one-second cooldown.
Ties select the earliest configured option. Closed rounds open one vote window;
the host validates selection against its allowlist. World startup failure
restores the prior running set and dependents; unrecoverable restoration fails
closed. Successful replacement retires dependencies tied to the old world;
general gameplay packages intentionally do not depend on a particular map.

## Verified competition and durable results

Tournaments enroll verified accounts, then schedule one connected actor at a time
in a vacant instance1000–1063. Missing/replaced connections forfeit the current
turn; a fresh actor cannot finish an old actor's attempt. The previous position
and instance are held by a host-owned temporary teleport lease. Completion,
cancellation, resource retirement and replacement release that lease through
the normal server teleport/admission path. Another authority's movement reset
invalidates stale restoration instead of teleporting the actor back unexpectedly.

The default `course-v1` round is a fixed three-checkpoint practice route along
X=-8,-4,0 at Z=0, worth50,50,75 points. The existing server verifier checks actual
accepted motion, raw movement bounds, swept walls/ceilings and its bounded
BODY.root terrain-reference policy. These are course outcomes, not claims that
the host simulated every trick or proved a full-rig trajectory.

To use `native-input-v1`, configure the existing trusted native companion as in
[native skating authority](native-skating-authority.md), then change
`platform-tournaments.rules` and restart that resource. These packs do not fetch
a companion executable or bundle private assets. Native attempts stay solitary
and static-world. Shared entities/dynamic rails, competing owners and unsupported
instances retain the existing rejection/cancellation rules. Configuring another
resource to place dynamic objects there does not disable the guard.

The leaderboard starts the verifier itself and accepts only matching host-local
`competition_result` completions with the expected actor, verified rules and
still-current account. It never subscribes to network events for results. Native
scores use the host's `score.awarded`; cosmetic trick events and reported scores
are irrelevant. Rules are separate ranking categories. The replicated standings
and `top()` export contain up to ten best accounts per rule (twenty total), ordered
by rule, descending score and account for ties. The client overlay shows five
places in each populated category, so course participation cannot hide native
rankings. Commit and startup recovery use the same category limits.

Before database submission, the result is written to a durable resource outbox.
One transaction inserts a unique attempt ID, updates the account's best for that
season/rule and reads standings. Duplicate replay cannot insert a second result
or add scores. The outbox is cleared only after successful completion. Restart
and commit-before-ack recovery replay that transaction; failure leaves the outbox
pending and blocks further attempts until retry/recovery. A tournament displays
`committed` only after that database acknowledgment. The storage serial is never
rolled back independently of the database; take a consistent stopped-store
backup of the whole `data` tree before changing schemas.

Enrollment, invitations, active rounds and in-flight competitions deliberately do
not resume across process restart. Connected actors disappear with the process;
a new server login begins ordinary admission. Durable profiles, crews, serials,
completed verified results and pending outboxes recover. Code rollback keeps data
and cannot undo database migrations. Resource replacement must retain compatible
exports/schema or explicitly migrate them.

## Validation

`cargo test --locked -p skate-server --test gameplay_resources` uses actual Lua/JS
hosts, the real services/SQLite worker, real UDP movement and the existing
competition verifier. It covers forged identities/events, late/reconnected actors,
crew/resource replacement, map-vote permissions, client startup without state,
course completion, outbox crash windows and temporary-room retirement.
`tools/verify_platform_capabilities.py` exercises actual Linux game clients,
X11 keyboard browser joining, settings/restart, authored worlds/rotation and
portable profiles. See [the evidence matrix](platform-capabilities-evidence.md)
for actual fresh results and limits; synthetic trusted native result injection
proves persistence contracts only, not native solver/trick acceptance.
