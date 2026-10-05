# Dedicated multiplayer

`skate-server` is a headless IPv4 UDP host for up to 16 players. Clients send
movement, all 33 physical bodies, skeleton poses, trick/landing/bail state and
score presentation. The server validates membership and compatible game data,
routes those streams, and authorizes shared player collisions and shoves.

For the resource-driven RP profile, see [interaction menu and phone setup](resource-interactions.md):
in-game HTML interfaces, player calls, local photographs and permission-controlled
dashboards. Its [evidence matrix](resource-interactions-evidence.md) records native
Linux results separately from physical-device and Windows acceptance.

## Build and start

From the repository root, with the existing Rust toolchain:

```sh
cargo build --locked --release -p skate-server
./target/release/skate-server --map /path/to/University.skate
```

The server reads the map once to fingerprint its bytes. It does not load map
geometry or require Bevy, a display, Steam, character assets or a game instance.
On Windows, use `target\release\skate-server.exe` and a Windows map path.

### Windows server package

From PowerShell on Windows with stable Rust (MSVC x64), Visual Studio C++
Build Tools/Windows SDK, and Python 3.11 or newer:

```powershell
.\scripts\Build-ServerRelease.ps1
```

This builds only the dedicated server with the MSVC runtime linked statically.
It writes `target/skate-server-windows-x64.zip` and its `.zip.sha256` checksum.
The archive contains the executable, usage instructions, license and
`server-build.json` (source revision and executable hash). Extract it anywhere
and run `skate-server.exe --test-world` or pass your own map. The host does not
need Rust, Python, Steam or the game installation.

The **Networking and dedicated server** workflow builds and tests the package on Windows and
uploads the ZIP/checksum as the `skate-server-windows-x64` Actions artifact.
This is separate from the game release/updater package; it does not publish a
release or include maps or Steam files.

### Connect clients

Run a client built from the same version, with its normal prepared assets:

```sh
./PLAY.sh --connect 127.0.0.1:31030 --map /path/to/University.skate
```

On Linux, `PLAY.sh` also sets the required dynamic-library paths. Use
`SKATE_RELEASE=1 ./PLAY.sh` for a release client. On Windows, pass the same
`--connect` and map arguments directly to `skate3rust.exe`.

