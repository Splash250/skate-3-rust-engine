---@meta
-- Editor declarations only. Runtime functions are installed by resource API 1.
---@class ResourceVector2
---@field x number
---@field y number
---@class ResourceVector3: ResourceVector2
---@field z number
---@class ResourceVector4: ResourceVector3
---@field w number
---@return ResourceVector2
function vector2(x,y) end
---@return ResourceVector3
function vector3(x,y,z) end
---@return ResourceVector4
function vector4(x,y,z,w) end
vec2,vec3,vec4=vector2,vector3,vector4
---@param callback fun()
---@return integer
function CreateThread(callback) end
---@param milliseconds number
function Wait(milliseconds) end
---@return integer
function GetGameTimer() end
---@param milliseconds number
---@param callback fun()
---@return string
function SetTimeout(milliseconds,callback) end
---@param key string
function ClearTimeout(key) end
Citizen={CreateThread=CreateThread,Wait=Wait,SetTimeout=SetTimeout}
---@class ResourceApi
---@field id string
---@field side 'client'|'server'
---@field generation string
---@field grants table<string,boolean>
resource={}
---@param name string
---@param callback fun(payload:any,sender:string)
function resource.on(name,callback) end
---@param name string
---@param callback fun(payload:any,sender:string)
function resource.on_net(name,callback) end
---@param name string
---@param payload any
function resource.emit(name,payload) end
---@param name string
---@param payload any
---@param recipient? string
---@param scope? ResourceScope
function resource.send(name,payload,recipient,scope) end
---@param name string
function resource.off(name) end
resource.state={}
---@param key string
---@return any
---@param scope? ResourceScope
function resource.state.get(key,scope) end
---@param key string
---@param value any
---@param scope? ResourceScope
function resource.state.set(key,value,scope) end
resource.storage={}
---@param key string
---@return any
function resource.storage.get(key) end
---@param key string
---@param value any
function resource.storage.set(key,value) end
---@param name string
---@param callback fun(payload:any):any
function resource.export(name,callback) end
---@param dependency string
---@param name string
---@param payload any
---@return any
function resource.call(dependency,name,payload) end
---@class ResourceTeleport
---@field position? number[] Required unless restore_previous=true.
---@field heading? number
---@field velocity? number[]
---@field instance? integer
---@field restore_on_stop? boolean Host restores temporary travel on owner generation retirement.
---@field restore_previous? boolean Return this owner generation's existing lease; omit all destination fields.
---@param player string
---@param destination ResourceTeleport
function resource.teleport(player,destination) end
resource.services={}
---@param key string
---@param operation table
---@param timeout_ms integer
function resource.services.submit(key,operation,timeout_ms) end
---@param key string
function resource.services.cancel(key) end

---Server-owned shared entity operation; requires resource.entities.
---@param command table
function resource.entity(command) end
resource.entities = {}
---@param command table
function resource.entities.command(command) end

---@class ResourceScope
---@field kind 'resource'|'instance'|'player'|'entity'
---@field id? string
---@return table[]
function resource.entities.all() end

---4 KiB bounded voice operation. Server resource.voice / client engine.voice.
resource.voice = {}
---@param operation table
function resource.voice.submit(operation) end

resource.world = {}
---@param operation table Server-only resource.world; rail_upsert/rail_remove,64KiB.
function resource.world.command(operation) end
resource.competition = {}
---@param operation table Server-only resource.competition; define/start/cancel/remove/native_start/native_cancel,16KiB; native requires configured companion.
function resource.competition.submit(operation) end
resource.transfer = {}
---@param key string
---@param name string
---@param payload any
---@param options? table recipient string (server required) and timeout_ms (default10000)
function resource.transfer.start(key,name,payload,options) end
---@param key string
function resource.transfer.cancel(key) end

---Own typed operator settings. Requires resource.settings; no script mutation API.
resource.settings = {}
---@param key string
---@return boolean|number|string
function resource.settings.get(key) end
---@return table<string, boolean|number|string>
function resource.settings.all() end
