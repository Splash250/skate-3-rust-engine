# Resource browser interfaces

Resources open packaged HTML/CSS/JavaScript through the isolated
`skate-browser-host` executable. Set `surface` to composite the actual rendered
page into the game window. Linux uses an offscreen WebKitGTK 4.1 view; Windows
uses WebView2 preview capture. Omitting `surface` retains the companion-window
API for existing resources. Existing canvas and menu APIs remain available.

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
    width=900, height=640, focus=true,
    surface={anchor="center",scale=1,offset={24,24},fps=20}
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
close it before reopening. Sending or changing focus after a page has closed is
an inert operation: host Escape can retire a page before an already queued VM
update is applied. These late updates never reopen a page or take input focus.
Messages to an existing page that is not ready and live renderer errors remain
errors. Resource scripts should still correlate asynchronous replies with their
current page/session request IDs when reusing a key. Only one page may request input focus. For composited surfaces, Escape or
controller Start closes the page in the host, even when page JavaScript is stuck.
Controller B sends an Escape key event to the page for internal back navigation.
Pages should also provide a visible close button. In companion-window mode,
Escape behavior belongs to the page.

A focused page suppresses skating input. A composited page stays inside the game
window and receives host-forwarded pointer, keyboard and controller input. Closing or releasing focus consumes keyboard/mouse edges while preserving actual
physical held state; the host gates gameplay and push-to-talk until release. Resource stop, failure, restart,
server disconnect and application exit tear down its pages and subprocesses.
A stopped renderer fails its owning resource instead of blocking the game loop.
A host-owned notice remains visible for ten seconds with the resource name and
bounded failure reason; rendering that notice does not depend on the failed page.

## Composited surface contract

`surface` is optional. Its validated fields are:

| Field | Values / default |
| --- | --- |
| `anchor` | `"bottom_right"` (default), `"center"` |
| `scale` | 0.5–2; default 1 |
| `offset` | Two nonnegative logical pixel values, each at most 256; default 24, 24 |
| `fps` | 1–30; default 20 |

Composited dimensions are 320–1280 by 240–960. The browser's layout size is fixed
until reopened; the game scales it down to fit a resized game window. Page CSS
can use media queries against the chosen layout size. Transparent HTML/CSS areas
preserve the visible game behind the surface. A phone can use 400 × 800 with
`anchor="bottom_right"`; a dashboard can use 1100 × 800 with `anchor="center"`.

`focus(false)` hides the surface and stops frame requests without losing its
resource state. `focus(true)` shows it again. Pause and the host camera viewfinder
hide the surface. Closing removes its entity and image asset and terminates its
owned process tree; no retained browser image remains after resource retirement.

Tab/Shift-Tab and directional controller navigation focus enabled visible HTML
controls. A/Enter activates buttons once per physical press; held Enter repeats
do not activate a new route or dialog. Focus changes require held buttons and
keys to be released before the new page accepts them. B delivers a cancellable Escape keydown to
the page. The fixed host adapter supports text inputs, textareas, checkboxes and
selects (arrow keys or controller left/right change a select). Numeric fields
support bounded text editing and controller step controls. Ctrl+M/Ctrl+D retain
their trusted local voice mute/deafen behavior while a page is open. A custom widget should use `tabindex`,
button semantics and cancellable `keydown` handlers. Pointer coordinates are
mapped from the game viewport into the page; clicks and scrolls stay in that
surface. These DOM events are synthetic browser events: they are not microphone
consent, native clipboard access, browser permission grants or trusted keyboard
input for engine actions. Physical voice push-to-talk remains an engine concern.

The companion accepts one snapshot at a time, including the write to its private
pipe. Binary RGBA bytes follow a bounded frame header; dimensions must match the
opened page before the game worker allocates or reads the body. A worker retains
only the latest frame, never a queue of frames. The maximum frame is 4,915,200
bytes (4.69 MiB). The game image and temporary snapshot/transfer buffers each have
that fixed upper bound. JSON and input queues remain separately bounded at 64;
only the currently focused surface requests frames. The existing four-page and
per-process-tree limits still apply.

## Native surface verification

The `composited_webview_renders_pixels_and_receives_host_input` test runs the real
web engine, verifies visible button pixels and transparent background, delivers
pointer input, navigates to a text field, checks a JavaScript reply, cycles focus,
and checks process teardown. `SKATE_BROWSER_TEST_CAPTURE=/absolute/local.png`
optionally saves its synthetic rendered fixture for inspection. Captures belong
outside the repository.

The shipped phone tests also render both themes, exercise actual app routes, and
retain unsaved binding drafts and gallery nodes during voice telemetry updates.
The shipped administration test revokes settings access before a delayed private
reply and checks that the private content stays absent.

Linux offscreen rendering and DOM input have passed locally. Windows snapshot
code has passed a Windows-target compile check; native Windows surface capture,
focus and WebView2 runtime behavior require a Windows graphical acceptance run.
The host test establishes actual HTML rendering; two-client game integration is
a separate acceptance scenario tracked in the interaction implementation ledger.

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
