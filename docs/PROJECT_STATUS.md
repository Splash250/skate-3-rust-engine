# Community multiplayer platform status

Snapshot: 2026-10-08. This fork extends the Skate 3 Rust/Bevy engine into a
community-hosted multiplayer and scripting platform. It preserves the upstream
engine, research credits, Git history and GPL-3.0-only licensing.

The development baseline is `4442b5433331f9e61c0bdd4453734bbff61a21bc`,
originally on `proper-map` and `cross-platform-dedicated-server`. Relative to
upstream `b3c967932a91d0db74275d14064bb6bdef8608af`, it contains 15 additional
commits affecting 513 files, with 97,587 insertions and 591 deletions. These
counts include documentation, tests, fixtures and dependency-lock changes;
they are not a measure of production readiness.

## What comes from upstream

The Rust/Bevy game, reverse-engineered skating and articulated physics,
tricks/scoring, grinds, offboard movement, rendering, animation, audio, asset
preparation, map formats and existing mod/lobby support form the foundation.
Gameplay parity with the original game remains incomplete. See the
[history and credits](../README.md#history) and [license](../LICENSE).
The workspace contains 17 Rust crates; the platform additions build on the
existing game rather than replacing it.

No retail game assets are distributed. Clients need their own prepared assets,
including character and animation data even when using an original example
map. Third-party code retains its existing notices and licenses.

## Implemented platform capabilities

“Implemented” means a capability is present in the committed development
baseline. Recorded checks below describe specific tested contracts, not a new
verification of this snapshot or a guarantee across every platform.

| Area | Implemented behavior | Guide and evidence |
| --- | --- | --- |
| Native Linux | Build/launch scripts, owned-asset preparation, platform integration and optional Steam relay | [Linux setup](LINUX.md) |
| Dedicated hosting | Headless UDP server, compatible-data admission, movement/body/pose replication, shared collisions and validated shoves; default 16 slots, configurable to 64 | [Hosting](multiplayer/README.md), [capacity evidence](multiplayer/production-validation.md) |
| Resource distribution | Server-selected manifests, digest-verified content/cache, dependency handling, capabilities, lifecycle and retirement | [Resources](multiplayer/resources.md), [SDK](../sdk/RESOURCES.md) |
| Script runtimes | Lua, embedded QuickJS, isolated .NET worker and cross-language exports with runtime limits | [Platform extension](multiplayer/platform-extension.md), [examples](../resources/README.md) |
| Shared worlds | Server-owned shared entities, ownership/state, instances, teleports, downloadable original parks and world cleanup | [Entities](multiplayer/shared-entities.md), [worlds](multiplayer/resource-worlds.md) |
| Competition | Course validation and opt-in native-input-v1 solver/scorer authority with client prediction and reconciliation | [Competitions](multiplayer/verified-competitions.md), [native authority](multiplayer/native-skating-authority.md) |
| Accounts and administration | Optional persistent identities, permissions, roles, bans, TLS login, protected UDP sessions and authenticated administration | [Accounts](multiplayer/accounts-and-administration.md) |
| Backend services | Asynchronous SQLite/HTTP, transactions, namespaced persistent data and bounded results | [Services](multiplayer/backend-services.md) |
| Browser UI | Separate browser companion, resource HTML interfaces, in-game compositing, input focus, containment and lifecycle | [Browser interfaces](multiplayer/browser-interfaces.md) |
| Voice | Opt-in CPAL/Opus, proximity/radio routing and resource-owned call channels | [Voice](multiplayer/voice.md) |
| Resource presentation | Custom animation/characters, attachments, markers and owner-scoped cleanup | [Presentation example](../resources/presentation-demo/README.md) |
| Interaction and phone | Master interaction menu, resource actions, phone themes/apps, accepted-only player calls, photo capture/gallery and permission-controlled dashboards | [Interactions](multiplayer/resource-interactions.md), [evidence](multiplayer/resource-interactions-evidence.md) |
| Operator tools | Resource settings/profiling, process supervision, maintenance/drain, recovery and administration | [Settings](multiplayer/resource-settings.md), [profiling](multiplayer/resource-profiling.md), [operations](multiplayer/server-operations.md) |
| Discovery and packs | Saved endpoints/favorites, server previews and admission queues; pinned recipes with install/verify/rollback | [Discovery](multiplayer/dedicated-discovery.md), [packs](multiplayer/server-packs.md) |
| Creator tools | Visual placement/transforms, snapping/history, rails/markers, deterministic export and local playtesting | [Visual authoring](multiplayer/visual-authoring.md) |
| Reusable gameplay | Account profiles, crews, rounds, map voting, tournaments and verified leaderboards | [Gameplay platform](multiplayer/gameplay-platform.md) |
| Roleplay showcase | Boardwalk Borough original town, phone, persistent economy, apartment leases/guest access and server-owned pizza jobs | [Showcase and acceptance](multiplayer/boardwalk-borough.md) |

The default dedicated transport and optional account-enabled platform are
separate configurations. Do not infer authentication or encryption for an
anonymous direct-connect session from the account-enabled feature set.
Likewise, ordinary replicated scores are presentation data; trusted competition
results require the relevant server verifier.

## Architecture and source navigation

| Component | Responsibility |
| --- | --- |
| [skate-game](../crates/skate-game/src) | Bevy client, native gameplay, dedicated client integration and resource presentation |
| [skate-net](../crates/skate-net/src) | Dedicated protocol, admission, replication and bounded transport |
| [skate-server](../crates/skate-server/src) | Headless hosting, shared authority, resources, competition and operations |
| [skate-resources](../crates/skate-resources/src) | Content manifests, verification, distribution and cache |
| [skate-mods](../crates/skate-mods/src) | Mod/resource host APIs, script runtimes and limits |
| [skate-services](../crates/skate-services/src) | Backend database and HTTP services |
| [skate-accounts](../crates/skate-accounts/src) | Account store, transport and administration |
| [skate-browser](../crates/skate-browser/src) | Browser companion and validated client bridge |
| [skate-voice](../crates/skate-voice/src) | Audio capture/playback and voice codec integration |
| [skate-platform](../crates/skate-platform/src) | Operating-system integration |
| [skate-steam-relay](../crates/skate-steam-relay/src) | Optional isolated Steam transport helper |
| [Resources](../resources/README.md) and [SDK](../sdk/RESOURCES.md) | Original runnable examples and public authoring contracts |

The [domain vocabulary](../CONTEXT.md) defines Hybrid Authority: the server
owns canonical shared state while clients predict detailed skating. The
[native authority guide](multiplayer/native-skating-authority.md) explains the
bounded opt-in input simulation; it does not establish exhaustive retail parity.

## Recorded validation and remaining acceptance

The detailed ledgers are the authority for which scenario passed and under
which prerequisites:

- [Platform extension ledger](multiplayer/platform-extension-evidence.md):
  resource/network/services integration, two native Linux clients, original
  maps, presentation, accounts and bounded native skating.
- [Operations and creator ledger](multiplayer/platform-capabilities-evidence.md):
  settings, profiling, supervision, packs, discovery, authoring and reusable
  gameplay integration.
- [Interaction ledger](multiplayer/resource-interactions-evidence.md): actual
  Linux browser surfaces, phone/calls/photos, permissions and teardown.
- [Production workload](multiplayer/production-validation.md): a bounded
  120-second impaired loopback workload with 64 simulated owners, resources and
  eight Opus talkers; this is not 64 graphical clients or a WAN deployment.
- [Boardwalk Borough acceptance](multiplayer/boardwalk-borough.md#acceptance-evidence):
  automated resource/pack checks and a short account-enabled headless startup.
  The integrated showcase's two-client graphical acceptance has not run.

Open work includes native Windows and mixed Windows/Linux acceptance, physical
microphone/speaker/controller acceptance, longer native multi-worker and
multi-host WAN workloads, exhaustive trick-family coverage and shared dynamic
native authority. Existing Linux phone acceptance does not establish acceptance
of the later complete Boardwalk Borough experience. Some historical evidence
refers to private assets or temporary local logs which are not shipped here.
CI definitions are not evidence of successful hosted CI runs.

## Local work excluded from this baseline

At the time of this snapshot, the working tree also contained uncommitted
programmable map/hologram code, map-view and resource API integration, SDK map
documentation/examples, input and marker changes, and planning documents for
downtown apartment entry. These are active local work, not published completed
features. Their presence in a developer checkout does not mean they exist in
the fork's committed baseline. Validate and publish them as separate changes.

## Build, host and contribute

1. Follow [Linux setup](LINUX.md) or the upstream Windows build instructions in
   the [README](../README.md#build) and prepare your own game assets.
2. Start with the [dedicated server guide](multiplayer/README.md). It includes
   procedural-world hosting and direct connection commands.
3. Add [resource examples](../resources/README.md), then optional accounts,
   browser companion, voice or managed runtime prerequisites as needed.
4. For the complete RP example, follow [Boardwalk Borough](multiplayer/boardwalk-borough.md).
5. Use the focused validation commands in each guide and the scoped
   [repository contributor instructions](../AGENTS.md). Keep private assets,
   credentials, player stores and generated captures outside commits.

## Fork and release policy

[The fork](https://github.com/Splash250/skate-3-rust-engine) preserves ancestry
from [upstream](https://github.com/SK8-ENGINE/skate-3-rust-engine). Its `main`
is the platform baseline; future work can be reviewed in focused branches.
Upstream engine improvements can be integrated without discarding either
project's history or credit.

This initial publication is source only. Upstream release downloads do not
contain the platform additions. The Windows packaging script still embeds
`SK8-ENGINE/skate-3-rust-engine` in release metadata; configure and verify fork
release/updater identity before distributing binaries. Review inherited
release automation before enabling it for the fork. No fork binary release or
successful fork build is claimed by this document.
