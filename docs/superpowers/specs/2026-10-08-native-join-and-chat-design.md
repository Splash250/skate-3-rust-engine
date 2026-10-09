# Native account joining and multiplayer chat

Date: 2026-10-08

Status: proposed design for user review; implementation has not started.

## Purpose and agreed requirements

Players select a community server, use a server-branded login/registration page
while permitted content loads, and enter gameplay once authentication, required
resources, and world/collision preparation are complete. Gameplay chat has native
input, delivery, and rendering, with server-established names and ranks and
bounded plugin contributions to presentation.

The supplied request establishes these requirements. The contracts and defaults
below are proposed engineering decisions, not APIs that already exist. Work stays
on `proper-map`, preserves existing map/interior/multiplayer changes, uses native
execution by the main agent, and ends with an independent implementation review.
There will be no commits, pushes, publication, or interference with unrelated
running processes.

The proposed first release covers dedicated servers, including guest-only
dedicated servers. Existing Steam/local free-skate lobbies keep their current
behavior. Whether to add chat to those lobbies in this release is a separate scope
decision submitted alongside this spec; choosing it requires extending the design
with lobby-host authority and transport tests before planning implementation.

## Verified current behavior

The graph query was used for navigation; the following findings were checked in
the current working-tree source, including its existing uncommitted changes.

| Existing implementation | Consequence for this design |
| --- | --- |
| `crates/skate-accounts/src/store.rs`: SQLite account store, Argon2 passwords, inherited roles/permissions, active verified sessions, transactional mutations and live permission refresh | Extend this authority; do not introduce another role or password store. |
| `crates/skate-accounts/src/admin.rs`: verified HTTPS `/v1/login`, private file-based client credentials, 16 KiB request ceiling, login rate limiting | Retain profile login; add public registration and an in-memory native login client. |
| `crates/skate-server/src/accounts.rs` and `lib.rs`: configuring accounts requires authenticated gameplay datagrams; session actor is checked against the packet | Preserve authenticated and guest-only server modes. This design does not introduce mixed authenticated/guest admission on one account-enabled server. |
| `crates/skate-game/src/multiplayer/dedicated_browser.rs`: discovery, favorites/profile paths, asynchronous profile login and cancellation | Replace the profile-required interactive join experience with a join coordinator; keep automation/profile compatibility. |
| `crates/skate-browser/src/lib.rs`, `main.rs`, `process.rs`: local packaged browser assets, bounded private IPC, denied external connections and permissions, optional composited surfaces | Reuse the browser host with a separate engine account owner and strict bridge. |
| `crates/skate-game/src/modding/browser.rs`: resource-owned pages send page events to VM callbacks; composited input forwards text/key edits, not a full IME editor | Never open the account page through a resource VM or route its requests through `on_event`. |
| `crates/skate-game/src/modding/resources.rs`: asynchronous content downloads, connection identity, cancellation, capability admission and lifecycle retirement | Prefetch only verified public content; retain normal admission before running client resources. |
| `crates/skate-game/src/map_transition.rs`: resource worlds remain unready until collision, rails, rendering and nearby streaming publish | Use this readiness signal for final gameplay activation; preserve interior movement and resource-world identity. |
| `crates/skate-game/src/input.rs`: gameplay is blocked for menus, map, locations, browser and interfaces | Add account/chat ownership to this contract and audit shortcut consumers. |
| `crates/skate-game/src/multiplayer.rs`: `mp:name` is a sanitized client-published name, currently limited to 16 characters | It is presentation input, not account identity. Establish canonical chat names at the server. |
| Searches of game/net/server/mod runtime and SDK sources found no native chat subsystem or chat API | All chat contracts below are new. |

The existing application transport has a 1,024-byte value ceiling and a
1,200-byte datagram MTU. Browser ceilings are 512 files, 8 MiB/file, 32 MiB total
assets and 16 KiB messages. Resource set validation already checks cumulative
bytes/files. None of those limits will be raised to accommodate this feature.

## Approach and boundaries

Use an engine-owned account page, an asynchronous native join coordinator, and a
native chat channel. Extend existing account, content, network and resource
lifecycle abstractions with focused modules.

Two alternatives were considered. A resource-owned account page would admit
executable plugins before authentication and expose credential requests to
ordinary callbacks. Browser-rendered chat would reuse HTML editing but would
conflict with the requested native renderer and make formatting executable.
Neither is selected.

Responsibilities are separated:

- `skate-accounts`: registration policy, invite consumption, account/session
  state, public TLS requests, safe session identity snapshots.
