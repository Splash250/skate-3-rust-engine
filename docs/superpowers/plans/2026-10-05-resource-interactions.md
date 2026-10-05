# Resource interactions and phone implementation plan

**Goal:** Deliver an extensible resource-owned RP interaction menu, in-game HTML phone and dashboards, real calls, and local scene photographs.

**Architecture:** Extend the existing granted resource commands, generation retirement, packaged browser bridge and native input host. Resource scripts own presentation, menu policy and call signaling. Existing authenticated administration, inventory, settings and Opus routing remain authoritative.

**Baseline:** `cross-platform-dedicated-server`, clean `ffe3374166cfbc64c79b14690583fd780d2b7242`, verified before editing. The user's detailed implementation request is the binding specification and authorizes local implementation, fixtures, tests and commits. Work stays in the requested checkout; no push or deployment.

## Constraints and decisions

- Keep all engine APIs independent of resource names and RP modes.
- Resource descriptors have host-supplied owner/generation, bounded names, metadata, destinations and callbacks. No executable command strings.
- Client visibility is presentation. Server operations derive the actor from the admitted sender and recheck live permissions, including before private completion delivery.
- Browser surfaces extend the existing companion process: immutable packaged assets, restricted CSP, JSON bridge and OS process containment remain. Native snapshots feed bounded Bevy textures; no external-window substitute.
- Photo shutter is physical host input only. Reuse Bevy image-target screenshot readback; never capture desktop windows. Resolve Desktop through platform directories; host selects unique PNG names. Only managed own photos enter gallery.
- Dedicated resource-owned policy suppresses manual session marker controls/HUD and restores on retirement/disconnect. Automatic recovery remains unchanged.
- Offline replay retains View/Back/Select. Resource master binding defaults to a 600 ms hold in dedicated gameplay; local bindings remain separate from server settings.
- Preserve voice opt-in, physical PTT, mute/deafen and instance boundaries. Calls survive phone closure.
- Serialize all Cargo work. After the original NTFS3 target stalled, final builds/tests use the separate `/tmp/skate-phone-20261005/target` and `flock /tmp/skate-phone-cargo-recovery.lock`. Evidence and disposable stores stay outside Git under `/tmp/skate-phone-20261005/`.

## Dependency-ordered tasks

### 1. Generic interface registry and input ownership (root)

Files: `skate-mods/src/interactions.rs`, command schema and SDK wrappers; `skate-game/src/modding/interactions.rs`, existing input/menu/marker host integration.

- [x] Define versioned bounded register/remove/list/invoke operations, descriptor validation, unique namespaced IDs, generation retirement and supported destinations.
- [x] Write validation/limits/stale-owner tests, run failing cases, implement, rerun.
- [x] Route invocation only to live owner callbacks, through explicit capability and dependency contracts.
- [x] Implement host binding arbitration, local remapping, reliable close, transition release gate and dedicated-only owned marker policy.
- [x] Test duplicate/conflicting bindings, held transition input, cleanup and offline restoration.

### 2. Composited browser surface (browser track; prerequisite for 4–5)

Files: `skate-browser/`, `skate-game/src/modding/browser.rs`; root wires scheduling/open signature.

- [x] Extend Options with optional `surface={anchor,scale,offset,fps}` while retaining companion compatibility.
- [x] Capture platform webview into bounded latest-frame IPC, one outstanding capture, at most 1280x960/30fps.
- [x] Composite into owned Bevy ImageNode; physical pointer/keyboard/controller forwarding; host Escape on failure.
- [x] Test bridge size/protocol validation, stale closure, snapshots, navigation, process failures and bounded frame retention.
- [x] Run actual Linux renderer and visually inspect in-game output; keep native Windows evidence separate.

### 3. Server-owned calls (calls track; independent of renderer)

Files: `resources/phone-calls/`, runtime and server integration tests; minimal voice-result channel correlation.

- [x] Implement ringing/connecting/active/ended transitions with canonical actual sender, exact generation IDs, cooldowns, busy/timeout/instance/disconnect cleanup.
- [x] Create two-member voice channel only after acceptance; publish active after successful host result; cap 16 calls.
- [x] Expose client `request` and `snapshot` exports for the phone through exact dependencies.
- [x] Test all transitions, forged/stale requests, races and actual encoded voice routing isolation.
- [x] Root adds trusted controller PTT and focus semantics; verify mute/deafen/disabled state separately from physical acoustics.

