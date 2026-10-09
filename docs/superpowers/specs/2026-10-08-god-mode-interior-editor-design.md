# Shared scene editor: interior authoring, plugin tools and future home decorating

Status: proposed written specification; conversational direction approved,
written specification awaiting review. Date: 2026-10-08.

## Intent and scope

Creators should configure interiors visually without measuring or typing world
coordinates. They move a free camera through the main map and an interior,
place the two markers and arrival/return anchors, and connect a new building or
add a floor to an existing building. Movement is manual and independent of
skating. Plugins supply placeable objects and editing interfaces in the same
editor.

The user selected local authoring first. The longer-term destination is a live,
invisible observer/admin dashboard: authorized admins can edit the world while
players continue playing, and plugins participate in that workflow. This first
release implements the reusable local editor and extension contract, not remote
administration or live multiplayer mutation.

The user also approved reusing the placement UI for players furnishing their own
homes with purchased objects. This is a third mode of the shared editor, with
home ownership and inventory rules rather than creator/admin privileges. Home
decorating, purchasing and inventory integration remain later-phase features.

Success means a creator can load a prepared interior, configure a working
round trip with the UI, save a standard native catalog, reopen it, and use it
without an editor plugin running. A sample plugin must demonstrate placing and
editing a teleport entrance using the same tools as native authoring.

## Approach and existing implementation

Use a native editor shell with a plugin tool registry and transactional draft
documents. Alternatives considered were an apartment-only wizard, which would
duplicate interaction machinery for later plugins, and a complete live admin
editor immediately, which would mix local authoring with unimplemented remote
permissions and concurrent editing. The shared shell with a local backend is the
selected approach.

Keep the reusable editor shell independent of the God Mode label. Mode-specific
entry points, palettes, camera bounds and command policies determine what a user
can see and change:

| Mode | Editable scope | Release |
| --- | --- | --- |
| Interior Setup | Local package entrances, floors and anchors; plugin tools | First |
| Live Admin / God Mode | Server-authorized world and player operations | Later |
| Decorate Home | Owned furnishings in the player's authorized home | Later |

All modes reuse selection, placement previews, move/rotate handles, snapping,
property controls and undo presentation. The session backend decides whether a
command is allowed and how it is persisted. Hiding a tool in the UI is not an
authorization check. Local simulation pausing is an Interior Setup behavior,
not an assumption built into the shared camera or tools.

Existing behavior verified in current code:

- [Native locations](../../../sdk/LOCATIONS.md) load installed, mod and server
  catalogs; own markers, travel, floor choice and interior minimaps.
- [Catalog types](../../../crates/skate-resources/src/locations.rs) already store
  interior transforms, spawn/facing, exit, entrances, return/facing and floor
  references. Keep the existing version-1 runtime format.
- [Catalog preparation](../../../crates/skate-game/src/locations/catalog.rs)
  composes collision and checks support, standing clearance and return distance.
- [Camera presentation](../../../crates/skate-game/src/camera.rs) already
  arbitrates gameplay, replay, manual camera and customiser presentation.
  [Debug camera](../../../crates/skate-game/src/debug_cam.rs) provides existing
  manual-camera and gameplay-input suppression behavior to reuse where suitable;
  it is not an editor or a multiplayer permission mechanism.
- [Mod SDK guidance](../../../sdk/AGENTS.md) defines the wrapper, command, host
  and declaration chain. Extend that chain consistently for editor tools.
- [Existing admin dashboard](../../../resources/admin-dashboard/admin.js) uses
  permission checks and revocation handling. Future live editing should join
  that authority model, not infer rights from having a camera or loaded plugin.

No asset budget increases, automatic rendering-to-collision conversion, or
modification of the underlying retail map are part of this feature.

## User workflow

Open **God Mode → Interior Setup** from an in-game menu in a local session.
The persistent toolbar shows the active document, Map/Interior selector,
Select/Place/Move/Rotate tools, Undo/Redo, Test, Save and Close. A tool palette
contains native tools and plugin sections. A property panel describes the
selected object. Camera position is available as optional advanced information;
normal placement needs no numeric input.

1. Open an existing package or create a local draft from a prepared GLB and its
   collision shell. Missing/invalid collision is an import error, not a walkable
   preview. Preserve source credits and notices when copying assets.