- `skate-resources`: immutable bounded pre-login package/public prefetch
  descriptors, allowlists, digests, cumulative content/cache checks.
- `skate-browser`: account-only bridge mode and browser lifecycle, maintaining
  network/OS restrictions.
- `skate-net`: versioned native chat codec, acknowledgement/retransmission,
  bounded queues and native reserved application records.
- `skate-server`: canonical chat identity, moderation, delivery, plugin ordering
  and the join bootstrap advertisement.
- `skate-game`: join state, loading/account presentation, native text editing,
  chat panel/settings, focus ownership and client plugin presentation.
- `skate-mods` and SDK: bounded versioned commands/callbacks with host-owned
  resource identity and generation.

New focused files are preferable to adding another subsystem inside the existing
large multiplayer/resource host files. Exact file decomposition and test names
will be established in the implementation plan after spec approval.

## Native join lifecycle

A monotonically increasing local attempt generation binds every worker, browser
page, content result, and transport result to the selected UDP endpoint, advertised
server session, and bootstrap revision. Discovery is untrusted navigation data.
The generation is a lifecycle identifier, not an authentication credential.

The phases are `Idle`, `Preparing`, `Account`, `Authenticating`, `Admitting`,
`PreparingWorld`, `Playing`, `Failed`, and `Cancelled`. Content preparation is a
parallel lane and is not represented as successful authentication. An account
failure returns to `Account` with a safe error while valid public prefetch may
continue. Cancelling, leaving, switching servers or restarting an attempt retires
the old generation immediately and requests cooperative worker cancellation.
Completed results from retired generations are discarded, including successful
logins and prepared worlds. Workers are polled; no network joins or password
hashing block the frame/simulation thread.

At most one current attempt and two retiring bounded workers are retained.
Starting another attempt when retirement capacity is full reports busy; it never
spawns unbounded workers. Auth/bootstrap operations have a 10-second overall
deadline; normal content transfer retains its existing deadlines/cancellation.
Dropping an attempt closes its page, releases its cache pins, discards its keys
and queued results, and best-effort revokes its issued session through native TLS.
A revoke failure cannot activate gameplay and is bounded by normal session expiry.

Gameplay opens only when all conditions hold for the same live attempt:

1. Verified authentication succeeded, or this is a guest-only server.
2. The canonical admitted server and resource revision match the attempt.
3. Required client resources completed existing capability/admission checks.
4. The world/collision/rails/render readiness gate succeeded, including required
   interior catalogs and nearby streaming preparation.

Content prepared for a retired attempt must not commit a map swap or mark another
connection ready. Existing interior teleports and their movement epochs remain
separate from joining. In-game reauthentication performs a controlled leave and
fresh join; it does not replace a transport codec under an existing actor.

## Bootstrap and pre-login content

Add a versioned native bootstrap descriptor outside ordinary resource admission.
It describes the account policy, optional UI manifest, and explicitly public
prefetch content. It contains no private settings, server scripts or credentials.
Discovery advertises only bounded bootstrap location/version data and continues
to fit its current 1,200-byte response ceiling; omission produces the default UI.

Account-enabled servers serve bootstrap and account UI assets through their
verified TLS account origin. The existing configured CA and hostname validation
remain required. A saved server entry can carry a trust-only configuration with
account origin and CA path; interactive joining no longer requires a password
file. Existing full account profiles continue to work. A missing trust
configuration produces a native setup error and allows selecting a local trust
file; discovery or page JavaScript cannot install trust, change the native
account origin, bypass certificate checks, follow redirects or use proxies.

Guest-only servers can provide a public package through the existing content
provider. Native chrome identifies such a server as a guest connection; the page
cannot enable password submission or claim a verified account identity.

The pre-login UI is a declarative allowlist of HTML/CSS/JS, fonts and images. It
has no executable resource entry points or resource capabilities. Proposed limits:
64 files, 1 MiB/file, 4 MiB total assets, 64 KiB manifest, one account surface.
Validate MIME, normalized paths, duplicate entries, symlinks, declared and actual
sizes, complete digests, and total memory before opening it. Publish an immutable
snapshot; do not reread mutable files during an attempt.

UI bytes and prefetched content count toward the existing configured total/cache
budgets, including retained/pinned generations. Decoded texture/world limits and
browser frame allocations also retain their existing aggregate checks. A package
that fits individual file limits but exceeds its cumulative budget is rejected.
Do not add per-transfer loopholes or increase existing limits.

Public prefetch downloads/validates permitted map/assets while the page is open,
but does not execute scripts, mount gameplay content, acknowledge admission, or
transfer owned retail assets. After login the authenticated offer must match
before prepared results are reused; a changed revision discards/revalidates them.

