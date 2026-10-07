# Boardwalk Borough roleplay showcase

Boardwalk Borough is a connected, account-required server pack built from the
repository's original geometric town map and declared Lua/JavaScript resources.
It demonstrates a phone, player-to-player calls, opt-in proximity and dispatch
voice, a persistent apartment lease with guest invitations, and a server-owned
pizza delivery shift. Economy balances, leases, invitations and job recovery use
verified account identities. The clients cannot choose prices, payouts, routes,
instances or teleport coordinates.

## Build and install

Build a trusted server/account tool and the game with its browser companion. On
Linux, also install the browser build prerequisites in
[Resource browser interfaces](browser-interfaces.md). On Windows, the WebView2
runtime is required.

```sh
cargo build --locked -p skate-server -p skate-accounts
./BUILD.sh --browser
python3 tools/server_pack.py plan resources/packs/boardwalk-borough.recipe.json --source-root resources
python3 tools/server_pack.py apply resources/packs/boardwalk-borough.recipe.json --source-root resources --root /private/boardwalk-borough --server-executable "$PWD/target/debug/skate-server"
python3 tools/server_pack.py verify /private/boardwalk-borough
```

The recipe contains exact SHA-256 and byte-count pins for every selected
manifest file. It selects one required world, `boardwalk-borough`, and requires
account login. `apply` validates files and the resource manifests; it does not
run Lua/JavaScript callbacks or launch graphical clients. Keep the root writable
only by the server operator. The pack's `data/` directory contains mutable
resource and account stores; back it up while the host is stopped. Rollback
restores code/configuration, not database migrations or player progress.

## Account and server setup

Keep the administrator password, account database and TLS private key outside
the repository and pack. For a local test host, bootstrap an administrator:

```sh
python3 tools/server_pack.py init-accounts /private/boardwalk-borough administrator --account-executable "$PWD/target/debug/skate-account" < /private/admin-password
```

The generated account config listens on `127.0.0.1:31443`. For other machines,
edit its bind to the intended reachable interface and use a certificate whose
DNS name covers the public address. Give each tester a separate account and
private client credential file. The administrator can create player accounts
through the HTTPS admin UI.

Start the account-enabled server, replacing the bind address with the host's
chosen UDP interface:

```sh
target/debug/skate-server --test-world --bind 0.0.0.0:31030 --resources /private/boardwalk-borough/server.json --accounts /private/boardwalk-borough/accounts.json
```

For each client, build/prepare the game's normal assets, keep
`skate-browser-host` beside the executable, and connect with verified account
credentials:

```sh
./PLAY.sh --test-world --connect 127.0.0.1:31030 --account-config /private/client/player.json --voice
```

Use the server's reachable hostname instead of `127.0.0.1` for another machine.
Voice remains opt-in; hold **V** (or **LB** on controller) to transmit. The Phone
surface requires the browser companion. Calls and local mute/deafen continue to
respect the native device and physical push-to-talk gates.

## Demo loop

1. Join the Boardwalk Borough spawn and open the Phone. Verify the Calls, Jobs and
   Properties interfaces appear. Call another player in the same world and use
   push-to-talk; walk beyond the operator's 12 m proximity radius to confirm
   local voice separation.
2. Open **Pizza Shift**, go to Slice of Life Pizza, start a shift and collect the
   order. Skate to the server-selected marker. An out-of-order, wrong-instance,
   too-fast or forged request must not pay. A valid delivery adds the configured
   reward once and joins `voice-room/pizza_dispatch` while the shift is active.
3. Open **Properties**, rent the Borough Studio for the configured price, enter
   the private apartment, invite the second account, and confirm only the owner
   and invited account can enter. Exit to return to the saved street position.
4. Restart `rp-properties`, `rp-pizza`, then the server. Confirm the lease,
   wallet, completed delivery and invitation survive. Active jobs reconcile the
   same payout operation ID; private interior travel is restored safely when
   its owning resource stops.

## Ports, backups and rollback

Allow/forward the server's configured gameplay **UDP** port (31030 in the
example) and account login **TCP** port (31443 in the generated config). Use a
valid TLS certificate and distribute its trust certificate privately. Do not
use the bootstrap localhost certificate for a public hostname. Restrict the
HTTPS administration endpoint to trusted operators.

Stop the server before copying `/private/boardwalk-borough/data`. The account
store and resource databases contain private player data. Use
`python3 tools/server_pack.py history` and `verify` to inspect pack versions;
`rollback` changes the active code/config only. Back up player data separately
before changes that may add database migrations.

## Acceptance evidence

Automated resource/pack checks exercise the real resource Host, SQLite services,
marker exports, teleport leases, phone interface registration, and exact pack
pins. The networking workflow is configured to run these checks on Linux and
Windows. A green Windows compile or
resource VM test does not prove native WebView2, microphone, two-client gameplay
or WAN behavior.

| Scenario | Result | Evidence / prerequisite |
| --- | --- | --- |
| Lua/JavaScript resources, wallet and roleplay flows | Automated pass | `gameplay_resources` test suite; see branch CI |
| Phone app registration and owner retirement | Automated pass | `phone_ui` resource Host tests; native browser cleanup is owned by the game host |
| Pack closure and file digests | Pass | `test_boardwalk_borough_pack.py` plus `server_pack.py plan/apply/verify` with the trusted server binary |
| Account-enabled packaged server startup | Local headless pass | Applied and verified the pinned pack, initialized a disposable local account authority, and ran the server for 3 seconds on ephemeral loopback UDP/TCP ports; no clients joined |
| Two Linux graphical clients | Not run | Needs prepared game assets, browser companion, two accounts, microphones and output devices |
| Two Windows graphical clients | Not run | Needs Windows clients, WebView2 runtime, two accounts and audio devices |
| Mixed Linux/Windows clients | Not run | Needs both graphical setups and two verified accounts |
| WAN clients | Not run | Needs a reachable host, DNS/TLS certificate, forwarded UDP/TCP ports and clients on separate networks |

Record operating systems, browser readiness, voice devices, packet loss/RTT and
forwarded ports when running graphical acceptance. Keep passwords, account IDs,
private maps and packet captures with identifiers out of this document and Git.