### 4. Resource menu, phone and inventory presentation (after 1–3)

Files: `resources/master-menu/`, `resources/phone/`, `resources/inventory-ui/`, independent app example.

- [x] Master policy uses typed settings for enabled IDs/order/categories/access; descriptor list drives both menu and phone apps.
- [x] Build original responsive bottom-right phone with two CSS themes, home/status/apps, contacts/calls, camera/gallery/settings, controller focus and transitions.
- [x] Integrate existing inventory backend and voice controls; no placeholder apps.
- [x] Add theme/app contract and a separate resource app using the same registration API.
- [x] Validate packaged assets, real resource callbacks, controller and keyboard paths, restart while open and failures.

### 5. Authorized administration dashboard (after 1–2)

Files: generic resource administration bridge in existing server/accounts boundary; `resources/admin-dashboard/`.

- [x] Reuse existing HostAction permissions/settings/profiling/lifecycle implementation. Bind admitted actor, resource generation and bounded request ticket.
- [x] Render typed forms, pending restart values and actual mutation results; private reads and writes checked independently.
- [x] Recheck authorization before result delivery; discard stale/revoked UI responses.
- [x] Test forged actor, non-admin direct request, role changes, private schema exclusion, settings persistence and resource retirement.

### 6. Native photo mode and constrained gallery (photos track; independent of browser)

Files: `skate-mods/src/photo.rs`, `skate-game/src/modding/photos.rs`, `skate-platform/src/photos.rs`; root wires command/capability/cleanup.

- [x] Implement mode/gallery/thumbnail commands; no script shutter or arbitrary path command.
- [x] Host physical shutter, framing/zoom and 1920x1080 scene-only target with asynchronous screenshot/save.
- [x] Resolve Desktop, confined create-new PNG files, one pending job, one-second rate limit, 32 metadata records and bounded thumbnails.
- [x] Test path/unique/write errors, physical-action enforcement, queue/rate limits, retirement and PNG decode/dimensions.
- [x] Integrate phone viewfinder/gallery and visually inspect saved photo containing the second player and excluding UI.

### 7. Complete RP installation and combined acceptance

Files: local RP resource config/recipe and `docs/multiplayer/resource-interactions.md`, evidence ledger.

- [x] Assemble exact resources/dependencies, grants, typed policy defaults and role instructions using existing operational patterns.
- [x] Run focused crates/runtime/server/UI tests and affected-consumer compilation.
- [x] Run two native clients, navigate menu/phone, call/accept/hangup, exercise encoded audio, photo/gallery, authorized setting edit and forged-request denial.
- [x] Cycle renderer/camera/resource restart, inspect cleanup and bounded resource use.
- [x] Recheck physical devices once; report native Linux, synthetic input/audio and unavailable native Windows separately.

### 8. Independent review and closeout

- [x] Fresh code and security-boundary reviews; reproduce findings and fix with regressions.
- [x] Run final scoped verification, AST-only `graphify update .`, diff/link checks and inspect staged paths for private/generated content.
- [x] Make a cohesive tested local commit; report its hash and final tree status in the handoff. Nothing pushed/published/deployed.

## Review focus

1. Resource retirement during renderer/GPU/admin callbacks must not deliver to replacements or keep input/channel ownership.
2. Permission revocation must prevent private reads and late results, even through direct forged network requests.
3. Held controls crossing close/pause/camera transitions must not become skating/marker actions.
4. Renderer and PNG work must retain fixed queue/memory limits and fail with a usable escape route.
5. Call acceptance races, resource restarts and instance changes must never expand audio membership.

## Evidence matrix

Implementation and the final 17-phase Linux native flow pass, including active-call/camera retirement and renderer-failure recovery. Final regression execution and the inspected dashboard layout correction are recorded in the evidence ledger; physical device and native Windows acceptance remain explicit prerequisites.
Track actual commands/results in [the evidence ledger](../../multiplayer/resource-interactions-evidence.md). Compilation, synthetic protocol tests, native screenshots and physical device acceptance are separate evidence classes.