Missing/invalid UI, digest mismatch, startup failure, page crash or failure to send
`ready` within five seconds selects the packaged default page and reports the
reason in native status. If the browser host itself is unavailable, native account
fields using the same editor/controller provide login, registration, guest,
retry and cancel. Native Escape/cancel always works independently of JavaScript.

## Account bridge and public registration

Account page mode exposes only version 1 requests: `ready`, `login`, `register`,
`guest`, and `cancel`. Login carries username/password; registration additionally
accepts an optional invite. Native responses contain policy, safe result/error
codes and separate authentication/content/world progress. Each request has a
bounded request ID; the host binds it to page/attempt generation and selected
origin. Unknown fields/actions, wrong generations and excess pending requests
are rejected. Only one auth request can be pending per attempt.

The browser necessarily sees what is typed into its forms. Custom JavaScript is
therefore server-authored account presentation, with the existing CSP denying
external network, forms, frames, workers and OS access. It receives neither issued
tokens nor transport keys. Page requests go directly to the engine account
controller over private bounded IPC, never to resource callbacks, general plugin
events, chat, telemetry, crash transcripts or logs. Errors are fixed safe messages
and never echo request bodies or submitted passwords. Clear sensitive form/native
buffers on submit completion, navigation, failure retirement and close; redact
sensitive types from debug formatting. No password persistence is added.

Retain existing username validation (3–32 lowercase ASCII letters/digits/`_`/`-`)
and password validation (12–1,024 UTF-8 bytes). Unicode chat/display text does not
change account-key normalization. Sessions stay in native memory; reconnection
creates a new codec with fresh counters.

Public registration policy is `disabled` by default, `enabled`, or
`invitation_only`. Add a TLS registration endpoint and native client operation.
Administrative account creation remains separate and permission-protected.
Registration never accepts roles, grants, account IDs, bans or whitelist flags.
It creates an unprivileged account; server administrators assign existing roles.
Whitelist mode continues to apply, so successful creation can still return
`awaiting_approval` rather than immediate gameplay access.

Invites are administrator-issued 32-byte random secrets stored as hashes, with
expiration and one use. Default lifetime is 24 hours, with an administrator-set
maximum of 30 days and at most 1,024 outstanding invites. Creation, invite
consumption and audit insertion occur atomically;
concurrent duplicate names or invite use cannot create extra accounts. Support
revocation and enforce bounded invite storage. Audit policy/invite changes and
registration outcomes without passwords, tokens or invite values.

Share the current login rate ceiling (10 auth attempts/IP/minute, 1,024 tracked
IPs) across login and registration, plus a registration ceiling of 3/IP/minute.
Apply bounded worker concurrency before Argon2 work. Errors distinguish duplicate
username, policy disabled, invalid/expired invite, rate limiting and storage
failure without exposing secret request data. Registration success returns to
login with username retained and password cleared; login remains a separate
native action.

## Native chat delivery and identity

Add chat protocol version 1 with explicit capability negotiation. It runs without
installed resources. Keep ordinary resource event transport separate. Reserved
native client/server records carry a small acknowledged stream; retransmit the
current outstanding event until acknowledged, then advance. Encode validated
records beneath the existing 1,024-byte application ceiling, fragment bounded
larger styled events, and keep every packet below the unchanged 1,200-byte MTU.
Reassembly has count/byte/deadline limits. Tests must exercise loss, duplicate and
reordered packets. Unsupported peers get explicit unavailable status, never
silently successful delivery.

Client submissions contain attempt/connection incarnation, client message ID,
channel ID and text only. The server derives sender actor/account/name/roles from
the admitted connection; it rejects submissions for absent, stale, unready,
revoked or disconnected actors. A server event gets a server sequence ID,
timestamp, canonical sender, identity revision, structured style and text.
Deduplicate by connection incarnation plus message ID and acknowledge acceptance
or a safe rejection. An acceptance ACK means the server accepted the message;
local pending/error presentation never impersonates a delivered server event.

The default channel is `global`, across world instances on the same server.
Proposed message bounds are 512 UTF-8 bytes and 256 Unicode scalar values after
single-line normalization. Reject empty text, malformed UTF-8 and disallowed
controls; remove terminal control sequences and unsafe directional overrides.
Do not parse HTML, Lua, JavaScript, URLs or color escapes as executable content.
Native structured spans are limited to 16, badges to 2, and serialized delivered
events to 4 KiB. Total formatted text cannot exceed the message limit.

