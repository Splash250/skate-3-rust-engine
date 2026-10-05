# Resource interactions implementation evidence

Baseline verified: clean `cross-platform-dedicated-server` at
`ffe3374166cfbc64c79b14690583fd780d2b7242` on 2026-10-05. No prior phone implementation
was present. This work extends the existing resource, accounts, settings, browser,
voice and capture systems. Logs, disposable stores, credentials and captures stay
outside Git under `/tmp/skate-phone-20261005/`.

## Implementation and acceptance matrix

| Capability | Implemented | Fresh verification | Incomplete / blocked prerequisite |
| --- | --- | --- | --- |
| Generic registry and input ownership | Versioned descriptors, dependencies, quotas, generations, local remapping, conflict/release gates | Game input/context/controller regressions pass; final native directional navigation and restart cleanup pass | Physical controller acceptance blocked below |
| In-game HTML surface | Isolated WebKit/WebView2 frames composited into owned Bevy images; bounded input and frame bridge | Actual two-game-client Linux composition and interaction pass; five validators and eight real WebKit tests pass | Native Windows blocked below |
| Master menu, phone, themes and apps | Typed shared policy, original phone, Twilight/Paper themes, separate People nearby app | Actual master navigation, both phones, centered SVG icon clicks and ordinary/admin filtering pass; resource startup and phone lifecycle tests pass | None in the tested Linux flow |
| Inventory and administration | Existing inventory persistence; audited typed settings/status/lifecycle/profile APIs | Native purchase persisted (`coins=75`, `deck_blue=1`); private forged request denied; administrator saved `ring_seconds=24`; server unit and authenticated TLS integration pass | Native role revocation is covered by separate automated account/renderer tests, not a live human role-change session |
| Real calls and voice | Authoritative signaling, accepted-only two-person channel, rate/instance/lifecycle controls | Two actual clients transmit encoded audio; accepted call survives phone close; three-peer encrypted UDP test rejects third-party/preaccept/post-hangup audio | Physical microphone/speaker acceptance blocked below |
| Photo mode, gallery and export | Trusted shutter, scene-only 1920×1080 PNG, OS Desktop resolution, bounded owned gallery | Actual saved photograph decoded and visually inspected; both players visible, UI absent; gallery and three repeated camera cycles pass | Native Windows Known Folder behavior requires Windows acceptance |
| Custom authorized plugin actions | Generic live-permission API and separate call diagnostics resource | Lua/JS, actual isolated C# worker and live account adapter tests pass; native administrator runs the bounded diagnostics test | No unrestricted evaluation or command execution API is provided |
| RP manual marker policy | Resource-owned suppression of manual set/return/HUD; automatic recovery retained | Union/retirement regression passes; native RP view has no manual-marker HUD; existing scene interaction marker remains | Human offline controller play remains part of physical acceptance |
| Native integration and cleanup | Reproducible private two-client harness | Final 17 phases pass with current binaries; active-call/camera restart clears channels/screens; killed renderer reports failure and core pause remains usable; all 16 browser hosts exit | Short repeated cycles are not a long soak |
| Physical controller | Selected raw-controller sample, navigation, trusted PTT/shutter and release gates | Synthetic raw-pad and DOM navigation tests; injected native keyboard/pointer execution | Every evdev event node and `/dev/uinput` is inaccessible. Readable `js0` identifies `mouce-library-fake-mouse`, not a verified gamepad. Provide a usable gilrs/evdev gamepad or permitted uinput device |
| Physical audio | Existing opt-in CPAL/Opus path and physical PTT preserved | Native deterministic ALSA capture and null playback; physical PCM access freshly checked and denied | Provide accessible physical capture/playback devices and perform listening/microphone acceptance |
| Native Windows | WebView2 surface and Known Folder path implemented | Browser MSVC cross-check passed during implementation | A native Windows machine, matching binaries and owned assets are required; cross-compilation is insufficient |

## Final native run

`native-verified/results.json` reports `ok:true`, 17 phases and no diagnostic semantic
workarounds. The harness copies packaged resources only to add read-only layout and
event observations and an explicit forged-request test. All ordinary actions use
injected X11 keyboard/pointer events in the actual game windows. Snapshot telemetry
observes the phone's existing call; it does not add a presenter heartbeat. Earlier
`native-*-probe` runs are diagnostic evidence only and are not the final acceptance.
Pointer movement follows an intermediate-to-target native path and waits for the
page to observe the target before a single click. This avoids an independently
confirmed X11/winit absolute-warp deduplication edge on returning focus; it does not
invoke page actions directly or change application behavior.

