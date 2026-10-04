# Resource browser interfaces

Resources can open a real local HTML/CSS/JavaScript page through the separate
`skate-browser-host` executable. Existing canvas and menu APIs remain available.
The browser is a companion window; it is not currently composited into the 3D
swapchain. Linux uses WebKitGTK 4.1 and Windows uses WebView2.

## Build and prerequisites

```sh
# Ubuntu/Debian build prerequisites, installed by the operator:
# libwebkit2gtk-4.1-dev libgtk-3-dev
cargo build --locked -p skate-browser --features host
cargo build --locked -p skate-game --bin skate3rust
```

Keep `skate-browser-host` (`.exe` on Windows) beside the game executable. Linux
needs a graphical X11/XWayland session and a writable delegated cgroup v2 with
memory and pids controllers. The companion places its entire process tree in its
own cgroup before reading the page: 1 GiB memory, no swap and 128 tasks. It fails
with an actionable error if delegation is unavailable; it does not modify parent
controllers. Typical systemd user sessions provide this delegation. Windows uses
a JobObject with a 1 GiB tree budget and kill-on-close. Windows also requires the
Microsoft WebView2 runtime. Native Windows browser execution remains unverified
in the current local environment; consult the evidence ledger before release.

## Resource API

Request and grant `engine.ui`. Declare every web file in the resource manifest's
`files` list. The host serves only the explicit subset supplied to `open`:

```lua
sdk.ui.browser.open("inventory", {
    entry="index.html", files={"index.html","inventory.css","inventory.js"},
    width=900, height=640, focus=true
})
-- After the ready event:
sdk.ui.browser.send("inventory", {coins=75})
sdk.ui.browser.focus("inventory", false)
sdk.ui.browser.close("inventory")
```

The resource's `on_event` lifecycle callback receives
`{type="browser",key="inventory",event={kind="ready"}}`, `message` (with `value`),
`focus` (with `focused`) and `closed`. Pages call
`window.skate.postMessage(jsonValue)` and listen to the standard `message` event
for script replies. JavaScript and C# resources use the same typed command
interface. Wait for readiness before sending. Opening an existing key fails;
close it before reopening. Only one page may request input focus. Escape behavior
belongs to the page; the shipped inventory implements Escape and a close button.

A focused page suppresses skating input. Closing or releasing focus clears held
keyboard/mouse state before returning control. Resource stop, failure, restart,
server disconnect and application exit tear down its pages and subprocesses.
A stopped renderer fails its owning resource instead of blocking the game loop.

## Permission and resource boundary

- Navigation is restricted to `skate://ui/` packaged files. Remote navigation,
  child windows, downloads, clipboard, drag/drop and browser device permissions
  are denied. A content security policy disables network connections, frames,
  workers, objects, remote code, forms and base URL changes. Remote networking
  requires the separate server backend API, never page credentials.
- The page has a JSON message bridge only. It cannot call engine APIs, read local
  files or choose another resource owner. Engine actions are issued by the
  owning resource VM and still require its grants. All resource languages share
  this boundary.
- Allowlisted HTML/CSS/JS/images/fonts are loaded as immutable bytes. Symlinks,
  traversal, query/encoded paths, unknown file types and unlisted paths fail.
  Bounds are 512 files, 8 MiB each, 32 MiB total; at most two pages per resource
  and four per game. JSON messages are at most 16 KiB and queues hold at most 64.
- Process creation and initial local asset reads happen when opening a page.
  Steady-state IPC uses bounded worker queues. Startup has a 20-second deadline;
  unresponsive JavaScript has a five-second watchdog. The OS process tree budget
  also covers renderer allocation attacks. Web engine vulnerabilities remain an
  upstream patching responsibility; the custom bridge is not a native-code API.

## Example and verification

[`resources/inventory-ui`](../../resources/inventory-ui/README.md) combines a
packaged page, script messages, verified account identity and transactional
persistence. Its UI has pending/error states, keyboard controls and a narrow
window layout. It never accepts an account ID from a page or client payload.

Actual desktop tests (run explicitly; requires the prerequisites above):

```sh
cargo test --locked -p skate-browser --features host --test host -- --ignored --test-threads=1
```

These test the web engine itself, including network denial, JSON, focus,
process cleanup, runaway JavaScript and bounded memory exhaustion. The
[evidence ledger](platform-extension-evidence.md) distinguishes tests actually
run from game activation, native Windows and platform checks still outstanding.
