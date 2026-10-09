# Live 3D map API (map capability 1)

The native map renders the complete active world's source geometry, colors and
textures. M or D-pad Left opens it. The expanded map uses Apple Maps on Mac's
navigation conventions, with Ctrl replacing Command on Linux/Windows:

| Action | Control |
| --- | --- |
| Pan freely | Left-drag or arrow keys; pixel trackpad scrolling |
| Rotate | Drag the compass, or Alt/Option + Left/Right |
| Face north | Click the compass, or Shift + Ctrl/Command + Up |
| Top-down / 3D | Click 2D/3D, or Ctrl/Command + D |
| Tilt | Drag the vertical slider beneath the view button |
| Zoom | +/- buttons, wheel, double-click; Alt/Option-double-click zooms out; Ctrl/Command + +/- |
| Recenter on YOU | YOU button or Ctrl/Command + L |

Panning detaches the map from the skater. Switching between top-down and 3D
preserves the center and zoom and restores the previous 3D tilt. Close zoom
selects richer geometry and larger render targets. D-pad Up/Down and bare +/-
remain available as game shortcuts. Navigation blocks local skating input
while expanded; the world and multiplayer simulation continue.

Native pinch and two-finger rotation are handled when the window backend
provides them (macOS/iOS). The current Linux backend does not expose those
gestures; pixel trackpad scrolling pans and mouse/keyboard controls cover
zoom/rotation. Mouse controls are scoped to the expanded map's viewport.

The scene uses a perspective camera, neutral sunlight and shadows to show
terrain height. Terrain writes depth; transparency is applied to the finished
image so buildings remain legible. The compact map continues following YOU.

## Server Lua resources

Request `resource.map` in both the resource manifest and server grants.
`resource.map.set(snapshot)` atomically replaces this resource's shared map
settings/layers. `resource.map.get()` returns its current snapshot (nil if
cleared), and `resource.map.clear()` removes it. The existing resource state
channel distributes it to clients, including newly admitted players. The
reserved state key `__map_v1` cannot be written using `resource.state.set`.
Client scripts cannot publish server map state.

```lua
resource.map.set({settings={title="Session map",opacity=0.92},layers={
  {key="spots",items={
    {kind="marker",key="meet",position={10,40,20},label="Meet here"},
    {kind="path",key="route",points={{10,40,20},{20,40,30}},
     style={color={0.2,0.8,1,1},size=3}}
  }}
}})
```

## Client Lua and local mods

`sdk.map.set(snapshot)` and `sdk.map.clear()` replace/remove this mod's local
snapshot. These calls never publish a network packet. Client resources must
request `engine.map`; trusted local API-2 mods use the compiled
`sdk.capabilities.map == 1` interface, like other local SDK primitives.
`sdk.map.status()` reads the current client snapshot: `name`, canonical string
`generation`, `bounds={min,max}` when available, `zoom_level`, `expanded`, and
`players` with the existing client player observations. The client resource
query also requires `engine.map`. Headless server scripts use
`resource.players()` for host observations; they have no rendering camera.

## Snapshot format

`settings` is optional: `enabled` (bool), `opacity` (0..1), `initial_zoom`
(0..2, used on opening), `title` (up to 64 UTF-8 bytes). Native defaults return
when the owner clears or stops. Server settings override local defaults;
conflicting server defaults follow resource ID order. All resources retain
separate layer ownership regardless of settings order.

`layers` is a list of `{key, visible=true, items}`. Each item is one of:

| kind | Fields |
| --- | --- |
| marker | `key, position={x,y,z}, label=""` |
| label | `key, position={x,y,z}, text` |
| path | `key, points={{x,y,z},...}` (at least 2) |
| region | `key, points={{x,y,z},...}` (at least 3; closed outline) |

Every item optionally has `style={color={r,g,b,a},size=4}`. RGBA values are in
0..1; size is marker diameter/line width in (0,128] logical pixels. Coordinates
are finite world XYZ values within +/-100,000 metres. Position height is used
by the same 3D camera projection as the terrain and player markers. Keys are
nonempty and unique within their owning layer; the same key may appear in a
different owner's layer. Labels/keys/titles accept at most 64 UTF-8 bytes and
no control characters. Vector layers accept no image, mesh, or filesystem path.

Each owner is limited to 8 layers, 128 items, 512 total path/region points,
and 8 KiB serialized state. Servers can publish 5 updates per rolling second;
batch changes into one `set`. Invalid, excessive or over-rate updates raise a
Lua error and preserve the last valid snapshot. Use `pcall` to handle expected
rejections. Generic resource queues and negotiated transport budgets still
apply. The native HUD renders at most 4096 script vector parts; built-in player
markers are independent of this ceiling.

Layers retire on resource/mod stop, disconnect, or committed map replacement.
Republish map-specific content after changing worlds. Zoom or target changes
do not alter layer ownership. No map API call changes terrain, collisions,
player transforms, or multiplayer authority.

Dedicated clients retain existing server-selected resource discovery; they do
not discover standalone local mod packages. The server example includes a
client script: press K on one client to toggle a private marker, proving that
local resource layers stay on that client.

Examples: [server resource](../resources/programmable-map/) and
[private local mod](examples/programmable-map/).
