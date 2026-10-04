---@meta
-- Resource API 1 declarations. Not executed. Client sdk declarations are skate.lua.
---@alias ResourceValue nil|boolean|number|string|ResourceValue[]|table<string,ResourceValue>
---@alias ResourceHandler fun(payload:ResourceValue,sender:string)
---@class ResourceAPI
---@field api_version integer
---@field id string
---@field version string
---@field side 'client'|'server'
---@field generation string Canonical decimal u64.
---@field grants table<string,boolean> Explicit grants, separate from compiled capabilities.
resource = {}

---Register a local event within this resource. Requires resource.events.
---@param name string
---@param callback ResourceHandler
function resource.on(name, callback) end
---Register a network event. Sender is provided by the host, never the payload.
---@param name string
---@param callback ResourceHandler
function resource.on_net(name, callback) end
---@param name string
function resource.off(name) end
---@param name string
---@param payload ResourceValue Maximum 384 JSON bytes.
function resource.emit(name, payload) end
---Client sends only to server. Server may target one player or broadcast.
---@param name string
---@param payload ResourceValue Maximum 384 JSON bytes.
---@param recipient? string Canonical decimal player ID; server only.
function resource.send(name, payload, recipient) end
resource.state = {}
---@param key string
---@return ResourceValue
function resource.state.get(key) end
---Server-owned state only; nil deletes. Requires resource.state.
---@param key string
---@param value ResourceValue
function resource.state.set(key, value) end
resource.storage = {}
---@param key string
---@return ResourceValue
function resource.storage.get(key) end
---Scoped by side, server source and resource; never sent as downloadable content.
---@param key string
---@param value ResourceValue
function resource.storage.set(key, value) end
---@param name string Declared by manifest exports.
---@param callback fun(payload:ResourceValue):ResourceValue
function resource.export(name, callback) end
---@param dependency string Declared exact-version dependency.
---@param name string
---@param payload ResourceValue
---@return ResourceValue
function resource.call(dependency, name, payload) end
---Server only. Host checks permission before invoking callback.
---@param name string
---@param permission string
---@param callback fun(args:string[],actor:string)
function resource.command(name, permission, callback) end
---Server observations are owner reported under Hybrid Authority.
---@return table[]
function resource.players() end

---Own typed operator settings. Requires resource.settings; no script mutation API.
resource.settings = {}
---@param key string
---@return boolean|number|string
function resource.settings.get(key) end
---@return table<string, boolean|number|string>
function resource.settings.all() end
