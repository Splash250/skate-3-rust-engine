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

The server reads the map to fingerprint it. It does not load map geometry,
character assets, Bevy, a display, Steam, or a running game. Rust and Python
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

The default is IPv4 UDP port 31030, up to 16 players, session 48031030.
Allow the selected UDP port through the host firewall and forward it on
your router for Internet hosting. Resource hosting additionally uses TCP
on the same port for bounded HTTP content downloads. Use
--bind 127.0.0.1:31030 for local-only testing. Enter quit for resource cleanup
and graceful shutdown. Restart the server and clients to change maps.

For a custom session, pass --session 12345 on the server and
--net-session 12345 on every client. Session numbers are not passwords.
No account authentication, encryption, or matchmaking is provided. Only
the configured resources' declared client/shared scripts and files are
downloaded; private server scripts/configuration are excluded. Resources
use sandboxed Lua and capability grants. Legacy local mods are skipped in
dedicated sessions. Do not combine --connect with --net-host or --net-local.

The owning client simulates detailed skating, articulated bodies, and map
collisions. The server validates membership and compatible data, routes
movement/pose/trick state, and authorizes coarse player collisions and
shoves. Scores are presentation data, not a trusted leaderboard.

Build and source
----------------

Source: https://github.com/SK8-ENGINE/skate-3-rust-engine
See server-build.json for this build's revision, local modification flag,
and executable SHA256.
This package has its own checksum; it is separate from the game updater.
Compare (Get-FileHash .\skate-server-windows-x64.zip -Algorithm SHA256).Hash
with the adjacent .zip.sha256 before extracting a downloaded package.

To build from source on Windows, install Rust (stable MSVC x64), Visual
Studio C++ Build Tools with the Windows SDK, and Python 3.11 or newer.
From the repository root, run:

    .\scripts\Build-ServerRelease.ps1

The ZIP and .zip.sha256 are written to target\. Optional -TargetDirectory
and -OutputDirectory parameters select the Cargo cache and package output.
Relative parameter paths are resolved from the repository root.
This builds skate-server with its headless networking and Lua resource
dependencies; it does not build the graphical game or require Steam.

The project license is included in LICENSE. Lua dependency notices are in
LUA-NOTICES.txt. Full multiplayer and resource instructions and current
limits are in docs/multiplayer/README.md and docs/multiplayer/resources.md
in the source tree. Resource inventory is not anti-cheat attestation; the
challenge validates requests but still relies on owner-reported landings.
