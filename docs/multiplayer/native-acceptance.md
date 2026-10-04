# Native platform and physical audio acceptance

This is the continuation procedure for acceptance that cannot be inferred from
Linux tests, source inspection, cross-compilation or a CI definition. Record
the exact revision, OS/runtime versions, commands, exit codes and logs for each
run. Keep private assets, passwords, account stores and audio out of Git and
packages. No external audio service is needed.

## Current prerequisites

The continuation session has a Linux graphical host and prepared owned assets.
It has no configured Windows remote/desktop. Its running QEMU guest is Docker
Desktop LinuxKit. The kernel lists a Plantronics Blackwire 3220 USB headset, but
this session cannot open `/dev/snd/controlC2`, `pcmC2D0c` or `pcmC2D0p` (EACCES).
The device ACL belongs to the login greeter; the task user has no device grant.
PipeWire shows Dummy Output only. The operator is away from the machine.
No device permissions were changed and no microphone audio was captured.

To continue, provide an interactive Windows desktop reachable through an
authorized local/remote session, and an active desktop session with authorized
access to physical capture/playback devices. A person at the devices must
confirm microphone/speaker acoustics; opening streams alone is insufficient.
Record device IDs again after login/reconnection because they may change.

The broader mod integration suite also requires the private owned
`sdk/examples/skyline/skyline.glb` fixture. It is absent on this host. Supply the
correct owned map before running that suite; do not substitute another map,
remove assertions or place the private fixture in commits/packages.

## Windows execution

Use a clean checkout of the exact local commit and matching game/server binaries.
Install the repository's MSVC/Windows SDK build prerequisites, CMake, Python,
WebView2 Runtime, and the trusted .NET 10 runtime/worker. Building the worker
requires the .NET 10 SDK; the worker uses SDK Roslyn assemblies and no external
NuGet service. Prepare the user's own character/animation assets separately.

Build and run in an interactive PowerShell session, preserving command output:

```powershell
cargo build --locked -p skate-game --bin skate3rust
cargo build --locked -p skate-server -p skate-accounts
cargo build --locked -p skate-browser --features host
dotnet publish crates/skate-mods/managed-host/Skate.ResourceHost.csproj -c Release -o C:/skate-private/managed-host
$env:SKATE_DOTNET_ROOT = 'C:/skate-private/dotnet'
$env:SKATE_MANAGED_HOST = 'C:/skate-private/managed-host'
cargo test --locked -p skate-browser --features host --test host -- --ignored --test-threads=1
cargo test --locked -p skate-mods --test managed_runtime -- --ignored --test-threads=1
cargo test --locked -p skate-voice --features devices
cargo test --locked -p skate-accounts
cargo test --locked -p skate-server --test accounts --test voice
cargo test --locked -p skate-services
python -m unittest tools.test_package_server -v
```

The runtime directory must contain `dotnet.exe` and the required runtime, and
the worker directory must contain the published trusted assembly/dependencies.
Use private application directories that the worker's ACL setup may grant read
and execute access to. The account implementation requires built-in PowerShell
for current-user ACL enforcement. Follow [account setup](accounts-and-administration.md)
for fresh private credentials; never copy a session key from Linux.

| Native acceptance | Required observation beyond build success |
| --- | --- |
| WebView2 | Downloaded inventory page exchanges real IPC; remote network/navigation/device requests fail; focus returns to game; stop, failure, restart and disconnect close every host/renderer process. Verify the Job Object memory test and keyboard/mouse behavior. |
| Managed worker | Real compiled resource executes and cross-language calls pass; forbidden APIs and runaway code fail; AppContainer has no network/ambient filesystem access; restart/disconnect kills worker; temporary profiles/ACL grants retire. |
| Accounts/admin | Real TLS login and AEAD UDP; wrong identity/replay/CA fail; kick and revocation affect live player; permissions update; private ACLs persist; restart preserves roles/data and expires sessions. |
| Persistence | Transaction rollback/reopen, abrupt-process test, stopped-store backup/restore, file locks and paths behave on NTFS. Process kill does not establish power-loss safety. |
| Packaged runtime | Build the package through the existing packaging workflow, extract to a fresh directory, run packaged executables and examples with required companions, .NET/WebView2 prerequisites and notices. An installed developer SDK must not mask a missing packaged runtime. |

## Mixed Windows/Linux session

Run two actual clients, one on each OS, against the same dedicated server and
exact resource set. Record content digests and admission stages. Clear only a
new test cache for a cold download; reconnect a new process for warm cache and
verify zero unchanged downloads. Exercise changed/corrupt content separately.

Both clients must observe/contact the same moving object, and a late join must
receive current state. Move one player into a private instance and verify no
peer, shared collider, scoped state or voice crosses the boundary; return and
verify names/replicas recover. Stop/restart resources, transition park→base→park,
and observe imported presentation and stock restoration on both renderers.
Test browser and C# teardown during those transitions, then disconnect/reconnect.
Use the existing native verification tools where their platform prerequisites
are met, retaining screenshots for visual review. A pair of Linux clients or
simulated protocol endpoints does not close this matrix.

## Physical audio procedure

Launch both clients with `--voice` on the private test server. Prefer separate
headsets/computers to avoid acoustic feedback. Use fresh device enumeration and
explicit input/output selection through the [voice API](voice.md). Keep speech
local to the clients/server; do not upload recordings. Log device/routing state
and a short operator result, not microphone samples.

1. Hold physical **V**, speak on each client in turn, and confirm intelligible
   speech at the other endpoint. Release V, lose focus, open the game menu and
   focus browser UI; confirm transmission stops each time.
2. Check proximity inside/outside its configured radius. Join the same resource
   radio channel and verify out-of-range speech, then remove either membership.
3. Test local mute/deafen, server mute, and switching between valid device IDs.
   Disconnect a device and confirm a bounded error followed by successful reselection.
4. Split instances while speaking; no old buffered speech may cross. Return and
   verify the intended policy resumes. Stop/restart the radio resource and
   verify memberships/overrides retire and defaults restore.
5. Disconnect, reconnect and exit both clients. Confirm streams close, device
   handles/processes return to baseline, and no audio worker remains accumulating
   after repeated resource restarts.

Actual Opus, authenticated UDP and ALSA synthetic-stream tests establish their
respective software paths. They remain separate from these physical observations.
