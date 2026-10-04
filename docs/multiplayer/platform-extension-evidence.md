# Platform extension requirement-to-evidence ledger

Started 2026-10-04. Branch `cross-platform-dedicated-server`; baseline and initial
HEAD `69ed3779342bd61ecff7f8d40628269152a79afc`. Initial working tree clean.
The initial implementation request prohibited commits. The user subsequently
authorized local commits on 2026-10-04; pushing, publication and deployment
remain unauthorized. Earlier no-commit statements below describe the initial
implementation handoff, before this authorization.

Statuses: Pending; In progress; Implemented but unverified; Implemented and
verified; Blocked (with concrete prerequisite). A subfeature's tests do not
establish completion of its parent capability or graphical/device acceptance.
The matrix is current; dated milestone notes below retain earlier failures and
pending checks for traceability and are superseded by later evidence.

| Requirement | Status | Evidence / remaining acceptance |
| --- | --- | --- |
| 1 Shared entities, state and instances | Implemented and verified | Real UDP ownership/restart/spoof/late-join tests and two native-client interaction/isolation pass. Both players produce actual server and native crate contacts; private instance retires peer/replicas/colliders and public return restores them. Fresh graphical rerun also restores both peer names after return. |
| 2 Teleports and authoritative gameplay | In progress | Immediate trusted resets, hot-reload continuity, approved travel cancellation and all 11 competition tests pass. Two native clients complete course-v1 through ordinary controls: server-derived 175 points, 2545 ms, three checkpoints, one pickup, zero extra resets. Capsule walls/ceilings, bounded terrain support, speed/acceleration and forged-result rejection are tested. Full server-derived articulated stock trick/landing/combo scoring remains unimplemented and open. |
| 3 Negotiated capacity and bounded transfer | Implemented and verified | Negotiated bounds, >16 MiB packages, >64 KiB persistence and real 12 KiB Lua bidirectional transfer/cancel/deadline pass. Combined64 real UDP simulated clients with objects/events/8 Opus talkers passes; measurements below. Not64 graphical clients or WAN capacity. |
| 4 Browser interfaces | Implemented but unverified | Actual Linux WebKit3 tests include memory enforcement; authenticated downloaded inventory synchronizes, stops, restarts with a new child and closes on disconnect. Keyboard focus/timeout recovery checked in actual Chrome. Native Windows remains unverified. |
| 5 Voice and radio | Implemented but unverified | Actual Opus/UDP and authenticated radio/proximity/isolation pass; CPAL ALSA deterministic capture/playback/teardown passes. Eight-talker fairness regression fixed; default bounded512-packet egress verified under combined64-client workload. Physical microphone/native Windows unverified. |
| 6 Backend services/persistence | Implemented and verified | Local asynchronous SQLite/HTTP, transactional progression, concurrent updates, rollback, full-process restart, cancellation/deadline/results/origin limits verified in service and actual-server tests. Native Windows and sudden power-loss validation unverified. |
| 7 Accounts/permissions/administration | Implemented and verified | Persistent identity/role/ban/revocation, TLS/AEAD/replay protection, actual admin status/kick and fresh actual game login/UDP tests pass. Authenticated inventory uses verified account identity. Native Windows filesystem/admin runtime remains unverified. |
| 8 Maps/streaming/custom parks/placement | Implemented and verified | Two native Linux clients download/enter the original park and survive park→base→park transitions with exact collision/rail counts, unpaused gameplay and all shared objects. Owned six-map-transition cleanup and native paired-truck grind tests pass. Native Windows remains unverified. |
| 9 Animation/characters/gameplay hooks | Implemented and verified | Both native clients display distinct imported held poses on both25-joint robots, correctly oriented hats and markers. Stop visibly restores stock skaters and zeros all owned animation registrations; restart restores both presentations. Cosmetic clips and course-v1 hooks are verified; stock trick authority remains open under requirement2. |
| 10 Lua/JavaScript/C# | Implemented but unverified | Real Lua/QuickJS and isolated .NET10 client/server resources, cross-language exports, generations, capability revocation and containment pass on Linux. Native pattern runaway fixed and tested. Native Windows worker runtime remains unverified. |
| 11 Reusable resources/developer experience | Implemented and verified | Redistributable examples, placement and repeatable native verification tools, SDK declarations, timing/heap/queue/error diagnostics, setup docs and explicit packaging inventory pass local checks. Two-client interaction, course, presentation and map lifecycle pass. Independent review findings are fixed and tested. Linux/Windows CI definitions are updated; hosted jobs and native Windows acceptance have not run. |
| Existing resource guarantees | Implemented and verified | Fresh cache/content/contracts/server/UDP/runtime suites cover distribution, process cache reuse, confidential files, dependencies, capability/persistence/generation isolation. Actual cold/warm game reconnect and authenticated browser stop/restart/disconnect pass. Native Windows/mixedOS unverified. |
| Native Windows, mixed OS and physical audio | Blocked | Requires a native Windows host, a mixed Windows/Linux session and physical capture/playback devices. Linux game/WebKit and synthetic CPAL audio actually ran. CI is configured but has not run remotely. |