The exact final invocation was:

```sh
DISPLAY=:96 python3 tools/verify_resource_interactions.py --assets /run/media/csanad/Linux-Big/skate/skatemp/skate-3-rust-engine/target/debug/data/installations/63c576c7def843f29f48845d03c7ad3b/assets --bin-dir /tmp/skate-phone-20261005/target/debug --output /tmp/skate-phone-20261005/native-verified > /tmp/skate-phone-20261005/native-verified-run.log 2>&1
```

Both actual Linux clients loaded the original Creator Courtyard on the RTX 4080
Vulkan renderer. Down/Up/Enter opened Phone through the master menu; both phones
were composited in the game window. Native Ctrl+M/Ctrl+D state changes passed.
After accepting the call and closing the caller's phone, trusted V input exercised
actual CPAL capture, Opus encoding, server routing, decoding and output accounting:
**39 / 40 captured frames**, **43,020 / 47,275 audible samples** added. These are
deterministic ALSA tones and null playback, not physical acoustic evidence.

The independent three-peer TLS/encrypted-UDP/Opus test decoded **12 frames**, energy
**308.17133**, with **zero third-party frames**, and verified no membership before
acceptance or after hang-up/resource retirement. See `calls-udp-final.log` and
`calls-udp-current-resource-smoke.log`.

Final visual evidence, inspected by the implementing agent and the primary agent:

- `native-verified/phone-home-client0.png`: actual bottom-right phone with game visible.
- `native-verified/administrator-saved-setting-client0.png`: actual permitted settings UI.
- `native-verified/desktop0/Skate Photos/Skate-1791176400807219044-3603928.png`:
  decoded **1920×1080 PNG**, both players visible, no phone/menu/HUD. The green
  scene interaction marker is intended game content, not the suppressed stock
  manual marker HUD.

For this disposable run, the OS XDG Desktop pointed at `native-verified/desktop0`.
Normal use resolves the user's configured OS Desktop; no arbitrary path crosses
the resource API. Photographs and screenshots are not committed.

The native run also verifies ordinary-user menu filtering and direct private-read
denial, existing inventory purchase persistence, an actual administrator setting
save to the existing namespaced storage, the custom authorized invariant check,
call resource restart while an accepted call and viewfinder are active, both
clients' channel/screen cleanup, and three further phone/camera/gallery cycles.
The final deliberate renderer SIGKILL produces the engine-owned failure notice,
releases input and leaves native Escape/core pause usable. The required pack is
retired and the client returns offline, as expected for this terminal fault test.
All **16 browser host PIDs exited**. Peak observed browser process-tree memory
across phase samples was **87,093,248 bytes (83.1 MiB)**, below the existing 1 GiB
containment cap. This is a sampled observation, not a long-duration memory guarantee.

## Automated checks and build evidence

| Check | Result | Local log |
| --- | --- | --- |
| Final Linux game/server/accounts/browser build, full package feature union, `--locked -j1` | Passed after lifecycle fixes, 4m39s | `browser-lifecycle-green-build.log` |
| Game interactions / browser input / photos / voice input / controllers / debug camera | 30 distinct tests passed across focused runs: 5 + 6 + 8 + 1 + 8 + 2 | `recovery-game-tests.log`, `browser-lifecycle-green-build.log` |
| Browser IPC, asset, surface, input and frame validators | 5 passed in freshly Cargo-built test executable | `recovery-browser-validator-tests.log` |
| Real WebKit host suite, including ignored native cases | 8 passed, zero ignored: real pixels/input/SVG clicks, both themes, draft/gallery stability, revocation, runaway, memory containment, killed-host versus graceful-close classification | `browser-lifecycle-green-build.log` |
| Server administration unit checks | 5 passed, including actor forgery, bounded operations and oversized private replies | `recovery-admin-tests.log` |
| Real authenticated resource administration TLS integration | 1 passed, including private typed settings and actual actor | `recovery-admin-tests.log` |
| Actual full registry descriptors through Lua shared policy | 3 Cargo tests passed: actual full descriptors, policy expiry/startup and dropped-request/late-reply behavior | `interactions-native-descriptor-red.log`, `recovery-interactions-tests.log` |
| Current Lua/JS call, phone, custom dashboard and authorization test sources | 13 direct tests passed; the three policy/startup/dashboard cases also passed the final Cargo run above | `phone_calls-direct.log`, `phone_ui-direct.log`, `call_diagnostics-direct.log`, `resource_authorization-direct.log` |
| Registry ownership/limits/reserved binding tests | 4 direct tests passed | `registry-unit-direct.log` |
| Platform photo filesystem and live account authorization adapter | 4 + 1 direct tests passed | `direct-photo-authorization-tests.log` |
| Actual isolated .NET 10 authorization worker | 1 passed | `direct-managed-authorization-final.log` |
| Existing audited AdminBridge | 3 passed | `admin-bridge-tests.log` |
| RP configuration, assets, versions and grants | Full nine-resource closure validated; final CLI validator passed, 1,321,843 bytes | `rp-config-closeout-validation.log` |
| Source hygiene | Seven changed JavaScript files pass syntax checks; all 15 new Rust files pass rustfmt; diff whitespace and private/generated-path checks pass | `new-rust-format-check.log` |