Use a 5-message burst with refill of one message/second per connection; commands
share this quota. Retain at most 32 queued events/64 KiB per connection and a
bounded 512-entry/1 MiB server ring. Slow recipients receive an explicit gap/resync
notice rather than unlimited queues. A resync is bounded and cannot duplicate
events already displayed. No persistent chat archive is introduced.

Account chat names default to the verified account username. Guest chat names
are sanitized from the admitted player's name proposal, with a native guest
marker and actor disambiguation. A name proposal never establishes account or
rank authority. Preserve current map/nametag behavior initially while exposing a
canonical chat identity lookup for future consistency.

Extend verified session snapshots with safe current username/direct role IDs and
an identity revision, refreshed alongside the existing live permission mutation
path. Cosmetic rank resolution maps those existing roles to presentation. Choose
the highest configured presentation priority, tie-breaking by role ID. With no
configured style, display the selected role ID plainly; unassigned players have
no fabricated role. Plugins can contribute server-approved cosmetic rank metadata
for guests or accounts but cannot grant permissions. Role removal, bans/revocation
and permission changes apply before the next accepted message/command. History
retains the identity snapshot at delivery time.

## Native panel and text input

Provide a compact lower-left translucent panel with separate badge, name and
message spans, readable line spacing, six recent lines and a smooth idle fade.
Typing expands to twelve lines and displays input, selection/caret, command
suggestions, channel and pending/error feedback. Use native Bevy text/layout;
never render chat in a webview. Keep 512 entries/1 MiB of client scrollback, prune
oldest entries, preserve a stable scroll anchor, and show unread count while
scrolled away from the bottom. New messages do not force the user to the bottom.

Open with T, or `/` with a command prefix; Escape closes and keeps the non-secret
draft for the current connection, Enter submits, Shift+Enter does not introduce
multiline messages. Tab cycles suggestions; arrows/history navigation do not
steal caret movement while text is being edited. `/login` and `/register` are
reserved native actions opening the account UI. If extra arguments follow them,
reject locally, clear that draft and show a safe explanation; never transmit,
record or print password-bearing versions of those commands. Guest-only servers
report account actions unavailable. Plugins cannot shadow these reserved names.

Implement a reusable native single-line editor with grapheme-aware cursor/editing,
selection, pointer caret placement/drag selection, Home/End, word movement,
Ctrl+A/C/X/V, paste sanitation and bounded Unicode text. Route OS IME preedit and
commit separately, show preedit/candidate position, and do not submit Enter while
composition is active. Enable IME only for a focused editable field and clear
composition on owner loss. Use native OS clipboard access only on explicit editor
actions; browser/plugin code gains no clipboard capability. Any additional
grapheme/clipboard dependency must be justified and recorded in the plan.

Add a common native focus owner for account/chat, integrated with current browser,
menus, replay, map, debug camera and location interaction ownership. Input routing
runs before conflicting menu/map/shortcut consumers and before fixed gameplay
publication. Opening chat releases/blocks skating actions without changing the
Input → Controls → Physics schedule. Closing, window blur, disconnect, menu
takeover or server switching releases ownership, clears composition/edges and
requires held activation/control buttons to be released before reuse. Privacy
account focus takes precedence over chat and resource pages. Tests cover both
keyboard shortcuts and controller gameplay; inspect voice/PTT conflicts too.

Persist local player settings for font size (14–24 px, default 17), panel opacity
(0.2–0.9, default 0.65) and effects enabled (default true). Disabling effects removes
animated plugin effects and idle animation while retaining readable history.
Settings use the existing local settings conventions, never server permission
state. Support scale/resolution changes and long names without losing the input.

## Versioned plugin contract

Introduce `chat.v1` schemas with capability grants in existing API-2 Lua resources
and supported client mod runtimes. Update wrappers, Rust command/output schemas,
host handlers, SDK declarations, documentation and examples together. Existing
`sdk`/resource APIs stay compatible. Managed adapters use the same host schemas
and generation checks rather than inventing a separate authority path.

Server APIs register commands/channels, publish styled announcements, register
rank/name styles and bounded effects, and contribute moderation/formatting hooks.
Client APIs register local commands and local presentation preferences/effects.
Client plugins cannot change canonical sender/rank, publish accepted remote
messages, authorize channels or replace server moderation. Custom server rank
plugins use an explicitly configured provider allowlist and cosmetic presentation
priority; no automatic role authority is inferred from a plugin name.

The proposed new capabilities are `resource.chat`, `resource.chat.moderate`, and
`resource.chat.rank`. All require declaration plus host grant. Separate local client mod
capabilities authorize local commands/styles. An installed plugin is not granted
account secrets or a pre-login callback.