Use the host's reachable IPv4 address for another computer. Everyone must have
the same map bytes locally; filenames and directories can differ. Prepare the
client using the normal [build instructions](../../README.md#build) or
[Linux guide](../LINUX.md). Server-selected Lua resources and explicitly listed public assets are downloaded
when the server enables the [resource platform](resources.md). Existing local
`mod.json` packages remain disabled for launches with `--connect`; downloaded
resources use their own grants, lifecycle and persistent verified cache. Stock
maps and character assets still need local preparation.

For the built-in procedural test world, pass `--test-world` instead of `--map`
on both server and clients. Game clients still need their usual local character
and animation assets. The server's automated tests use synthetic data only.

```sh
./target/release/skate-server --test-world --bind 0.0.0.0:31030 --max-players 16
./PLAY.sh --connect 127.0.0.1:31030 --test-world
```

The default server bind is `0.0.0.0:31030`. Allow/forward that UDP port when
hosting across a firewall/router. Stop the server with Ctrl+C; clients detect
lost connections through the existing timeout. `--help` lists server options.
For a custom session number, use `--session 12345` on the server and
`--net-session 12345` on every client. Session numbers are routing identifiers,
not passwords. Dedicated `--connect` cannot be mixed with `--net-host` or
`--net-local`. Changing maps leaves the session; restart with the server's map
to reconnect. Restart without `--connect` to return to other multiplayer modes.

## Gameplay

- Movement and poses use the existing delta compression, interpolation and
  distance-based update rates. Ragdolls include all physical body transforms and
  velocities. Dedicated sessions use locally available stock remote models.
- Built-in trick and landing labels appear with remote player nametags. Current
  trick, landing/bail sequence and scores travel independently of the mod system.
- Press the right bumper while facing a nearby player to request a shove. The
  server checks fresh body state, range, facing, player state, action sequence and
  cooldown. An accepted shove triggers the existing attacker animation and
  victim wipeout path, then applies the server's velocity change.
- The server computes conservative player collision responses. Clients turn off
  the old independently computed remote-player response in dedicated sessions.
  Shared collision/shove effects are retained until applied and acknowledged;
  duplicate deliveries do not apply an impulse twice. A client that stops
  acknowledging effects is disconnected when its bounded queue fills.

## Authority and current limits

This implements the repository's [Hybrid Authority](../../CONTEXT.md) model.
The owning client predicts detailed skating, articulated bodies and world
collision. The server checks snapshots and computes coarse shared player
contacts; it does not run the retail map collision solver or replay every
player's inputs. Trick/score data is presentation, not a trusted leaderboard.
Shared contacts use a 0.45-metre horizontal player radius and a 1.8-metre
vertical separation limit, including crossings between fresh snapshots. They
do not resolve individual limbs or loose boards. A distant respawn/marker
teleport can pause replication for about a second while establishing a new
baseline; it does not sweep a collider through the intervening world.
Two-client visual playtesting is still needed to tune collisions and fighting
across real networks and different game states.

The server requires its configured map fingerprint. The first admitted player
pins the rig and physics fingerprints for that server run; later players must
match. Dedicated and legacy lobby handshakes are distinct. Names, built-in gameplay and bounded resource control records are allowed;
arbitrary application records and legacy blob transfers remain rejected.
Resources use a separate bounded TCP HTTP content endpoint and gameplay
admission waits for verified resource activation. This direct-connect platform
has no account authentication, encrypted transport, matchmaking or competitive
anti-cheat. See [resource setup and limits](resources.md).

The design draws on FiveM's [OneSync ownership and relevance model](https://docs.fivem.net/docs/scripting-reference/onesync/),
its distinction between [server creation and client simulation](https://docs.fivem.net/docs/scripting-manual/migrating-from-other-platforms/),
and [server validation of client events](https://docs.fivem.net/docs/developers/server-security/).
Its [cancelable damage request](https://docs.fivem.net/docs/scripting-reference/events/server-events/#weaponDamageEvent)
is an example of server authorization. The protocol and collision behavior here
are specific to this engine, with no FiveM dependencies.

## Windows and Linux interoperability

Use server and clients built from the same source version, with identical map,
rig and physics data. The UDP wire format uses fixed-width little-endian values
and platform-independent JSON gameplay/effect records; map identity hashes file
bytes, so Windows and Linux paths may differ. Golden-wire tests pin the actual
handshake, snapshot and gameplay/effect encodings on both platforms. Each CI
runner also exercises two clients over real UDP sockets, and the Windows job
repeats an executable-level session against the extracted release package.

These checks cover protocol compatibility and each operating system's socket
path. For live mixed-platform validation, run a Windows server with a Linux
client and a Linux server with a Windows client using `--test-world`, then
repeat with identical `.skate` bytes. Verify remote movement, all-body ragdoll,
trick labels, shove effects, leave/rejoin and timeout recovery. Graphical/WAN
playtesting remains separate from automated headless tests.

## Optional Steam lobby transport

The existing peer-hosted Steam lobby/relay mode also supports native x86_64
GNU/Linux clients. Build with `./BUILD.sh --steam` (or
`./BUILD.sh --release --steam`), open the native Steam client and sign in, then
use the game's existing Steam host, browse or join controls. See the
[Linux Steam instructions](../LINUX.md#optional-steam-lobbies) for staging and
runtime details. Windows and Linux use the same lobby namespace, Steam channel
and packed messages; two-player testing requires different Steam accounts.

Default Linux builds, solo play and direct UDP `--connect` need no Steam.
Steam is loaded only by the optional helper process. Missing relay files or
Steam initialization failure do not disable direct UDP. Steam integration into
dedicated servers remains a separate follow-up: `--connect` continues to select
the dedicated UDP transport, and dedicated sessions do not host Steam lobbies.

## Validation

```sh
cargo test --locked -p skate-net
cargo test --locked -p skate-server
cargo check --locked -p skate-game --bin skate3rust --features dev-dynamic
cargo test --locked -p skate-game --bin skate3rust --features dev-dynamic dedicated
cargo test --locked -p skate-game --bin skate3rust --features dev-dynamic physics::network::delta::tests
```

Networking tests exercise admission, packet loss/reordering, body/pose/gameplay
replication, effect acknowledgements, shove validation, shared collisions and
legacy behavior. Server tests use actual loopback UDP sockets, without launching
the renderer or loading private assets. They do not replace gameplay testing.

The networking CI runs the full `skate-net` and `skate-server` suites on both
Windows and Linux, including `wire_compat` golden fixtures and the `package`
executable smoke test. To test an extracted Windows release explicitly:

```powershell
$env:SKATE_SERVER_EXE = (Resolve-Path 'C:\server\skate-server.exe').Path
cargo test --locked -p skate-server --test package
Remove-Item Env:SKATE_SERVER_EXE
```

Without that override, the test launches Cargo's newly built server. It uses an
empty working directory and two actual UDP clients to check admission,
body/pose/trick replication, shove/collision effects, acknowledgements and
departure. Package-content/checksum tests use synthetic executables:

```sh
python3 -m unittest tools.test_package_server -v
```

To exercise the actual stock shove animation, wipeout lifecycle and impulse
application with locally prepared assets, also run:

```sh
SKATE3_ASSET_ROOT=/path/to/assets cargo test --locked -p skate-game --bin skate3rust --features dev-dynamic physics::network::dedicated_asset_tests -- --ignored
```

The asset root contains `private/`. These two tests are explicitly ignored in
ordinary runs because the repository does not distribute the owned game data.

The [platform extension](platform-extension.md) documents shared entities,
movement epochs, 64-client capacity, bounded large values, browser interfaces,
voice, accounts, backend services, resource worlds, animation and Lua/JS/C#.
See its [evidence ledger](platform-extension-evidence.md) for open requirements.
