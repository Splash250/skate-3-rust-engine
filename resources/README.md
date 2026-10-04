# Redistributable resource examples

These original Lua scripts and the text UI asset are distributed under the
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