## Investigation

- `git status --short`: empty; branch and HEAD verified; baseline ancestry succeeds.
- `graphify query "resource platform dedicated server entities teleport budgets Lua runtime persistence"`
  succeeded, with skill/package version warning (0.9.42/0.9.44). Current source
  confirms graph navigation results.

## Execution evidence

Fresh integration milestone (before shared entities/accounts/browser/C# additions):

- `cargo test --locked -p skate-net -p skate-resources -p skate-services -p skate-server`: passed. Network: 91 tests across all suites; resources: 5 contracts + 15 distribution; server: 24 parent tests (plus subprocess helpers actually executed); services: 9 parent tests plus a fresh child-process reopen. No failures. Raw local log `/tmp/skate-platform-core-tests.log`.
- `cargo check --locked -p skate-game --bin skate3rust`: passed after replacing a queue fairness `Cell` with `AtomicBool` to satisfy Bevy's `Resource: Sync` requirement. Local log `/tmp/skate-platform-game-check.log`.
- `cargo test --locked -p skate-mods --lib --test resource_runtime --test physics_api --test validate_example -p skate-resources --test contracts`: 97 passed at the earlier runtime milestone. Two performance tests remain ignored; the finalizer helper is launched by its parent test. Subsequent runtime review regressions: `cargo test --locked -p skate-mods --test resource_runtime`: 34 passed; helper executed by its parent.
- `python3 -m unittest tools.test_package_server`: 4 passed after adding bundled Lua/JS/progression resources, config and dependency license notices.
- Reviewed and corrected: correlated scheduling cursors starving actor/recipient pairs; state tombstones bypassing byte accounting; aggregate small/bulk pending limits; small incoming byte preflight before acknowledgement; stale backend outputs after failed callbacks; useful oversized backend-result errors; Lua `coroutine.close` swallowing exhaustion; JS subscription callback retention and plain-JSON export enforcement.

### 64-player UDP measurement

Actual server and 64 simulated clients used individual loopback UDP sockets in
one test process, with movement plus 256 × 1 KiB Lua event echoes. Last explicit
measurement: 5.001 seconds, 8,513,202 bytes client TX and 20,873,014 bytes client
RX, all clients observing all 63 other actors, 63,739 movement samples, p95 source
age 110 ms (max 228 ms), end-of-run observer age p95 589 ms, echo p95 1340 ms.
The fixture deliberately discarded 1,064 packets. This is not a measurement of
WAN loss. RSS 174,048 KiB includes the server and all simulated clients together.
Shared-object/voice coexistence and 64 graphical windows were not tested.

### Subsequent vertical slices

- Shared entities: 16 net unit + 30 dedicated tests passed; authoritative Rapier world tests (2), actual UDP shared-object/late-join/disconnect/resource-cleanup test, and five maximum inventory/fairness tests passed. Game check passed before later account client edits. Graphical two-client acceptance remains open.
- Browser: `cargo build --locked -p skate-browser --features host` passed on Linux with locally extracted WebKitGTK development headers; two library tests and two actual WebKit graphical tests passed. The graphical tests exercise JSON round-trip, blocked network requests, focus, process teardown and an infinite-JavaScript watchdog. Log `/tmp/skate-browser-graphical-tests.log`. Shipped inventory UI additionally passed Chrome desktop/narrow-window DOM/IPC checks; screenshots `/tmp/skate-inventory-desktop.png` and `/tmp/skate-inventory-narrow.png` use explicitly synthetic UI state.
- Accounts: `cargo test --locked -p skate-accounts -p skate-server --test accounts`: 3 account tests + 1 actual server test passed. Covers TLS trust, roles/reopen, tamper/replay/reflection, plaintext and actor-spoof rejection, actual status and kick, immediate bearer/UDP revocation.
- Managed runtime: four actual isolated .NET 10.0.401 Linux tests passed (5.76 seconds), including C#→JS→Lua→C#, shared host APIs, restart/capability revocation, forbidden native/IO/reflection/process/thread APIs, timeout/memory failure isolation and 200 callback ticks. Requires local `SKATE_DOTNET_ROOT` and `SKATE_MANAGED_HOST`; worker built from repository source.
- Windows managed launcher: the actual module cross-compiled with installed `x86_64-pc-windows-msvc` target in an isolated minimal check crate; this is not native Windows execution.

### Unavailable or not yet executed at the early milestone

- Native Windows, mixed Windows/Linux and actual microphone/device acceptance.
- Private `skyline.glb` integration prerequisite: retained unchanged; not run.
- Browser companion graphical tests passed; game browser/map/animation acceptance remains pending.
- Final graph update and end-of-task branch/diff verification remain due.

## Work ownership during implementation

Shared entities/scoped state then maps: `/root/teleports`; protected accounts/admin then voice: `/root/backend`;
C# worker/common language bridge then animation: `/root/lua_runtime`; browser host, authenticated inventory, resource
integration and overall verification: root. All builds sharing `target/` are
serialized. The approved complete request remains active; this ledger does not
mark the full platform complete.

### Game, inventory and voice integration milestone

- `cargo test --locked -p skate-game --bin skate3rust authenticated_tests`: 1 real TLS/UDP test passed; rejects foreign endpoints, plaintext, tampered/replayed packets. `dedicated_config_tests`: 3 passed. `cargo build --locked -p skate-game --bin skate3rust -p skate-server --bin skate-server`: passed (1m04s).
- `cargo test --locked -p skate-server --test inventory`: 1 passed; downloaded public files, authenticated stable account, concurrent SQLite transactions/rollback, forged account field ignored, full host restart persistence.
- Actual WebKit graphical tests: 3 passed, including a resource process tree exhausting its enforced Linux cgroup memory budget and being retired. Linux requires delegated memory/pids cgroups; native Windows remains unverified.
- Fresh game graphical smoke `/tmp/skate-resource-graphical-d4qg6cv_`: cold activation downloaded4476B, separate-process warm reconnect reused4476B with0B download; stop/reconfigure activated new set and reused691B. All3 screenshots and GAME_VERIFY_OK succeeded, all3 server departures within3s.
- Authenticated game inventory displayed and synchronized through actual WebKit process. Stop removed the child process. This exposed a hot-reload protocol defect: a new roster incarnation could carry the previous resource ACK before the new offer was published. Restart/disconnect acceptance remains open pending the fix and rerun; initial script result counting was corrected to count one log record per synchronization.
- Agent voice runs (pending root inspection): real Opus/UDP/router/resampling4tests + CPAL ALSA synthetic capture/playback/teardown1parent test passed; authenticated server voice1test passed with3TLS clients, proximity, radio, mute, replay/identity denial, instance/resource cleanup. Physical microphone/native Windows not exercised.

- Graphical hotreload ACK regression reproduced by a packet-by-packet test; fix retains strict ACK validation and clears stale server records before incarnation publication. Dedicated31 + resource12 tests passed afterward. A separate newly-epoched first-body discontinuity regression is being added.
- Placement/export Python tests3 and decoded map tests8 passed; community-park is entirely authored redistributable data. Root integrated terrain into server shared simulation and required-world spawn, plus bounded server competition contract. Native rail provider merge and full graphical map acceptance still pending.
- Animation parser2 and synthetic Bevy3 tests passed; full resource runtime45 passed (+executed finalizer subprocess helper). These are not graphical imported/remote skin acceptance.
- Packaging/Linux-script/placement Python suites:18 passed after expanding the explicit package file inventory. CI now builds native browser/audio and trusted .NET worker, runs account/voice/runtime suites on Linux/Windows, and adds graphical browser jobs. CI workflows were edited locally; no hosted run has been executed.

### Combined voice workload and explicit transfer handles

- `cargo test --locked -p skate-server --test resources explicit_bulk_script_handles_report_delivery_cancellation_and_deadlines_over_udp`: passed1 actual socket/HTTP/Lua test. Transfers12KiB in each direction, observes acknowledgement completion, explicitly cancels another queued transfer and expires a1ms request; cancelled/expired payloads never reach the receiver callback.
- `cargo test --locked -p skate-server --test capacity -- --nocapture`: passed64 real UDP simulated clients with movement,2 moving server objects,256×1KiB resource echoes and8 real Opus-encoded talkers. The first run exposed excessive voice queue loss under the256-packet egress default.
- Controlled512-packet egress rerun passed:5.007s, TX8,920,808B, RX41,318,389B,45,775 movement samples, movement source age p95109ms/max218ms, observer age p95793ms, resource echo p951363ms,9,962 object samples,106,537 voice frames and voice age p9544ms. Combined server+simulated-client RSS144,644KiB. Deliberately discarded2,613 datagrams; this is controlled loopback loss, not measured WAN loss. Router accepted1728 voice frames, rejected0, dropped320 queued recipient copies (versus48,864 at256), sent111,016 voice/control packets/16,582,976B. Every listener heard all eligible talkers. Default512 adjustment and its exact-default regression now pass.
- Agent fresh voice/device suite6 passed, server library5 plus authenticated voice1 passed. Game animation cleanup6, voice lifecycle2, asset-limit2 and native rail overlay1 passed; packaged presentation Lua replication1 passed.
- Lua native pattern matching bypassed the instruction hook in a kill-bounded subprocess reproduction. Budget guards, kill-bounded containment subprocess and ordinary-pattern regressions pass; the subsequent complete112-test fixture-free runtime run is green.

### Fresh integration and visual review milestone

- Full game suite:354 passed,104 ignored,1 pre-existing setup-cache test failure. `setup.rs` is byte-identical to baseline (blob477b2770b096d178e9983e5b10fe995e445f5a5c). Its `pipelines_accept_valid_group_outputs_when_fingerprint_changes` fixture changes both core/maps fingerprints but supplies only a maps receipt, contradicting the existing validation contract. No unrelated fixture or assertion was weakened. Log `/tmp/skate-game-full-final.log`.
- Broad platform rerun: `cargo test --locked -p skate-net -p skate-resources -p skate-services -p skate-accounts -p skate-server -p skate-voice -p skate-browser` passed196 parent tests plus3 child-process test executions;2 ignored helper entries are invoked by their parents. Log `/tmp/skate-platform-broad-final2.log`. Includes fresh cache/distribution15, dedicated33, resource contracts7, explicit transfer handling, authenticated inventory/server/admin, competition6 and combined capacity.
- Adding all of `skate-data` to that command first exposed unchanged `examples/hud_data.rs` importing `hud_runtime.rs` without `scoring_hud` (E0433); both files have no diff. Focused changed map decoding tests pass; the unrelated example prevents the all-target data run. Log `/tmp/skate-platform-broad-final.log`.
- Fixture-free Lua/JS/local-mod suites freshly pass112 tests (lib57, animation2, physics3, resource runtime49, example1). The new actual Host regression exercises64 distinct one-time numeric-instance park pads and departure reuse. Two benchmarks remain ignored; finalizer helper runs through its parent.
- Authenticated graphical inventory `/tmp/skate-inventory-graphical-uib8lmu0`: actual browser synchronization, child removal on stop, new child/synchronization after restart, removal on disconnect; three resource admissions and no panic. Separate desktop/narrow actual Chrome UI checks cover purchase IPC, keyboard focus restoration, real10-second timeout, disabled duplicate purchase until authoritative refresh, Escape and overflow. Scoped visual reviewer approved both fixes; this does not fabricate missing earlier formal design-workflow provenance.
- Park visual failures were investigated rather than accepted from metadata. Untextured portable materials previously sampled absent textures and rendered black; explicit portable albedo now passes a material regression and all6 shader validation tests. The original tiny mascot was replaced with an authored25-joint robot; validated owned remote skins now avoid invalid rest-bound culling. New spawn pads exposed a numeric-instance script error, fixed with the real Host regression above.
- Fresh native run `/tmp/skate-park-graphical-ivzzbe1k`: both required worlds committed with32 collision triangles and1 native grind primitive; both rendered the ramp, rail, markers and two custom robots, and received both actors' clip-marker/skin-ready callbacks. Shared crate converged to identical `[3.2517738,0.5999665,-0.0045631942]`; both recorded13 native network contacts. A static floor/portal disappearance remains reproduced in the second client's full replicated inventory and is not a visual acceptance pass for shared entities.
- Review-driven diagnostics regressions reproduced then fixed: escaped control characters in16 valid log lines exceeded the administration snapshot limit, and failed Lua updates retired silently. Resource status now bounds serialized bytes with omitted counts; full status preserves player identity while trimming optional gameplay if necessary. Retirement logs once with resource/generation/reason. Server library11 tests pass. Logs `/tmp/skate-status-budget-red.log`, `/tmp/skate-runtime-log-red.log`, `/tmp/skate-server-diagnostics-final.log`.
- Packaging review corrected obsolete anonymous-only account claims and included voice/course usage READMEs; all5 package tests pass. CI now includes animation/parser/generated-asset tests and Linux synthetic game animation/entity/streaming/shader checks. Hosted jobs have not run.

- Focused `cargo test --locked -p skate-data --lib --tests` passes40 tests with2 asset probes intentionally ignored; this avoids compiling the unrelated broken example while exercising all changed map decoders. Fresh Python package/Linux-script/placement/presentation/graphical-harness checks pass26 tests. Every193 newly introduced external crate version has an explicit record in `PLATFORM-NOTICES.txt` (read-only lockfile comparison).

### Native travel, presentation and lifecycle review

- Real graphical traces exposed duplicate native relocation: `Skater::travel` queued both a direct teleport and an actor-reset reply; the second relocation was interpreted as another recovery request, producing a500ms loop. An actual owned-asset full-frame regression reproduced two relocations, then passed with one actor-mediated relocation and preserved requested velocity. All3 native network/physics tests and6 shader tests pass (`/tmp/skate-native-travel-red.log`, `/tmp/skate-native-network-green.log`, `/tmp/skate-park-shader-final.log`). Existing direct relocation remains tested.
- Two UDP entity tests pass, including200Hz clients against100/59/30Hz host cadence and repeated500ms teleports. The exact temporary crate-only inventory was reproduced inside the bounded150ms recovery window; every static/dynamic entity recovered afterward. No speculative protocol change was applied. Log `/tmp/skate-entity-cadence-final.log`.
- Close two-client presentation `/tmp/skate-presentation-proof-20261004a/presentation-poses` visibly shows both imported held poses and establishes HEAD localX as the attachment-up axis. The original hat offset/orientation and original head visor geometry were corrected accordingly. Each client logged only three expected initial relocations (base, required world, pad), with no repeated reset loop. Stock restoration was obscured by a newly reproduced map-cancellation pause/menu bug, so that acceptance was not marked passed.
- Native owned rig override/restore test passes. The existing ignored private-world lifecycle fixture failed before its first transition because it omitted `Assets<SkyMaterial>`, already required by baseline renderer code. One real resource initialization was added without weakening assertions; rerun pending. Original failure log `/tmp/skate-owned-native-lifecycle-final.log`.
- Added bounded exact native shared-object contact IDs and captured owned animation counts, pause/menu state and periodic screenshots to the opt-in generic verifier. These distinguish actual crate contact from floor contact and make lifecycle cleanup inspectable. Reusable park and presentation tools retain all phase evidence locally; no private assets are copied into the repository.
- Server diagnostics now count true solver-contact steps per entity/actor, bounded256 pairs with retirement/disconnect/instance pruning. The server also drains otherwise accumulating physics contact-edge events each tick. A separate borrowing solver-contact query excludes sensors and separated broadphase candidates without changing existing local-mod event semantics. Dynamics1 and server11unit+2UDP tests pass, including real crate contact for the approaching actor and none for the distant actor. Log `/tmp/skate-entity-telemetry-green.log`.

### Final native presentation and map lifecycle evidence

- Fresh game/server build passes (`/tmp/skate-platform-acceptance-build.log`). Full game suite now357 passed,105 ignored,1 unchanged setup-cache fixture failure (`/tmp/skate-game-complete-final.log`); the baseline-identical failure is described above.
- Map cancellation previously abandoned the publishing phase when a resource readmission returned to the already active world. A red→green regression now checks restoration of the original pause/menu state while preserving intentionally paused/open state (`/tmp/skate-map-cancel-red.log`, `/tmp/skate-map-cancel-green.log`).
- `/tmp/skate-presentation-proof-20261004b/presentation-poses/results.json`: six phases, both clients pass and exit0 without errors. Root inspected both distinct held poses and visible stock restoration. Pose phases have banks1/layers2/appearances2/attachments2/pending0; stopped phase has all zero. Every phase remains unpaused with menu closed and all three shared objects. Original robot/head/hat geometry and HEAD-local-X attachment orientation are corrected; no retail geometry is distributed.
- `/tmp/skate-park-lifecycle-20261004b/lifecycle/results.json`: both clients pass all five phases (admitted, presentation stopped/restored, world stopped/restored). Community park generation1 has32 triangles/1 native grind primitive, base test world generation2 has344/7, restored park generation3 has32/1. Root inspected screenshots showing base-world replacement and the restored colored ramp/rail/markers. Both clients retain the three shared-object solids and finish with exit0.
- The owned world cleanup fixture passes all six transitions (BlackBoxPark→base→University→base→BlackBoxPark→base), including mesh/image/reference/physics retirement assertions (`/tmp/skate-owned-map-lifecycle-green.log`). This uses locally owned assets that are not part of the patch.
- Native network regression3, dedicated adapter2, session-marker6 and shader6 checks pass. The additional ignored `physics::wipeout_tests::marker_reply_restores_on_foot` fails at tick61 after its manual reset: PhysicsAir versus PhysicsGround. A method-scoped A/B reproduces the identical failure with the original HEAD `check_teleport` and current method. It rules out that method change, not every change in this branch; the failure remains unresolved. Exact source bytes were restored and the graphical executable hash was unchanged (`/tmp/skate-marker-ab-metadata.json`, `/tmp/skate-marker-baseline-ab.log`, `/tmp/skate-marker-current-ab.log`). The three actual native network tests passed again afterward (`/tmp/skate-native-network-after-marker-ab.log`).

### Native shared-object interaction and instance evidence

- `/tmp/skate-park-interaction-20261004c/interaction/results.json` passes with both clients exiting0. Each actor received an approved approach velocity, never an object impulse. Server solver contacts for exact crate2 precede horizontal displacement of at least0.29083m and0.07998m, respectively. Native solved contacts with crate2 appear for8 and164 frames in the corresponding captures. Delayed join receives the current same crate and all three public objects.
- In the empty verification-only instance7, both clients report zero remote players; the moved client has an empty replicated inventory, zero registered/live shared solids and no current shared contact IDs. The public client retains all three objects. Both regain peer visibility and all three solids after public return. Root inspected both contact captures and the private-instance image. Voice/scoped-state isolation has separate actual socket evidence; the screenshots do not establish audio isolation.
- This run exposed a resource-policy defect: community-park's initial pad allocator admitted private players into public pads. An actual Host regression first failed for both returning and newly seen private actors, then passed with allocation restricted to instance0. It retains coverage for64 unique pads, departure reuse and later public entry. The example script is fixed; no resource-specific rule was added to Rust.

### Final independent review corrections

- Competition review found that rejection of a newer authorized epoch also issued a correction into the old course instance. Two actual protocol regressions failed, then passed with trusted epoch/instance cancellation before observation filtering. Same-instance and cross-instance approved travel now retain their destination/epoch, including readmission with no fresh BODY. Invalid movement under the current epoch still rejects and corrects. All8 competition tests pass (`/tmp/skate-competition-authority-red.log`, `/tmp/skate-competition-authority-green.log`).
- Available measurement host: x86_64 Linux7.0.0-34,32 logical CPUs (CPU reports `Genuine Intel(R) 0000`),129331732KiB system RAM. Graphical runs use Vulkan on NVIDIA GeForce RTX4080 SUPER, driver595.91.07. These are local development-build/loopback results, not release/WAN sizing claims.

- Metadata replay review reproduced missing player names after an instance round trip. Dedicated application acknowledgements now carry both existing movement epochs, validate the sent/visible record, and retire visibility-specific ACK state. A delayed pre-transition ACK cannot suppress a lost replay after return. Legacy peer wire bytes remain unchanged. Fresh net suite113 tests pass (`/tmp/skate-name-net-suite.log`); matching new client/server binaries are required.
- Packaging review found global `*.glb` ignores hiding the two original presentation assets from a source patch. Exact exceptions now include only those generated assets; private `platform-example-data/` is ignored. All59 explicit resource package inputs are present and visible to Git. A red→green ZIP-content regression now verifies every bundled example README's local setup links. The archive explicitly includes26 documentation/declaration files; no recursive source/private-data inclusion was added. All6 package tests and the combined36 Python tooling tests pass (`/tmp/skate-package-docs-red.log`, `/tmp/skate-package-docs-green.log`, `/tmp/skate-platform-python-final2.log`). CMake prerequisites and separate server/client-terminal startup instructions are corrected.

### Consolidated final Rust and capacity run

- `cargo test --locked -p skate-net -p skate-data -p skate-resources -p skate-services -p skate-accounts -p skate-server -p skate-dynamics -p skate-voice -p skate-browser --features skate-voice/devices --lib --tests -- --nocapture` passes283 parent tests, plus4 child-process helper executions. Six ignored declarations comprise3 explicitly parent-invoked helpers and3 asset/optional probes. This includes the new competition8, dedicated34, server library11, dynamics39, map/data40, distribution15, and real CPAL synthetic-device path. Log `/tmp/skate-platform-consolidated-final.log`. Browser host feature tests retain their separately recorded actual WebKit evidence.
- The final64-client workload in that run uses the default512 egress budget and current contact telemetry/metadata framing. Over5.018s: client TX8,810,349B, RX41,237,877B;43,001 movement samples, source-age p95110ms/max216ms; end observer-age p95703ms;256 resource echoes (256KiB), echo p951370ms;9,855 moving-object samples;8 Opus talkers,107,567 received voice frames, voice-age p9543ms. Combined server+simulated-client RSS/peak138,540KiB. The fixture deliberately discarded2,524 datagrams; this is controlled loopback loss, not WAN packet loss. Router accepted1,736 frames, rejected0, dropped0 queued recipient copies and sent111,960 voice/control packets totaling16,732,800B. Every listener heard all eligible talkers. This supersedes earlier capacity measurements. The simulated receivers validate/routinely receive encoded voice packets; this workload does not run64 audio decoders/devices, native player rigs or graphical windows. Separate voice tests exercise real Opus decode and CPAL playback.
- Actual isolated .NET10 client/server/cross-language/lifecycle/containment integration rerun passes all4 tests in5.83s with the locally built trusted worker (`/tmp/skate-managed-final.log`). No native Windows execution is inferred from this result.

- Final game/server build passes (`/tmp/skate-platform-build-final.log`). Fresh full game suite358 passed,105 ignored,1 baseline-identical setup-cache failure (`/tmp/skate-game-consolidated-final.log`). The new real-Host game retirement regression is included in the358 passes.
- Final matching-binary interaction rerun `/tmp/skate-park-interaction-20261004d/interaction/results.json` passes both clients with exit0. Exact crate2 server contacts accompany horizontal displacements of at least0.263093m and0.046148m; native client contact counts are8 and174. Private instance again has no peer/replica/shared collider, and public return restores three objects, peer visibility and the original player names on both screens. Root inspected both return captures and the private capture. This closes the metadata replay regression graphically.
- First native course run (`/tmp/skate-native-course-20261004a/native-course`) mounted through normal controls and received an approved start, but failed with `terrain_collision` during normal settling before pushing. Source investigation confirms BODY.root uses `animation_to_world`, not a feet position; the on-board animation root settled below the floor. This is a real verifier/native mismatch under correction, not a passing course demonstration.

- Two final containment regressions reproduced and passed after fixes. A failed voice-result callback previously left drained owner/dependent engine commands executable; actual running-owner checks, immediate metadata synchronization and nested-request retirement guards now prevent those effects. Eight caught export failures previously retained eight1,048,751-byte diagnostics outside the Lua heap limit. A bounded UTF-8 formatter now caps each retained error at2048 bytes, diagnostics at128, and pending export faults at one per owner, while metrics count every failure. The fixture-free mods suite passes113 tests (57 library,50 resource runtime,2 animation,3 physics,1 example). Logs recovered verbatim from the agent's saved command records are `/tmp/skate-voice-retirement-{red,green}.log`, `/tmp/skate-export-retention-{red,green}.log` and `/tmp/skate-mods-final-113.log`; these are recovered execution evidence, not additional reruns.

- Three new verifier regressions reproduce the native animation-origin settling failure, then pass with a collision reference adjusted only to trusted terrain support within0.20m. Raw movement/speed/acceleration/window scoring remains unchanged; full capsule casts still reject walls, low ceilings and deeper below-floor movement. All11 competition tests pass (`/tmp/skate-course-native-root-red.log`, `/tmp/skate-course-native-root-green.log`). Independent source review confirms the native coordinate convention; this is a bounded course-v1 policy, not full rig simulation. A fresh server-only build passes before the graphical rerun.


### Final native course and content acceptance

- `/tmp/skate-native-course-20261004c/native-course/results.json` passes with both clients exiting 0. Ordinary mapped mount/push/brake controls drive the actual native skater; the server awards exactly 175 points for three checkpoints and one pickup in 2545 ms. Ten distinct on-board observations span 5.611995 m, with zero additional native travel/reset events during the continuous attempt. The verifier injects no score, body pose or velocity during the attempt. Both completed-phase captures show one peer, three shared replicas/colliders, unpaused gameplay and a closed pause menu. Root independently read the results and inspected both screenshots.
- The example shared floor now sits at top -0.01 m, below the community park floor and above the base test-world floor. This removes the coplanar dark triangles without changing Rust behavior. The final course capture shows the clean floor and both named original robot presentations. All 17 focused package/course/park checks pass (`/tmp/skate-final-example-checks.log`).

## Continuation record and remaining acceptance

At the initial implementation handoff, the checkout was on
`cross-platform-dedicated-server` at baseline
`69ed3779342bd61ecff7f8d40628269152a79afc`; implementation is in local tracked
changes and new untracked source/assets. No commits, pushes, publication or
deployment occurred. Do not discard untracked files: they include the new
crates, resource examples, original GLBs, tests and documentation. Graphify
output and local database state remain ignored. This is a substantial implemented
Linux platform extension, not a claim that every requested acceptance is closed.

Completed work is indexed by the current matrix and
[implementation plan](../superpowers/plans/2026-10-04-platform-extension.md).
[Run instructions](platform-extension.md) include prerequisites and separate
server/client terminals. Matching freshly built game and server binaries are
required by the changed dedicated wire framing. The examples still require the
user's prepared character/animation assets; they contain no retail assets.

Remaining work, in dependency order:

1. Full articulated skating authority remains open. Course-v1 independently
   verifies its documented movement envelope, swept capsule, checkpoint, pickup
   and trusted entity-contact rules. It does not independently derive stock
   flips/grabs/grinds, landings or combo totals. Extending authority requires a
   headless deterministic skating/input contract and corresponding native
   reconciliation tests; cosmetic animation events must not become score proof.
2. Investigate `physics::wipeout_tests::marker_reply_restores_on_foot`, an
   additional ignored owned-asset test that fails at tick 61 (PhysicsAir versus
   PhysicsGround). The controlled old/new teleport-method comparison reproduces
   both failures; it does not establish that the whole branch is uninvolved.
   Preserve the existing assertions and owned-asset prerequisite.
3. Run native Windows browser, audio, managed worker, account/admin and filesystem
   integration, then a Windows/Linux mixed session. Requires an actual Windows
   host with WebView2, .NET worker build prerequisites, client assets and audio
   devices. Windows cross-compilation and edited CI files are not runtime proof.
4. Validate physical microphone/speaker capture and playback with two clients.
   Requires physical devices; deterministic ALSA capture/playback, Opus decoding,
   authenticated UDP proximity/radio/mute and lifecycle already pass.
5. Run the unchanged private Skyline integration suite after providing the owned
   `sdk/examples/skyline/skyline.glb` fixture. Do not substitute a different map or
   remove its assertions. Hosted CI remains unexecuted because no push was
   authorized.

Other known check failures are retained without unrelated repair: the full game
suite has one baseline-identical setup-cache fixture failure (358 pass, 105
ignored); the data all-target run encounters the unchanged `hud_data` example's
missing `scoring_hud` import, while data `--lib --tests` passes 40 tests. The
workspace-wide `cargo fmt --all -- --check` also reports formatting differences
(`/tmp/skate-platform-format-final.log`); it is not a passing check. No broad
format-only rewrite was applied.

Useful reproduction commands from the repository root:

```sh
cargo test --locked -p skate-net -p skate-data -p skate-resources -p skate-services -p skate-accounts -p skate-server -p skate-dynamics -p skate-voice -p skate-browser --features skate-voice/devices --lib --tests -- --nocapture
cargo test --locked -p skate-server --test competition
cargo test --locked -p skate-mods --lib --test resource_runtime --test animation --test physics_api --test validate_example
cargo test --locked -p skate-game --bin skate3rust
python3 -m unittest tools.test_package_server tools.test_linux_scripts tools.test_resource_park tools.test_presentation_demo tools.test_verify_resource_park tools.test_verify_resource_presentation tools.test_verify_resource_course
```

For actual managed execution, set `SKATE_DOTNET_ROOT` to an installed .NET 10
root and `SKATE_MANAGED_HOST` to the built trusted worker, then run
`cargo test --locked -p skate-mods --test managed_runtime -- --ignored --test-threads=1`.
This host used `/tmp/skate-csharp-sdk/dotnet` and `/tmp/skate-managed-host`.
Native scripts expose `--help`: `tools/verify_resource_park.py`,
`tools/verify_resource_presentation.py` and `tools/verify_resource_course.py`.
They require matching binaries and the existing prepared owned asset directory;
keep their captures outside the source/package inventory. The `/tmp` evidence
paths above refer to this development host and are not bundled deliverables.


### Final repository readback

- All 37 Python tooling tests pass after the final example change
  (`/tmp/skate-platform-python-final4.log`). The final documentation correction
  adds the missing presentation grant setup and documents the existing voice
  default plus `--voice` opt-in; all six ZIP/package tests pass again afterward
  (`/tmp/skate-package-final-docs.log`). All 59 resource inputs and 26 explicit
  documentation/declaration inputs are present, regular files and visible to Git.
- `graphify update .` succeeds after the final code/example changes
  (`/tmp/skate-graphify-final3.log`). It reports no further graph topology change.
  Nonfatal warnings remain: installed skill 0.9.42 versus package 0.9.44, and 24
  inputs producing no AST nodes. Generated graph output remains ignored.
- Final branch/HEAD and baseline ancestry checks pass. `git diff --check`, both
  edited workflow YAML parses, shell syntax for `BUILD.sh`/`PLAY.sh`, and local
  Markdown file-link checks pass. Dependency provenance covers all 193 newly
  introduced external crate versions. No whole-workspace test, formatting,
  native Windows or hosted-CI pass is claimed.
- Final independent evidence review agrees with the four native scenario result
  files and the recorded Rust/Python/capacity counts. Its missing presentation
  grant instructions and voice-default documentation findings are corrected.
  No build, verification client or delegated implementation is left running.

### Authorized local commit

The user subsequently requested local commits. The integrated platform changes,
original example assets, tooling, documentation and recorded limitations are
committed together because their build and packaging contracts are coupled.
Immediately before committing, all 11 competition tests and all 37 Python
tooling tests passed again (`/tmp/skate-precommit-competition.log` and
`/tmp/skate-precommit-python.log`); `git diff --check` also passed. This does not
supersede the broader run results or close any remaining acceptance gaps above.
No push, publication or deployment is part of this authorization.
