# Redistributable resource examples

These original resource scripts, UI assets and the authored community park are distributed under the
repository's [GPL-3.0 license](../LICENSE). They contain no retail game data,
credentials, downloaded native code or CitizenFX source.

- `skate-rules` is a shared dependency exporting `landing_point` and
  `progress_label` on both sides.
- `landing-challenge` runs server-owned, 60-second rounds. Joining skaters race
  to three observed landings. A downloaded client script renders a canvas and
  adds Join/Leave controls to the Challenges menu. The server validates enrollment
  source, payload and rate, owns round state, and stores aggregate totals.
- `server.json` selects the challenge and grants only its declared capabilities.

From the repository root:

```sh
cargo run --locked -p skate-server -- --test-world --resources resources/server.json
./PLAY.sh --connect 127.0.0.1:31030 --test-world
```

Clients still need their normally prepared character/animation assets. The
procedural map and resources do not substitute for those assets. The challenge
automatically requests enrollment; use **Challenges → LANDING CLUB** to leave or
rejoin. Native landing observations advance points; the example never accepts a
client-provided score or winner in its enrollment event. Nevertheless, landing
observations originate on the owning client: this cooperative example is not a
verified competitive leaderboard.

The server console supports `command challenge_reset`, `stop landing-challenge`,
`start landing-challenge`, `restart landing-challenge` and `ensure landing-challenge`.
The reset command requests the `challenge.admin` permission; the local server
console is trusted. `quit` shuts down cleanly. Stopping the resource must remove
its UI, handlers and timers even when its Lua cleanup callback fails.

Aggregate round/landing totals persist below `resource-data/`, outside the
download set. They are not player accounts or permanent player rankings.
`server.lua`, `server.json` and the persistence directory must never be published
to clients. The public set consists only of shared/client scripts and listed
files. Edit `shared.lua` to change the round policy; reconnect/update content must
use new exact digests even if the resource's version label is unchanged.

See the [authoring API](../sdk/RESOURCES.md),
[server and cache guide](../docs/multiplayer/resources.md) and
[Cfx comparison](../docs/multiplayer/resource-compatibility.md).

## Platform extension examples

Run `cargo run --locked -p skate-server -- --test-world --max-players 64
--resources resources/platform-examples.json` (one command line). This selects:

- `cross-language-demo`: downloaded JavaScript client/server scripts call a Lua
  dependency which calls a JavaScript rules export. Its lightweight engine UI
  shows results; this is not an HTML browser host.
- `persistent-progression`: SQLite migrations and transactional inventory, with
  trusted console commands documented in its README. `progress_award test 100`,
  `progress_buy test`, and `progress_inspect test` are prefixed with `command`.
  Labels are local administrator data keys, not authenticated player identities.
- `shared-objects`: authoritative moving objects and private instances. Its
  server console commands are documented in its README.
- `inventory-ui`: a downloaded HTML inventory backed by authenticated accounts
  and SQLite transactions. Start the server with `--accounts` and the game with
  `--account-config`; see [account setup](../docs/multiplayer/accounts-and-administration.md).

Optional examples are granted in the config but added to `ensure` explicitly:
`voice-room` supplies radio administration and proximity controls (`--voice` is
the local client opt-in); `managed-language-demo` requires the isolated .NET
worker. Add `community-park` to select its downloadable map and native rail.
Only one required resource world may be active at a time.
`verified-course` depends on that park and adds server-validated checkpoints,
pickups and a private competition instance; see the
[competition contract](../docs/multiplayer/verified-competitions.md).
`presentation-demo` imports an original clip, mascot and bone attachment and
replicates their descriptors within each instance; see its
[setup and controls](presentation-demo/README.md). Clients must explicitly grant
its `engine.animation` capability for the server origin before activation.

Lua and QuickJS are embedded; SQLite and HTTPS support are built into the server.
Neither Node.js nor a database service is required. The authenticated inventory
uses locally created credentials. Browser pages require the separately built
`skate-browser-host` companion and its platform prerequisites; see
[browser interfaces](../docs/multiplayer/browser-interfaces.md). Managed resources
require .NET10 and the trusted worker. See [voice setup](../docs/multiplayer/voice.md).
See [extension evidence](../docs/multiplayer/platform-extension-evidence.md) for
precise implemented and open capabilities.