2. In Map view, choose **New entrance**, or select an existing entrance and
   **Add floor**. Enter building/floor labels through the inspector. Existing
   floors and keys remain stable unless explicitly edited.
3. Point at a surface and place the entrance. Set its RGB color, opacity,
   radius and height. Place a separate return anchor and its facing arrow.
4. Switch to Interior view. Place the arrival anchor and facing arrow, then
   the exit marker. The editor remembers independent map/interior camera poses.
5. **Validate** highlights unsupported anchors, obstructions, unsafe returns,
   overlapping interior placement and asset-budget failures at their objects.
6. **Test** temporarily uses the draft through the native locations service.
   Enter, choose a floor, move normally, and exit. **Return to editing** restores
   the editor camera. Testing never silently saves or overwrites the package.
7. **Save** validates and writes the package. Reopening reconstructs the same
   objects, properties and floor connections from persisted data.

Selecting an existing building in the scene and selecting it from the palette
are equivalent. Adding a floor reuses the entrance and return rather than
creating overlapping entrance cylinders. Delete reports affected references;
it cannot leave orphaned floor references. Removing the final floor removes its
entrance as one undoable operation after showing that consequence.

## Manual movement and placement

Keyboard/mouse is the required initial control path: hold right mouse to look,
WASD to translate, Q/E down/up, Shift for faster movement and wheel to adjust
speed. Left click selects or places, Escape cancels the active operation,
Ctrl+Z/Ctrl+Y undo/redo. Text fields and UI pointer capture suppress movement.
Bindings belong to editor actions rather than scattered key checks, allowing
controller support without changing tool implementations. Physical controller
acceptance is a separate follow-up, not implied by keyboard testing.

The camera flies without gravity, momentum or collision. Placement uses native
collision ray hits, with optional grid snapping and position/heading handles.
The ghost is translucent and includes a standing-character clearance preview.
No-hit and invalid placements are visibly invalid and cannot be committed as
working anchors. Users can preserve incomplete work as a draft, but it cannot
be exported as a validated runtime package.

Marker anchors remain floor positions, not the camera position or a character's
root height. Interior-local editing coordinates are converted explicitly through
the interior's shared model/collision transform when exporting the world-space
catalog. Initial asset layout is fixed after import; arbitrary mesh scaling and
remeshing are out of scope. New interiors get isolated pocket placements checked
against terrain and other interiors; changing the view does not teleport the
player or accidentally change an interior's transform.

Static clearance alone did not catch the previously rejected Rippon sliding
entrance. Show floor slope and require a short native standing test at all four
anchor roles before marking a new/changed placement tested. Record drift and
loss of support visibly. Test completion is invalidated by changes to the
relevant placement or collision. A supported ray hit is never described as full
walkability verification.

## Session and resource lifecycle

States are Closed, Editing, PreparingTest, Testing and Restoring. Camera/input
ownership is exclusive with replay, debug camera, customiser and other modal
interfaces. Entry captures the local player's pose and current camera state,
suspends local gameplay advancement and hides the local avatar/board in the
authoring view. Editor UI and camera continue updating independently of the
paused simulation. Editing time must not accumulate physics catch-up steps.

Testing resumes the native simulation using prepared draft collision and normal
travel/input. Returning to editing restores the authoring state. Closing restores
the previous valid gameplay package, avatar, camera and safe player pose before
discarding draft collision. Draft interiors must not disappear beneath an actor.
Loading or restoring failures keep the last valid support installed and report
the error; there is no fallback teleport into an unloaded destination.

Focus loss cancels dragging and releases capture. Input held during a mode
transition must be released before it can trigger gameplay. Scene replacement,
plugin unload and application close follow the same cleanup path. Unsaved work
offers Save draft / Discard / Cancel. Local editing cannot open or apply changes
while connected to a multiplayer session in this first release.

## Native components and draft transactions

Separate the editor session/camera, selection and handles, tool registry,
document model, command history, validation and persistence. Native interior
tools and plugin tools use the same document-command interface. Tool code does
not receive unrestricted ECS access or write a runtime catalog directly.

