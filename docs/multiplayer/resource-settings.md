# Typed resource settings

Operators configure resources without editing scripts. Settings belong to the
resource declaring them; runtime exports are the explicit way to share derived
behavior with another resource. Lua, JavaScript and C# use the same host contract.

```json
{
  "requires_features": ["resource.settings.v1"],
  "capabilities": ["resource.settings"],
  "settings": {
    "round_seconds": {"type":"integer","default":180,"min":10,"max":3600,"visibility":"replicated","change":"live"},
    "rotation": {"type":"enum","default":"park","options":["park","street"],"visibility":"public","change":"restart"},
    "operator_note": {"type":"string","default":"","max_bytes":256,"visibility":"private"}
  }
}
```

This fragment extends an ordinary API1 manifest. Exact resource dependency
versions and grants remain unchanged. Unsupported required feature IDs reject
installation; no dependency resolver or executable installer is introduced.
Defaults are mandatory. Types are `boolean`, `integer`, `number`, `string` and
string `enum`. Numeric bounds are inclusive. Integers must be exactly representable
within ±9,007,199,254,740,991; strings use UTF-8 byte limits. Unknown fields,
unknown setting names, invalid values and duplicate JSON schema keys are rejected.
Schemas have at most64keys/32KiB, effective values at most8KiB. String values have
`max_bytes` default256, maximum4096. Enum options must be distinct bounded strings.

## Script use

Request and receive the `resource.settings` capability. Own configuration is
read-only to scripts, and returned objects are copies:

```lua
local duration = resource.settings.get('round_seconds')
return {on_settings = function(change)
    if change.key == 'round_seconds' then duration = change.value end
end}
```

```javascript
let duration = resource.settings.get('round_seconds');
resource.lifecycle({on_settings(change) {
  if (change.key === 'round_seconds') duration = change.value;
}});
```

```csharp
var duration = resource.SettingsGet("round_seconds");
resource.Lifecycle("on_settings", change => {
    duration = resource.SettingsGet("round_seconds");
});
```

`resource.settings.all()` / C# `SettingsAll()` returns the own namespace. Scripts
cannot read another resource's private values, alter grants or write the reserved
replicated-state key `__settings`. Account IDs and secrets must not be embedded in
setting names, public values or callback error messages.

## Visibility, changes and persistence

- `private` is the default. Definitions, defaults and values remain server-side.
- `replicated` definitions/defaults are public content; effective values are sent
  to admitted clients in the authenticated resource state lane.
- `public` additionally allows the active value to be advertised in discovery.

Never put secrets into replicated/public definitions or defaults. Operator
private overrides and settings stores are excluded from immutable content.
Private reads require `settings.read` at the admin boundary; updates require
`settings.write`. Audit the resource, key and result, never the value. General
`status.read` must not expose private schemas or effective values.

`change:"live"` applies after the durable update and invokes `on_settings` with
`{key,value}`. `change:"restart"` records a pending value while running, applies
when the resource next starts, and uses normal dependent-resource restarts.
Changing a stopped resource applies immediately for its next startup. The full
client settings snapshot must be accepted before startup callbacks; the game
uses two-phase content activation. Generation checks reject stale settings.

The host merges manifest defaults, operator startup defaults, then persisted
admin overrides. Persisted values win across process restart. Removed or changed
schemas that invalidate persisted values reject startup with a migration error;
stop the resource and migrate its settings store deliberately. Automatic coercion
or silently discarding operator values is not supported.

Stores live under the existing side/source-scoped host storage directory at
`settings/RESOURCE.json`, separate from script-writable storage. Updates use a
bounded temporary file, file sync and atomic rename; Unix also syncs the parent
directory and creates private permissions. Validation occurs before writes.
The update commits before notification: if a notification fails, the resource
retires and the saved value remains effective on restart. This is a single-file
update contract, not an atomic transaction spanning a resource database, the
accounts store and configuration. Use the documented stopped-store operational
backup workflow for coherent multi-store backups.

## Administration

The existing authenticated admin action endpoint accepts
`{kind:"settings_read",resource:"my-resource"}` and
`{kind:"settings_set",resource:"my-resource",key:"round_seconds",value:240}`.
Read the returned action ticket to see the actual result. Reads require
`settings.read`; updates require `settings.write`. The resource configuration
also accepts an operator-only `settings` object keyed by resource ID, with flat
key/value objects inside, for startup defaults. Existing persisted admin updates
win over these startup defaults.

## Host integration and checks

`Host::configure_settings(id, values)` supplies stopped-resource startup defaults.
`set_setting(id,key,value)` returns `{applied,restart_required,notification_error}`.
`settings_snapshot(id)` includes definitions/current/pending values and belongs
only on an authorized surface. `settings_values(id,SettingAudience::Client)` strips
private fields; `Public` additionally strips replicated-only fields. A monotonic
`settings_revision()` signals changes without serializing every tick. The network
host alone may call client `apply_settings(id,generation,values)`.

```sh
cargo test --locked -p skate-resources --test contracts
cargo test --locked -p skate-mods --test resource_runtime resource_settings
SKATE_DOTNET_ROOT=/absolute/dotnet SKATE_MANAGED_HOST=/absolute/managed-host \
  cargo test --locked -p skate-mods --test managed_runtime csharp_typed_settings -- --ignored --test-threads=1
```

The C# check requires the trusted worker rebuilt from this revision using the
[SDK instructions](../../sdk/RESOURCES.md). Native Windows validation remains a
separate acceptance requirement.
