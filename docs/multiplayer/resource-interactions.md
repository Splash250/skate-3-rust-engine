# Resource interactions and RP phone

The supplied RP profile runs the interaction menu, phone, inventory, call signaling,
policy and administrator dashboard as ordinary versioned resources. HTML is rendered
by the isolated browser companion and composited into the game window. Theme and
app changes require resource replacement/restart, not an engine rebuild.

See [the evidence matrix](resource-interactions-evidence.md) for actual verification
and remaining native acceptance. This guide describes the implementation contract;
it is not a claim that every platform/device has passed acceptance.

## Build and local installation

Prepare the ordinary owned character/animation assets using [Linux setup](../LINUX.md)
or the Windows installer. The included Creator Courtyard is original redistributable
map content. No GTA assets, FiveM binaries, central service or paid service is used.

```sh
cargo build --locked -j1 -p skate-browser -p skate-game -p skate-server -p skate-accounts --features host --bin skate-browser-host --bin skate3rust --bin skate-server --bin skate-account
```

Keep `skate-browser-host` beside `skate3rust`. Linux needs GTK3/WebKitGTK4.1 development
packages for building and their runtime libraries plus an X11/XWayland session and
delegated memory/pids cgroup controllers. Windows needs WebView2. See
[browser interfaces](browser-interfaces.md) for the existing process isolation
prerequisites. The companion has no remotely fetched UI dependency.

Copy [rp-server.json](../../resources/rp-server.json) to a private installation,
set its `root` to the absolute repository `resources` directory and `storage` to an
external private data directory. Keep the configuration's path stable: persistence
is scoped to that path. The checked-in configuration can also run directly; its
`resource-data/rp` output is ignored by Git.

Client grants remain independent of operator server grants. Merge
[rp-client-grants.example.json](../../resources/rp-client-grants.example.json) into
`grants.json` under `SKATE3_RESOURCE_CACHE` (default: the prepared asset installation's
`settings/resources`). Replace the exact source key if address, port or session
changes. Grant `phone: engine.photos` and `interaction-policy: engine.input` only
for the reviewed server. `engine.photos` cannot supply arbitrary paths or request a
shutter. Other ordinary UI/audio/voice resource grants use existing safe defaults.
Do not replace unrelated local grants.

Choose a private directory you own and substitute it for `/private/skate-rp` below.
Create that parent directory before initializing the authority. Keep passwords on
standard input and private files outside the repository:

```sh
mkdir -p /private/skate-rp
target/debug/skate-account init /private/skate-rp/accounts administrator < /private/admin-password
```

Create the account/server/client configuration described in
[accounts and administration](accounts-and-administration.md), then start the
server and client in separate terminals:

```sh
target/debug/skate-server --test-world --bind 127.0.0.1:31030 --resources resources/rp-server.json --accounts /private/skate-rp/accounts.json
./PLAY.sh --assets /absolute/prepared/assets --test-world --connect 127.0.0.1:31030 --account-config /private/skate-rp/player.json --voice
```

On Linux, `PLAY.sh` sets the development Bevy/Rust shared-library search path.
On Windows, use `target/debug/skate3rust.exe` with the same arguments. For a custom
Cargo output directory on Linux, launch its executable with `LD_LIBRARY_PATH`
containing that directory's `debug/deps` and `rustc --print target-libdir` output;
`PLAY.sh` uses the ordinary repository target directory.

The browser process, account authority and server are local. Accounts are required
for inventory and administration. Anonymous dedicated sessions can still use the
phone/camera/call UI but have no verified inventory or administrative authority.
Offline/peer sessions keep their normal mods, replay and travel controls; these
resources are selected by the dedicated server only.

## Controls

| Context | Keyboard | Controller |
| --- | --- | --- |
| Master interaction menu | Hold F2 for 600 ms | Hold View / Back / Select for 600 ms |
| Phone shortcut | P | Choose Phone in master menu |
| Inventory shortcut | I | Choose Inventory |
| Interface navigation | Arrows / Tab, Enter | D-pad or left stick, A |
| Back within an interface | Focus on-screen Home/Back with Tab, Enter | B |
| Emergency close | Esc | Start |
| Call push-to-talk | Hold V | Hold LB while a call channel is selected |
| Native local mute / deafen | Ctrl+M / Ctrl+D | Voice app resource controls cannot clear native overrides |
| Camera framing | Arrow keys | Right stick |
| Camera zoom | Page Up / Page Down | Triggers |
| Photo shutter | F12 | X |
| Leave viewfinder | Esc | B |