Each object has a stable ID, owner, type, scene reference, transform and typed
properties. Each document has a revision. A command identifies its target and
expected revision, includes before/after values, and is validated before one
atomic draft change. A drag creates one undo step. Failed commands create no
history entries. Undo/redo uses the same validation and ownership checks; stale
commands are rejected with a refresh path rather than overwriting external edits.

Runtime catalog records remain the source for native location behavior. A
versioned editor sidecar stores plugin type versions, object IDs and editor-only
metadata. Camera poses and undo history are session state; history need not
survive reopening. Unknown/unloaded plugin records are preserved read-only and
identified by owner, never silently discarded. Draft recovery can retain them;
runtime export is blocked if required plugin-owned edits cannot be validated.

Save stages a complete new package beside the destination, validates it, then
switches it into place with a recoverable previous version. Use contained paths,
refuse symlink/path escapes, and compare the loaded revision before replacing
an existing package. Save As can create a new mod/resource directory; existing
assets, manifests and unrelated files are not erased. No implicit publication,
server upload or mutation of the retail map occurs.

## Plugin editor contract

The following is a new contract to implement, not an already available API.
Expose it through a versioned `sdk.editor` namespace using the existing command
and result transport. A local plugin registers namespaced tool/type descriptors
and receives stable IDs plus events. Descriptors cover:

- Palette label/category and bounded preview assets or native marker shapes.
- Typed, bounded property schemas (text, number, boolean, color, enumeration,
  destination/object reference) rendered by the native inspector.
- Placement rules, allowed scenes and supported move/rotate operations.
- Selection, placement, property-change and action callbacks.
- Versioned serialization and validation of plugin-owned records.

Plugins can query the active scene, camera and selection, request camera framing,
register placeable types, submit draft commands, and supply additional tool panels
through the existing UI mechanisms. Native selection, handles and undo work for
plugin objects. Custom panels must submit the same commands as the inspector.
Callbacks execute within existing runtime limits; an exception cancels only the
pending operation and reports the responsible plugin.

The host derives ownership and registration generation from the running plugin,
not a supplied owner string. Access combines the editor session's privileges
with plugin grants and object ownership. Explicit delegation is needed for
cross-owner changes. A registration is not permission to alter arbitrary objects,
camera ownership or filesystem paths. Stale registrations and commands after
unload are rejected; active previews and panels are removed on unload.

Initial extensibility is real, not only reserved API names: ship a synthetic
example plugin that supplies a teleport-entrance tool and property panel, places
and moves its entrance, adds a destination/floor, supports undo/redo and saves
through the native catalog adapter. Ordinary travel still works after closing
the editor. Arbitrary dynamic-body authoring and unbounded script UI frameworks
are outside this first release.

## Future live admin backend

Build on the existing optional native accounts/roles system before enabling
live admin edits. Roles group explicit permissions and may inherit from other
roles; a displayed rank or numeric priority alone never authorizes an action.
Server owners can configure ranks such as builder, moderator and administrator
without hardcoding those names into tools. Observe, placement, deletion and
persistent-save permissions remain independently grantable.

A custom rank plugin can supply rank management, presentation and assignments
through a server-authorized integration with that common permission boundary.
This integration is a future API to design, not an existing arbitrary role-write
grant. The server verifies the admitted identity and effective permissions for
each operation regardless of which provider supplies rank policy. Revocation
invalidates active privileges and queued edits. Plugin unload must not grant
default admin rights or silently fall back to a more privileged policy.

Keep a usable native configuration available without a rank plugin. Servers
without authenticated admin permissions cannot enable privileged remote world
editing. Local authoring does not require server ranks. Player home decorating
uses home/item ownership checks independently of admin rank membership.

The same camera, palette, selection, inspector and document commands will later
serve as the default immersive admin surface. That future backend must explicitly
grant observe, place, move, delete, player-action and persistent-save rights.
An invisible observer is server-authorized and has no gameplay collision or
visible avatar; hiding a local mesh alone is insufficient. The world continues
running, and committed edits are visible to players. Plugin tools get the same
capabilities available to native tools within the combined grants.