The workspace-wide `cargo fmt --all -- --check` is not green: existing formatting
differences span unrelated crates. The same failure was reproduced on unchanged
baseline copies of `crates/skate-game/build.rs` and `crates/skate-accounts/src/admin.rs`.
No unrelated formatting rewrite was applied.

Final AST-only `graphify update .` completed successfully: 27,279 nodes, 59,273
edges and 1,311 communities (`graphify-closeout.log`). Its generated files remain
ignored. It reported a nonfatal installed-skill version mismatch, 37 inputs with
no AST nodes, and fallback community labels; no semantic/API extraction was run.
All 56 real local links in changed Markdown resolve, and the final native harness
passes Python compilation. All owned test processes and the test X server exited;
private evidence remains outside Git.

The original Cargo target stalled in an NTFS3 artifact-cleanup operation.
Superseded jobs were terminated cleanly without deleting the original cache or
forcing filesystem cleanup. A matching dependency closure was copied to an
independent `/tmp/skate-phone-20261005/target` (initially 11.18 GB, no symlinks).
Final Cargo jobs use that target, `CARGO_INCREMENTAL=0`, and serialize with
`flock /tmp/skate-phone-cargo-recovery.lock`. Some test feature combinations require
separate dependencies. Direct `rustc --test` runs compile the actual current test
source against matching built libraries; they are identified separately from
Cargo suite completion and never counted as native device acceptance.

## Independent review and fixes

Independent source and security-boundary reviews covered ownership/generation,
authenticated sender and live reauthorization, private response delivery, browser
isolation, path confinement and bounded input/messages. Reproduced findings were
fixed and reviewed again. Both final lifecycle regressions first failed, then
passed after generic fixes: queued presentation work after native close is inert,
and unexpected renderer exit cannot masquerade as graceful closure. The final
17-phase native run verifies both lifecycle fixes. No actionable source finding remains from
these passes; this is not a claim that every possible vulnerability is absent.

Corrections include held PTT/Enter/pointer release across focus changes; selected
controller sampling; acyclic pre-menu input scheduling; pause/debug-camera gating;
stable settings drafts/gallery DOM; bounded metadata/device/status exports;
permission-query and optional descriptor serialization through Lua; dropped-request
expiry and late-reply rejection; low-frame-rate call leases; SVG icon hit handling;
and observing native telemetry without creating a second presence heartbeat.
Visual inspection also corrected the administrator metrics pane so it scrolls
inside its allocated area without painting over the footer.

## Enforced resource bounds and continuation

Registry: 32 entries per owner / 128 overall and 12 KiB metadata including binding
reserve. Browser: two pages per owner / four overall, maximum 1280×960 at 30 fps,
one outstanding capture and one latest frame; process-tree limits 1 GiB / 128
processes. Photo: 1920×1080, one pending capture/save globally, one-second interval,
15-second readback timeout, 32 gallery entries per generation / 64 overall, 160×90
thumbnails at most 8 KiB each, PNG at most 16 MiB. Calls: 16 concurrent and one per
player. Administration: four pending per actor / 64 overall, eight requests/second.

Use [setup and controls](resource-interactions.md) to build/install and run the
committed native harness. For remaining physical acceptance, provide the device
access listed above, repeat the same two-player sequence using a real controller,
verify physical V/LB release, mute/deafen and microphone/speaker sound, and use X to
photograph the second player. On Windows repeat composition, WebView2 process
failure/restart, controller input, voice and Desktop PNG export natively. Longer
cycles should record process-tree memory and verify every retired generation exits.

Broader platform gaps remain in
[platform-capabilities-evidence.md](platform-capabilities-evidence.md): this work
does not claim exhaustive native solver, shared dynamic authority, WAN, long native
worker soak or private-fixture acceptance. Nothing was pushed, published or deployed.