Input is host-owned; scripts receive action callbacks, not competing controller
polls. Pause/disconnect/recovery remain accessible. Held controls must be released
when returning to gameplay. The View binding applies to dedicated resources;
offline View replay is preserved. Phone Settings provides local binding preferences
for registered actions. Conflicting/reserved bindings fail (including native F9/F10 profiling shortcuts);
defaults resolve in stable namespaced-ID order. Preferences live in the asset installation's
`settings/interface-bindings.json`, separate from server settings.

Voice remains opt-in through `--voice`; it is push-to-talk, not hands-free. Opening,
closing or changing focus releases transmission and requires a fresh physical hold.
An accepted call continues after the phone closes. The controller LB button is
reserved for voice while a channel is selected; other skating controls remain
available. Local mute/deafen always win over
resource requests; use the same native shortcut to clear a native override.
Device errors and voice-disabled peers are shown explicitly.
Calls are restricted to the same server instance but bypass distance. Only the two
accepted participants enter the server-owned channel. Decline, cancel, timeout,
disconnect, instance changes and owner retirement release it. Maximum 16 concurrent
calls, one call per participant; ringing and contact requests are rate-limited.

## Server policy and permissions

`interaction-policy` owns typed replicated settings. Both phone and master menu
call its `filter` export, so there is one configured ordering/access policy.

| Setting | Format / behavior |
| --- | --- |
| `enabled` | `*`, or comma-separated full interface IDs |
| `order` | Comma-separated IDs, followed by remaining entries in stable ID order |
| `labels` | Optional comma-separated `resource/key=Label` overrides |
| `categories` | Optional comma-separated `resource/key=Category` overrides |
| `access` | Optional comma-separated `resource/key=permission` requirements |
| `manual_markers` | Default false: suppress manual stock session-marker set/return/HUD |

These are bounded typed settings, editable through existing administration. Do not
put commas or equals signs in override values. Required descriptor permissions are
combined with configured requirements. Unknown/uninstalled entries are absent.
Custom descriptor/configuration permissions are checked against the permissions
of the account's inherited roles.
The policy queries up to 128 distinct names in stable order through the bounded
`permissions` action; installing a custom permission needs no engine rebuild.
Permission visibility expires after two seconds without fresh authority; private
handlers still recheck independently. The profile does not add a Spots app.
Operators may opt out by setting `manual_markers=true`; automatic bail recovery,
respawning, checkpoints and resource interaction markers are unchanged. Policy is
resource/session-owned and is removed on disconnect, failure or retirement.

Grant separate powers through the existing role inheritance system:
`status.read` opens diagnostics; `settings.read` reveals typed settings (including
private schemas); `settings.write` changes them; `resources.manage` controls
lifecycle; `profile.read` reads measured timings. `calls.diagnostics` reveals the separate
call-diagnostics plugin, and `calls.test` additionally permits its bounded live
call-state invariant check. A moderator may have only status
and profiling access. No single administrator boolean is used.

The dashboard sends a narrow reserved request through the existing resource lane:
`resource.send('__host_admin',{seq='1',action={kind='settings_read',resource='interaction-policy'}})`.
It requires declared/granted `resource.admin` plus `resource.network`. The server
intercepts the request before Lua, derives the actual sender, checks the live
verified session and action permission, and uses the existing audited HostAction
queue. Results return privately as `__host_admin_result` with `{seq,ok,value,error}`.
The current connection, generation and original permission are checked again before
delivery. Browser pages never receive passwords, bearer tokens or account keys.
No console command, evaluation or arbitrary HTTP/filesystem operation is exposed.
Interactive status/profile replies trim log/span history to a 14 KiB view and
report omitted counts. Typed private settings are never partially truncated.

Types, bounds, enum choices, live/restart semantics and pending values come from the
existing typed settings schema. A saved restart setting is shown as pending until
its resource restarts. Lifecycle actions can retire the currently open dashboard;
use the master binding again after its generation is ready.

## Apps, interfaces and themes

A client resource with `engine.ui` registers an interface:

```lua
sdk.ui.interfaces.register('open', {
  version=1, label='My interface', icon='map', category='Player',
  destination='dashboard', phone=true, quick=true, permissions={},
  disabled_reason=''
})
-- on_event receives {type='interface', key='open', generation='...'}.
-- Open your own packaged page here using sdk.ui.browser.open(...).
```

The host constructs `resource-id/open` and supplies owner/generation; scripts
cannot choose another owner. Descriptor limits: 32 per resource, 128 total, label
96 bytes, icon/category 32 bytes, eight permission names, disabled reason 256 bytes.
The registry has a 12 KiB serialized metadata budget including reserved local
binding space; the first exceeded limit rejects registration predictably.
Only descriptor version 1 is accepted. Duplicate registration fails; remove before
replacing. `sdk.ui.interfaces.remove(key)` removes only the caller's entry.
Retirement removes all registrations and callbacks for that generation.

