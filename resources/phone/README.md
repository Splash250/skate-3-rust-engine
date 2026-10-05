# Skate Connect phone

A packaged HTML/CSS/JavaScript phone composited inside the game window. Its Lua
resource opens a bounded browser surface; presentation and app configuration
are replaceable without rebuilding the engine. Use the complete
[RP server configuration](../rp-server.json) and the installation instructions
in [RP interactions](../../docs/multiplayer/resource-interactions.md).

Open the master menu by holding **F2** or **View/Back/Select** for 600 ms, then
choose Phone; **P** opens Phone directly. D-pad/arrows move focus, **A/Enter**
chooses, **B** goes back, and another Back from Home closes. **Escape/Start**
closes the interface immediately; keyboard users can also focus the on-screen
Home/Back controls with Tab. Settings
includes a local shortcut editor for keyboard/controller buttons and hold
duration, with conflict detection and actual save results. The bottom home
indicator returns Home. Calls continue after closing. If the presentation
resource fails or retires, the call backend expires its three-second presentation
lease and ends the call. **V/LB** is physical push-to-talk;
**Ctrl+M/Ctrl+D** toggle local mute/deafen.

Home provides Phone/Contacts, Camera, Gallery and Settings. Installed apps come
from the same versioned interface registry as the master menu, ordered and
filtered by the shared `interaction-policy` resource. The Voice entry opens
actual device discovery, microphone/playback selection and phone mute controls.
Native local mute still takes precedence, so the interface explains how to
clear a keyboard mute. Device errors and disabled `--voice` are visible.
There are no fake contacts, balances, notifications, signal or battery levels.
The clock uses local wall time.

Camera freezes the current gameplay viewpoint and gives physical controls to
the host. Aim with right stick/arrows, zoom with triggers/PageUp/PageDown,
photograph with **X/F12**, and return to Gallery with **B/Escape**. The scene
capture excludes phone/menu UI. Photographs save asynchronously as PNGs in
OS-resolved Desktop / Skate Photos. They stay local. Gallery requests only
owned opaque photo IDs, displays eight thumbnails per page, and keeps at most
eight thumbnail strings resident. It cannot read arbitrary desktop files.
Photos remain in the desktop folder after the bounded session gallery retires.
See the photo host documentation for native export limits and errors.

## Themes and layouts

`phone.css` defines the version 1 visual contract. The `twilight` and `paper`
themes replace `--ink`, `--muted`, `--surface`, `--panel`, `--line`, `--accent`,
`--accent-ink`, `--danger`, `--positive` and `--wallpaper` tokens. Typography uses
the bundled Ubuntu Sans font under its included license; icons are original
24 × 24 SVG paths in `phone.js`, with a consistent 1.7-unit stroke.
`ring.wav` is an original generated 659.25/987.77 Hz two-second chime. The
outer viewport is 390 × 760 with bottom-right anchoring and 20 Hz composition;
local sizes are 75%, 85% and 100%. Themes and scale persist in the existing
source/resource/client storage namespace. No local paths are sent to the server.

Edit CSS for appearance and HTML/JS for layout; retain the Lua bridge action
contract (`route`, `call`, `refresh_contacts`, `preferences`, `binding`,
`invoke`, `thumbnail`, `close`).
All dynamic text is inserted with `textContent`; pages never evaluate plugin
strings or load remote content. Open/close animations honor reduced motion.
Any custom page must preserve visible focus, back/close access and native camera
input ownership. Browser failure retires the resource and returns input safely.

## Add an app from another resource

Declare `"phone":"1.0.0"` in your resource dependencies and request/grant
`engine.ui`. Your resource registers its own entry and handles invocation:

```lua
return {
  on_load=function()
    sdk.ui.interfaces.register("guide", {
      version=1, label="Park Guide", icon="map", category="Player",
      destination="dashboard", phone=true, quick=true
    })
  end,
  on_event=function(event)
    if event.type=="interface" and event.key=="guide" then
      sdk.ui.browser.open("guide", {
        entry="index.html", files={"index.html","guide.css","guide.js"},
        width=900,height=640,focus=true,
        surface={anchor="center",scale=1,offset={0,0},fps=20}
      })
    end
  end
}
```

List packaged files in `resource.json`. See the working
[`park-guide`](../park-guide/) resource for a complete extension. For a return
button, close your dashboard, then invoke `resource.call("phone","open",{})`
with `resource.exports` granted. The exact dependency permits cross-resource
invocation and safely restarts dependents when the phone updates. Every entry
carries a host-owned generation; restart removes old entries and surfaces.

Use `permissions` for requested visibility and validate every privileged action
on the authenticated server independently. The phone calls the same
`interaction-policy/filter` export as the master menu; a custom app must not
trust an icon's visibility as authority. Never put private settings or account
credentials in page assets or replicated values.