Commands carry actor identity, owner/generation and expected world revision to
the server. The server validates permissions, assets, references and collision,
records an audit event, commits a revision, and publishes it to clients. Undo is
a new authorized command against current state, not a blind historical rewind.
Permission revocation immediately ends observation/editing access and retires
pending operations. Ephemeral session edits and persisted changes are explicit
choices in that backend.

Interior/collision edits must prepare matching assets and collision on the host
and clients, resolve existing occupants and return leases, then activate the new
revision. The current immutable resource-admission contract must not be bypassed
by a mutable marker setting. Concurrent edits, live admission transitions and
player administration require their own implementation design before activation.
These are extension constraints for this release, not claims of delivered live
editing support.

## Future player home decorating

Players enter **Decorate Home** inside a home they are authorized to furnish.
The palette shows available owned items and quantities. Select an item, preview
its placement, move or rotate it, and confirm. Selecting existing furniture
offers Move, Rotate and Return to inventory, along with any permitted plugin
properties. Free-camera navigation stays within the home's editing bounds;
world navigation and admin tools are unavailable in this mode.

The shared placement machinery must accept a mode policy, asset palette and
scene bounds without assuming that every editor user can edit structural
geometry, entrances or other owners' objects. Distinguish the plugin that
defines an object type from the player who owns an item instance and the home
where it is placed. Those identities are checked separately.

In multiplayer, the server validates home access, item ownership and available
quantity, permitted properties, support/collision and the current layout
revision. Confirming placement atomically assigns an owned item instance to the
home and updates its persisted layout; moving it does not consume another item.
Returning it to inventory reverses that assignment. Retry, reconnect and undo
must not duplicate items or bypass current ownership. Undo is a newly validated
operation; it cannot reverse a purchase or restore an item subsequently traded
away. Purchasing and trading systems are separate from editor command history.

Furniture previews are local and have no live collision. Accepted changes use
matching server/client collision and become visible to occupants only through
the authoritative apply path. Placement protects arrival/exit clearance and
required access routes, respects home boundaries and rejects unsafe overlap
with occupants. It cannot move structural walls or alter another home's layout.
Exact furnishing rules, concurrent edits, disconnect recovery and collision
activation will be specified with that later backend.

Plugins can supply furniture types, previews, property schemas and actions using
the same registry. Only types and actions permitted for player decorating appear
in this mode; registration alone never grants an item or charges currency.
Initial interior authoring does not implement a shop, player-home ownership,
inventory persistence or mutable furniture replication. Its design must simply
avoid coupling the shared interaction tools to unrestricted creator access.

## Validation and acceptance

Use synthetic redistributable interiors for automated tests and owned Downtown
only for separately reported live checks. Required coverage:

1. Draft add/edit/delete and floor-link invariants; stable IDs; undo/redo; stale
   revisions; schema version handling; malformed and nonfinite properties.
2. Camera movement changes no player pose; UI capture/focus loss suppresses
   controls; mode close restores input, camera, visibility and supported state.
3. Local/world coordinate round trips; shared model/collision transform; native
   support/clearance and return separation; cumulative resource budgets.
4. Transactional save/reopen, conflict detection, contained paths, failed writes,
   attribution preservation and restoration of the prior playable package.
5. Plugin registration, placement, inspector edit and serialization; unauthorized
   cross-owner edits; exceptions, unload and stale generation cleanup.
6. Live keyboard authoring: new building, second floor on existing building,
   Map/Interior switching, all four anchors, undo/redo and save/reopen. Run normal
   entry/exit from the saved result without a positioning fixture for that round
   trip; capture actual editor and gameplay screenshots.
7. Test-mode failure and cancellation, unsupported/sliding placements, closing
   while inside a draft interior, and attempted editor entry while connected.
8. Load the exported standard package on a dedicated server and exercise a
   two-client round trip with matching admitted collision. This checks export
   compatibility, not live administration or verified-native-input competition.

Obtain an independent final implementation review. Report controller hardware,
verified-native-input and room-traversal coverage separately; do not infer them
from keyboard, static placement or neutral replay checks.

## Delivery constraints

Remain on `proper-map`, preserve the existing map/interior work and unrelated
changes, and do not commit, push or publish. After implementation, run focused
checks and `graphify update .`. This specification is documentation only; its
review precedes the implementation plan and selection of execution method.