`sdk.ui.interfaces.list()` delivers `on_event {type='interfaces',version=1,entries}`.
Consumers invoke `sdk.ui.interfaces.invoke(id,generation)`; stale generations fail.
The consumer must declare an exact dependency on the provider, or the provider on
the consumer. Dependencies confer availability, while manifests and local/server
grants still control APIs. Wrapped `sdk.commands.request` returns failures without
retiring the resource. JS/C# use the same validated `ui_interfaces` command schema.
Descriptor destinations are presentation metadata; invocation always calls the owner.
There is no generic executable callback string.

[park-guide](../../resources/park-guide/) is a separate working extension: it registers
People nearby, depends on `phone`, and shows the actual local session roster in its
own page. Include its ID in the master resource dependencies to surface it there.
Other apps follow this same pattern. Privileged providers must use a server-checked
operation; hiding an icon does not authorize anything.

[call-diagnostics](../../resources/call-diagnostics/) is the custom administration
example. Its packaged interface requests aggregate counts or an explicit invariant
check from its own server script, using the actual sender and
`resource.authorized(sender, permission)`. It neither grants channels nor captures
audio to perform a test. Registering another such handler/page needs no engine
rebuild or generic command execution.

The [phone README](../../resources/phone/README.md) documents theme tokens, DOM/message
contract and app integration. The shipped Twilight and Paper themes are local
preferences, with adjustable scale. Replace CSS/wallpaper/layout within the package,
keep the message contract, list all assets in the manifest, and restart the resource.
Original inline SVG icons and the bundled licensed font require no network access.

## Photographs and gallery

Open Camera from Phone. The visible scene is the viewfinder. The host copies its
3D camera into a 1920×1080 image target and uses asynchronous GPU screenshot
readback; phone/menu UI and unrelated desktop windows are excluded. Only a physical
X/F12 press while the focused owner's camera context is active can capture.

PNGs save automatically after that explicit shutter to the OS-resolved Desktop /
Skate Photos directory. Linux uses XDG user directories; Windows uses Known Folder
resolution. If no Desktop is configured, or the directory is unwritable/symlinked,
the feature reports an actionable error; it does not silently guess `~/Desktop`.
Configure a real desktop directory through your OS, then reopen Camera. Filenames
are host-generated and opened exclusively, so existing photos are never overwritten.
No remote peer receives local paths or photos; there is no upload facility.

There is one pending GPU/readback/save job globally, a one-second shutter interval,
and at most 32 gallery records per owner / 64 overall. Thumbnails are 160×90 JPEGs
capped at 8 KiB each, retrieved individually by opaque photo ID. The gallery covers
this generation's managed captures only, not arbitrary desktop files. PNGs remain
on disk across resource restarts; in-memory gallery entries retire with their owner.
This deliberately avoids scanning the desktop. Closing the viewfinder restores
normal framing while an explicitly requested save finishes. Retirement cancels pending work and releases cameras, targets and
thumbnails; a canceled writer retains its single job slot until it finishes.

## Verification

Run focused runtime/host tests before native acceptance:

```sh
cargo test --locked -p skate-mods --lib interactions::tests
cargo test --locked -p skate-mods --test interactions --test phone_calls --test phone_ui --test call_diagnostics --test resource_authorization
cargo test --locked -p skate-platform photos::tests
cargo test --locked -p skate-game --bin skate3rust modding::photos::tests
cargo test --locked -p skate-server --test phone_calls --test resource_admin
```

The native harness creates two verified local accounts, private stores and an XDG
Desktop under a new output directory. It launches the actual game clients and
browser companions, then exercises UI using injected X11 keyboard/pointer events
and ALSA test tones. It requires Python Pillow, X11/XTest, working Vulkan and the
same browser containment prerequisites; it does not establish physical controller
or microphone acceptance:

```sh
python3 tools/verify_resource_interactions.py --assets /absolute/prepared/assets --output /tmp/skate-rp-acceptance
```

For a custom Cargo target, add `--bin-dir /absolute/target/debug`. Run inside your
graphical session, or set `DISPLAY` to an already running X11 test display. The
harness uses a new disposable Desktop and private account stores under `--output`;
it does not change the normal user's Desktop configuration.

The output directory must be absent or empty. Keep its credentials, databases and
captures out of Git. Native two-client and browser tests require graphical/process-isolation prerequisites;
see [the evidence ledger](resource-interactions-evidence.md) for exact fresh commands,
images, audio evidence and remaining gaps. Synthetic native input/audio and Linux
cross-compilation are reported separately from physical devices and native Windows.
