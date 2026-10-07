# Boardwalk Borough roleplay showcase

## Purpose

Build a small, playable roleplay server that demonstrates how dedicated servers,
downloadable resources, native voice, accounts, persistent data, map authoring,
and plugin UIs work together. This is a coherent showcase slice, not a complete
live-service roleplay game.

The intended session loop is: join a custom skate-town, meet and talk to other
players, use the phone to call or find work, deliver pizza for account-bound
money, and use that money to get an apartment that can be visited with a friend.
Players can return to the plaza to skate and socialize.

## Existing foundation

Reuse the current resource and server APIs wherever possible:

- `phone` and `phone-calls` provide the in-game phone, app registry, and
  two-party call signaling.
- `voice-room` and native voice provide instance-scoped proximity/radio policy,
  local opt-in, physical push-to-talk, and local mute/deafen.
- `platform-profiles` provides verified account identity. `inventory-ui` is a
  reference for authenticated purchases and resource-owned SQLite transactions.
- The server resource host provides version-pinned content delivery, events,
  scoped state, settings, commands, database services, voice controls, shared
  entities, and approved teleports.
- `tools/resource_park.py`, the park editor, and resource-world support provide
  the authoring route for an original town map, spawn pads, and interaction
  markers.

The current `rp-server.json` is a starting point for grants and resources; the
showcase pack must explicitly ensure all resources it needs. Its contents must
not be assumed to be enabled merely because they have grants.

## Player experience

The original map uses the working title **Boardwalk Borough**. It contains a
central skate plaza, a pizza shop, a compact apartment block, and several
delivery addresses. A small number of visibly labeled interaction markers
connect the locations. Interiors use separate instances so residents and invited
guests can share a room without exposing it to the street population.

The phone keeps its existing contacts and calling behavior and gains showcase
entries for Jobs, Properties, and local map/dispatch information. A pizza shift
assigns a pickup and a short sequence of dropoffs. Job state is visible on the
phone and through world markers. Completing deliveries credits the same wallet
used for property transactions. Property pages list available units, report
ownership/access, and provide entry/exit actions. An owner can grant a friend
access for the current property generation.

Proximity voice is configured to a short local radius (12 metres for the initial
demo). Same-instance radio supplies a pizza dispatch channel. Calls and radio
continue to require the engine's normal `--voice` opt-in and physical V/LB
push-to-talk; resources cannot override local mute or deafen.

## Resource boundaries

### Town world and marker data

Create one original required-world resource for the showcase map. Geometry,
collision, rails, spawn pads, and marker records are authored as redistributable
content and installed through the pinned server pack. Marker IDs and positions
are the shared source of truth for the map and the job/property scripts. The
server selects only the configured/allowlisted world; only one required world
is active at a time.

### Shared economy

Add a server-only `rp-economy` resource that owns persistent balances and a
transaction ledger keyed by verified account identity. It exposes a narrow
dependency API for balance reads, charges, and credits. Purchases and job
rewards use unique operation IDs and database transactions so retries cannot
charge or pay twice. Resources must not access another resource's database
files directly or trust account IDs supplied in client payloads.

The demo uses operator-configured prices and rewards with a modest starter
balance. It does not add a tradable player marketplace, player-to-player cash
transfers, debt, or a broad economic simulation.

### Properties

Add `rp-properties`, depending on `rp-economy` and `platform-profiles`. It owns a
small configured set of units, account-bound ownership/lease records, and
property access grants. A purchase or lease must charge successfully before
ownership is activated. To recover safely across separate resource databases,
the resource records a pending purchase ID, calls the economy with that same
idempotency key, then records ownership after success. On restart, it retries
or reconciles that operation before allowing another purchase. Entry requests
resolve to a predefined unit destination and instance; clients cannot submit
destination coordinates. Owners may invite guests, and access checks use the
authenticated caller. Exiting, disconnecting, resource retirement, or
invalidated travel leases return a player through the host's normal
teleport/admission path.

### Pizza work

Add `rp-pizza`, depending on `rp-economy`, `platform-profiles`, and the town
world's marker contract. Each player may have one active shift. The server owns
the job phase, assigned order, required pickup/dropoff order, expiry, and reward
operation ID. It advances only after validating the actual admitted sender,
current job, instance, and latest accepted player observation near the expected
marker. Duplicate, stale, out-of-order, implausibly fast, disconnected, or
wrong-instance requests do not produce a payout. Cancellation and timeout clear
the active job without charging or paying.

Movement observations still originate from the owning client in this server
model. Distance and timing checks discourage ordinary mistakes and simple
replays, but do not make job movement cheat-proof. The showcase must describe
this limit and keep rewards demonstrative rather than high-stakes.

### Phone and voice integration

Extend the existing phone through its registered app contract; do not give the
HTML page direct authority over money, property, or job state. Phone actions are
requests to client resource scripts and then authenticated server handlers.
Authoritative results return through scoped resource state/events.

Configure the trusted voice resource with the proximity radius and dispatch
membership policy. Radio never crosses instance boundaries. Phone calls continue
to use `phone-calls`, and every resource-owned voice channel is removed on
hangup, retirement, or restart.

## Persistence and trust

Persistent player data uses verified account IDs; connection actor IDs are
temporary and are used only to address live players. Server resources own all
prices, balances, property records, job phases, allowed marker IDs, teleporter
destinations, and permissions. Clients request named actions and render returned
state. They never choose a reward, claim a purchase, name another sender, or
provide a teleport destination.

Each resource receives only the capabilities it needs. Server scripts,
databases, account files, and private settings remain outside downloadable
content. Public map/UI assets are explicitly listed and digest-pinned in the
server pack. Custom clothing catalog integration is deferred; downloaded GLB
skins and native mix-and-match clothing items are different pipelines, and the
latter still needs dedicated-server outfit integration.

## Acceptance evidence

The implementation plan must produce evidence for all of the following:

1. A clean pinned server pack installs and validates the town and resources;
   missing, altered, or incompatible content blocks admission with an
   actionable error.
2. Two verified players can see each other, use proximity voice within the
   configured radius, fail to hear one another outside it, use dispatch radio
   in the same instance, and complete a phone call. Physical microphone and
   speaker operation is checked on real devices.
3. A player can complete a pizza order and receive one persistent payment.
   Replaying completion, completing out of order, changing instance, or
   disconnecting cannot create an extra reward. Balance and ledger survive a
   server restart.
4. A player can acquire an apartment, enter its predefined interior instance,
   invite a friend, reject an uninvited player, and exit safely. Property access
   and account ownership survive a resource/server restart.
5. The core scenario is exercised with two clients on Linux and Windows, then
   in a real WAN session. Record host/client OS, transport/account mode, actual
   voice-device result, and any network limitations. Loopback tests alone do
   not satisfy cross-platform or WAN acceptance.

Automated resource-host/database tests cover authority, idempotency,
reconnection, stale generations, timeouts, instance isolation, and persistence.
Graphical acceptance covers phone navigation, map markers, apartment visibility,
and handoff between the browser and game controls. Where Windows hardware, WAN
endpoints, or audio devices are unavailable, report the missing evidence rather
than marking that acceptance item passed.

## Out of scope for this showcase

- A complete collection of occupations, police/criminal systems, player trading,
  factions, moderation appeals, or a large persistent city.
- Full movement anti-cheat or claims that client-owned skating is server
  simulated.
- A custom clothing storefront that installs individual shoes, pants, or other
  modular retail parts. This can follow after a server-selectable outfit catalog
  and dedicated appearance synchronization are designed.
- Retail game assets or private account data in the public resource pack.
