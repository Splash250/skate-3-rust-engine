Skate dedicated server for Windows x64
=====================================

Extract the entire ZIP, open PowerShell in this folder, and run:

    .\skate-server.exe --test-world

Run the included Lua landing challenge, dependency and downloadable UI:

    .\skate-server.exe --test-world --resources resources/server.json

The example saves private server progress in resource-data next to this
README. See resources/README.md for lifecycle commands and configuration.

Or host a map you already have:

    .\skate-server.exe --map "C:\Maps\University.skate"

The server reads the base map to fingerprint it. With resources enabled it also
reads bounded spawn metadata for recovery after removing a resource world.
Required resource worlds load their validated collision and rails on the server.
The base map does not otherwise require geometry decoding. Dedicated hosting
does not load character assets, Bevy, a display, Steam, or a running game. Rust and Python
are not required on the server machine. No retail assets are included.

Connect with a Windows game client built from the same source version:

    .\skate3rust.exe --connect 127.0.0.1:31030 --test-world
    .\skate3rust.exe --connect 127.0.0.1:31030 --map "C:\Maps\University.skate"

On Linux, use the game's launcher with the same arguments:

    ./PLAY.sh --connect 127.0.0.1:31030 --test-world

Use the host's reachable IPv4 address instead of 127.0.0.1 for another
computer. Windows and Linux clients use the same dedicated UDP protocol.
All clients need their normal prepared character and animation assets;
all participants must use identical map bytes (paths may differ). The first
player pins the rig and physics fingerprints for that server run.

Hosting options
---------------

    .\skate-server.exe --test-world --bind 0.0.0.0:31030 --max-players 16
    .\skate-server.exe --help

The default is IPv4 UDP port 31030, 16 players by default (configurable up to 64), session 48031030.
Allow the selected UDP port through the host firewall and forward it on
your router for Internet hosting. Resource hosting additionally uses TCP
on the same port for bounded HTTP content downloads. Use
--bind 127.0.0.1:31030 for local-only testing. Enter quit for resource cleanup
and graceful shutdown. Restart the server and clients to change maps.

For a custom session, pass --session 12345 on the server and
--net-session 12345 on every client. Session numbers are not passwords.
Without --accounts, development hosting has no account authentication or
gameplay encryption. Optional local TLS accounts and encrypted authenticated
gameplay are described below. No matchmaking service is provided. Only
the configured resources' declared client/shared scripts and files are
downloaded; private server scripts/configuration are excluded. Resources
use sandboxed Lua/QuickJS and capability grants. Legacy local mods are skipped in
dedicated sessions. Do not combine --connect with --net-host or --net-local.

The owning client simulates detailed skating, articulated bodies, and map
collisions. The server validates membership and compatible data, routes
movement/pose/trick state, and authorizes coarse player collisions and
shoves. These ordinary client-reported scores are presentation data. Resources
can separately use course-v1 for verified checkpoints/pickups/contacts, or opt
in to native-input-v1 for server-simulated native outcomes in a solitary static
world. Native authority requires a separately supplied matching skate3rust.exe
companion and the operator's prepared owned assets; neither is bundled here.
See docs/multiplayer/native-skating-authority.md for its exact boundary and
prerequisites. Windows runtime acceptance remains open.

Optional local accounts and administration
-----------------------------------------

The standard server build includes skate-account.exe. Prepare a private file
containing a 12-1024 byte UTF-8 administrator password; do not place passwords
in command-line arguments or public resources. In PowerShell, use cmd for
stdin redirection (the secret itself is never a command argument):

    cmd /d /c ".\skate-account.exe init local-accounts administrator < C:\Private\admin-password.txt"

The new local-accounts directory must not exist. Initialization creates a
private database, certificate.pem and private-key.pem. Create accounts.json
alongside it, with paths relative to that configuration:

    {"database":"local-accounts/accounts.sqlite3","bind":"127.0.0.1:31443","certificate":"local-accounts/certificate.pem","key":"local-accounts/private-key.pem"}

    .\skate-server.exe --test-world --accounts accounts.json --resources resources/platform-examples.json

Open https://localhost:31443 after trusting this installation's certificate
in your browser. The UI manages real accounts, roles, moderation and resource
lifecycle. Enabling --accounts makes verified login and encrypted gameplay
mandatory; plaintext clients are rejected. Create player accounts in the UI.

Each client supplies a private credential configuration such as:

    {"endpoint":"https://localhost:31443","ca_certificate":"C:/Private/certificate.pem","username":"player","password_file":"C:/Private/player-password.txt"}

    .\skate3rust.exe --connect 127.0.0.1:31030 --test-world --account-config C:\Private\client.json

Protect password files with a current-user-only Windows ACL. Do not distribute
the server private key/database or player passwords. The generated certificate
covers localhost and loopback only; remote hosting requires a certificate for
the actual hostname, its trust certificate on clients, and a reachable TLS bind.
Certificate verification cannot be disabled. Full configuration and recovery
details are in docs/multiplayer/accounts-and-administration.md in the source.

Build and source
----------------

Source: https://github.com/SK8-ENGINE/skate-3-rust-engine
See server-build.json for this build's revision, local modification flag,
and executable SHA256.
This package has its own checksum; it is separate from the game updater.
Compare (Get-FileHash .\skate-server-windows-x64.zip -Algorithm SHA256).Hash
with the adjacent .zip.sha256 before extracting a downloaded package.

To build from source on Windows, install Rust (stable MSVC x64), Visual
Studio C++ Build Tools with the Windows SDK and CMake, and Python 3.11 or newer.
From the repository root, run:

    .\scripts\Build-ServerRelease.ps1

The ZIP and .zip.sha256 are written to target\. Optional -TargetDirectory
and -OutputDirectory parameters select the Cargo cache and package output.
Relative parameter paths are resolved from the repository root.
This builds skate-server with its headless networking and Lua resource
dependencies; it does not build the graphical game or require Steam.

The project license is included in LICENSE. Lua dependency notices are in
LUA-NOTICES.txt; platform dependency notices are in PLATFORM-NOTICES.txt.
Bundled setup guides and current limits start at docs/multiplayer/README.md
and docs/multiplayer/platform-extension.md. SDK declarations and authoring
guides are in sdk/, including C# worker configuration in sdk/RESOURCES.md.
Source-code references and tools/ build or verification commands require the
source checkout. Local test log paths in the evidence ledger describe the
validation host, not ZIP contents.
Resource inventory is not anti-cheat attestation; the
challenge validates requests but still relies on owner-reported landings.

The additional local examples run with:
    .\skate-server.exe --test-world --max-players 64 --resources resources/platform-examples.json
These include JavaScript/Lua cross-language resources and transactional SQLite
inventory. SQLite and QuickJS are bundled; no Node.js or database service is
required. Server/client protocol versions must match after the movement-epoch
upgrade. Native Windows execution still needs the Windows CI/runtime checks.