Pipeline order is native validation/authorization/rate limit → server moderation
→ server format contributions → native final validation → sequencing/delivery →
bounded local client presentation. Every hook observes the immutable original
message plus accumulated allowed presentation; server plugins cannot rewrite its
sender, ID or user text. Moderation can deny with a safe bounded reason. Native
permission checks remain authoritative and are repeated before execution.

Sort hooks by ascending configured priority, then resource ID, then registration
ID. Later contributions replace only explicitly supplied style fields; spans
are validated replacements, not appended without bounds. Command/channel name
conflicts reject the later registration using the same ordering and produce a
bounded owner diagnostic. Reserved native commands always win. Registration
limits are 64 commands, 16 channels and 32 hooks server-wide, with at most 16
commands and 8 hooks per owner. Existing VM instruction/memory/output budgets
apply to callbacks and their combined outputs.

Effects are finite native presets (`pulse`, `glow`, `shake`) with validated color,
intensity and duration up to two seconds; no arbitrary shaders, markup or scripts
are embedded in messages. Players can disable all effects.

Registrations and queued outputs carry host-owned owner/generation handles.
Unload, restart, failure or client retirement removes registrations, suggestions,
channels/styles/effects and invalidates pending outputs. Late callbacks cannot
publish, format or duplicate messages in the replacement generation. Failed or
malformed formatting falls back to canonical plain presentation. A failed
moderation hook rejects that submission safely and reports the owner failure;
it cannot bypass native permission checks. Continue native chat after retiring
the failed plugin under the existing runtime failure policy.

Deliver a runnable API-2 example demonstrating `/wave`, an announcement, existing
role-based rank/name colors, and an optional pulse effect. It must use supported
new APIs, include grants/setup instructions, and have callback behavior tests in
addition to manifest/syntax validation. Add a default account package and a
customization example that contain no real credentials or private content.

## Verification and delivery acceptance

Implementation is executed inline after the approved spec and implementation plan.
Use focused failing regression tests before changing behavior. Validation must
include actual selected tests, not a zero-test filtered run.

| Area | Required evidence |
| --- | --- |
| Account lifecycle | Verified TLS login; malformed/wrong cert failures; no secret leakage into events/errors/debug; session expiry/revoke; old profile compatibility. |
| Public registration | All policies, duplicate/concurrent names, atomic invite replay/expiry/revoke, unprivileged roles, whitelist behavior, audit atomicity and auth throttling. |
| Joining | Auth/content completion in both orders; cancel, retry, disconnect, switching servers/session/revision; late worker/page/world result rejection; guest join; default and native fallback; gameplay readiness predicate. |
| Content | Aggregate UI/set/cache/queue limits at boundaries; decoded limits; digest/symlink/path checks; zero pre-login VM execution; no limits increased. |
| Chat | Two admitted clients, authoritative name/rank, forged identities, stale actors, Unicode bounds, rate limits, permissions/moderation, delivery ACK/error, loss/reorder/duplicates, scrollback/gap resync, reconnect. |
| Editor/focus | Grapheme edits, caret/selection, clipboard/paste, IME preedit/commit, submit during composition, blur/close/held key/controller restoration, conflicting map/menu/debug/PTT shortcuts. |
| Plugins | Deterministic conflicts/style ordering, bounded spans/effects, exceptions, output budgets, unload/reload/stale generations, client restrictions, live role changes, runnable example. |

Check the affected account/net/resources/mods/server packages and the game binary
with the repository's locked scoped Cargo commands. Use existing synthetic
fixtures for automated tests. Run live singleplayer/UI checks and two clients on
a test-world dedicated server where the actual graphics/assets prerequisites
permit. Exercise branded/default login during loading, registration, both clients
sending messages, command/styling/effects, reconnect and permission/rank changes.
Test actual clipboard/Unicode/IME on the available desktop; distinguish automated
IME event tests from real OS composition. Do not claim platform coverage not run.

Capture real running-engine screenshots of login/loading and native chat, retaining
no visible credentials. Screenshots of HTML mockups do not satisfy this evidence.
Obtain an independent final review with read-only ownership after implementation;
resolve blocking findings and rerun affected checks. Run `graphify update .` after
code changes and report its result. Documentation alone does not require a graph
rebuild or game build.

Final delivery identifies the implemented API surfaces versus old APIs, documents
server/client configuration and limits, includes examples and screenshot links,
and reports checks and unavailable live prerequisites candidly. Nothing is
committed or published. Spec approval is followed by a concrete written plan for
review; native inline execution is already the user's selected execution method.
