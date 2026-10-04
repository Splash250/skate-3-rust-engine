//! Server-owned resource configuration and runtime. Only the public content
//! projection reaches HTTP; configuration, grants and persistence stay here.
use serde::Deserialize;
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_net::{
    dedicated::Server,
    resources::{Kind, Message},
};
use skate_resources::{HttpServer, Manifest, PublishedSet, build_set};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub native_authority: Option<crate::native_authority::Config>,
    #[serde(default)]
    pub content_limits: skate_resources::Limits,
    #[serde(default)]
    pub network_budgets: skate_net::resources::Budgets,
    #[serde(default)]
    pub runtime_limits: skate_mods::resources::RuntimeLimits,
    #[serde(default)]
    pub http_origins: BTreeMap<String, BTreeSet<String>>,
    pub root: PathBuf,
    pub storage: PathBuf,
    pub ensure: Vec<String>,
    #[serde(default)]
    pub grants: BTreeMap<String, BTreeSet<String>>,
}
pub struct Platform {
    authenticated_accounts: BTreeMap<u64,String>,
    logs: std::collections::VecDeque<(String,String)>,
    entities: crate::entities::ServerWorld,
    rails: crate::world::Rails,
    transfers: skate_net::transfers::Transfers,
    competition: crate::competition::Competition,
    native_authority: crate::native_authority::NativeAuthority,
    services: skate_services::Services,
    owners: BTreeMap<String, (u64, skate_services::Owner)>,
    requests: BTreeMap<u64, (String, u64, String)>,
    voice_owners: BTreeMap<String, u64>,
    voice_operations: std::collections::VecDeque<(String, u64, serde_json::Value)>,
    config: Config,
    lua: Host,
    http: HttpServer,
    known: BTreeMap<String, Manifest>,
    published: PublishedSet,
}
impl Platform {
    fn native_blocked_instances(&self) -> BTreeSet<u64> {
        let mut blocked=self.rails.instances();
        blocked.extend(self.entities.snapshot().into_iter().map(|entity|entity.instance));
        blocked
    }
    pub fn load(path: &Path, bind: SocketAddr, server: &mut Server) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|e| format!("Resource configuration {}: {e}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65_536 {
            return Err("Resource configuration exceeds 64 KiB".into());
        }
        let mut config: Config =
            serde_json::from_slice(&bytes).map_err(|e| format!("Resource configuration: {e}"))?;
        let parent = path.parent().unwrap_or(Path::new("."));
        if let Some(native) = &mut config.native_authority { native.resolve(parent)?; }
        config.root = parent.join(&config.root);
        config.storage = parent.join(&config.storage);
        config
            .content_limits
            .validate()
            .map_err(|e| e.to_string())?;
        config.network_budgets.validate()?;
        for (resource,origins) in &config.http_origins {
            skate_resources::validate_id(resource).map_err(|e|e.to_string())?;
            skate_services::Grants {database:false,http_origins:origins.clone()}.validate().map_err(|e|format!("HTTP grants for {resource}: {e}"))?;
        }
        config.runtime_limits.validate()?;
        if config.runtime_limits.max_payload_bytes > config.network_budgets.value_bytes
            || config.runtime_limits.max_state_keys > config.network_budgets.state_keys
        {
            // One host value contract on both sides of the script/wire boundary.
            config.runtime_limits.max_payload_bytes = config
                .runtime_limits
                .max_payload_bytes
                .min(config.network_budgets.value_bytes);
            config.runtime_limits.max_state_keys = config
                .runtime_limits
                .max_state_keys
                .min(config.network_budgets.state_keys);
        }
        // Do not advertise payloads the receiving script host cannot accept.
        config.network_budgets.value_bytes=config.network_budgets.value_bytes.min(config.runtime_limits.max_payload_bytes);
        server.set_resource_budgets(config.network_budgets)?;
        if config.ensure.len() > config.network_budgets.resources
            || config.ensure.iter().collect::<BTreeSet<_>>().len() != config.ensure.len()
        {
            return Err("Duplicate or too many ensured resources for configured budget".into());
        }
        let selected = config.ensure.iter().map(|id| (id.clone(), 1)).collect();
        let published =
            skate_resources::build_set_with_limits(&config.root, &selected, config.content_limits)
                .map_err(|e| e.to_string())?;
        // Decode and validate server-selected collision before scripts can run
        // or clients can receive an admission offer for this world.
        let terrain = crate::world::Terrain::from_published(&published)?;
        let mut competition=crate::competition::Competition::default();
        competition.set_terrain(terrain.as_ref())?;
        let mut native_authority=crate::native_authority::NativeAuthority::new(config.native_authority.clone());
        native_authority.set_world(&published)?;
        let scope = std::fs::canonicalize(path)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let mut lua = Host::new_with_limits(
            Side::Server,
            config.storage.clone(),
            &scope,
            config.runtime_limits.clone(),
        )?;
        let services = skate_services::Services::new(
            config
                .storage
                .join("services")
                .join(skate_resources::digest_bytes(scope.as_bytes())),
            skate_services::Limits {
                response_bytes: 128 * 1024,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let mut known = BTreeMap::new();
        let installed = Self::installed(&config, &published, &mut known)?;
        lua.install(installed)?;
        lua.start_all()?;
        let http = HttpServer::bind_with_limits(bind, published.clone(), config.content_limits)
            .map_err(|e| e.to_string())?;
        let mut platform = Self {
            authenticated_accounts: BTreeMap::new(),
            logs: Default::default(),
            entities: Default::default(),
            rails: Default::default(),
            transfers: Default::default(),
            competition,
            native_authority,
            services,
            owners: BTreeMap::new(),
            requests: BTreeMap::new(),
            voice_owners: BTreeMap::new(),
            voice_operations: std::collections::VecDeque::new(),
            config,
            lua,
            http,
            known,
            published: published.clone(),
        };
        platform.entities.set_terrain(terrain)?;
        platform.advertise(server, &published)?;
        platform.flush(server);
        Ok(platform)
    }
    fn installed(
        config: &Config,
        published: &PublishedSet,
        known: &mut BTreeMap<String, Manifest>,
    ) -> Result<Vec<InstalledResource>, String> {
        published
            .set
            .resources
            .iter()
            .map(|resource| {
                let id = &resource.manifest.id;
                let manifest = Manifest::read(&config.root.join(id)).map_err(|e| e.to_string())?;
                known.insert(id.clone(), manifest.clone());
                Ok(InstalledResource {
                    manifest,
                    root: config.root.join(id),
                    generation: resource.generation,
                    grants: config.grants.get(id).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }
    fn advertise(&mut self, server: &mut Server, published: &PublishedSet) -> Result<(), String> {
        server.set_world_spawn(self.entities.terrain().map(|terrain|skate_net::dedicated::TeleportDestination {
            position:terrain.spawn,heading:terrain.heading,velocity:[0.;3],instance:0,
        }))?;
        server.configure_resources(
            published.set.revision.clone(),
            self.address().port(),
            published
                .set
                .resources
                .iter()
                .map(|r| (r.manifest.id.clone(), r.generation))
                .collect(),
        )
    }
    /// Trusted Host input only. Script event payloads never set account identity.
    pub fn set_authenticated_accounts(&mut self, accounts: BTreeMap<u64,String>) {
        self.authenticated_accounts = accounts;
    }
    fn sync_voice_owners(&mut self, router: &mut skate_voice::Router) {
        let active: BTreeMap<_,_> = self.lua.installed().iter().filter(|(id, installed)| {
            self.lua.running(id) && installed.grants.contains("resource.voice")
                && installed.manifest.capabilities.iter().any(|cap| cap == "resource.voice")
        }).map(|(id,_)|(id.clone(),self.lua.generation(id).unwrap())).collect();
        self.voice_owners.retain(|id, generation| {
            if active.get(id) == Some(generation) {true}
            else {router.revoke(id,*generation);false}
        });
        for (id,generation) in active {
            if self.voice_owners.contains_key(&id) {continue;}
            match router.activate(&id,generation) {
                Ok(()) => {self.voice_owners.insert(id,generation);},
                Err(error) => eprintln!("Voice activation {id}: {error}"),
            }
        }
    }
    pub fn sync_voice(&mut self, router: &mut skate_voice::Router) {
        self.sync_voice_owners(router);
        while let Some((resource,generation,operation)) = self.voice_operations.pop_front() {
            if !self.lua.running(&resource) || self.lua.generation(&resource) != Some(generation) {continue;}
            let result = serde_json::from_value::<skate_voice::ServerCommand>(operation.clone())
                .map_err(|e|format!("Invalid voice operation: {e}"))
                .and_then(|command|router.command(&resource,generation,command));
            let value=serde_json::json!({"operation":operation.get("kind"),"ok":result.is_ok(),"error":result.err()});
            let _=self.lua.host_event(&resource,generation,"voice_result",value);
            // A failing completion handler cannot retain a channel for another tick.
            self.sync_voice_owners(router);
        }
    }
    pub fn admin_status(&self) -> serde_json::Value {
        let metrics = self.lua.runtime_metrics();
        let sampled: Vec<_> = metrics.iter().take(16).map(|(id, value)| serde_json::json!({
            "id":id, "generation":value.generation.to_string(), "running":value.running,
            "invocations":value.invocations, "errors":value.errors,
            "last_error":value.last_error.as_ref().map(|s|s.chars().take(512).collect::<String>()),
            "max_wall_time_us":value.max_wall_time_us, "max_host_cpu_time_us":value.max_host_cpu_time_us,
            "lua_heap_bytes":value.lua_heap_bytes, "javascript_heap_bytes":value.javascript_heap_bytes,
            "managed_resident_bytes":value.managed_resident_bytes,
            "queued_outputs":value.queued_outputs, "queued_output_accounted_bytes":value.queued_output_accounted_bytes
        })).collect();
        bounded_admin_status(serde_json::json!({
            "resources": self.known.keys().map(|id|serde_json::json!({"id":id,"running":self.lua.running(id),"generation":self.lua.generation(id).map(|value|value.to_string())})).collect::<Vec<_>>(),
            "logs": self.logs.iter().rev().take(16).collect::<Vec<_>>(),
            "runtime_metrics":sampled,
            "runtime_metrics_omitted":metrics.len().saturating_sub(16),
            "diagnostics": self.lua.diagnostics.iter().rev().take(16).map(|value|value.chars().take(512).collect::<String>()).collect::<Vec<_>>()
        }))
    }
    pub fn address(&self) -> SocketAddr {
        self.http.local_addr()
    }
    pub fn entity_contact_metrics(&self) -> crate::entities::ContactMetrics {
        self.entities.contact_metrics()
    }
    pub fn shutdown(&mut self) {
        self.native_authority.stop();
        self.lua.disconnect();
        self.sync_services();
    }
    fn sync_services(&mut self) {
        self.entities.sync_resources(self.lua.running_ids().into_iter().filter_map(|id| self.lua.generation(&id).map(|generation|(id,generation))).collect());
        self.rails.sync_resources(self.lua.running_ids().into_iter().filter_map(|id|self.lua.generation(&id).map(|generation|(id,generation))).collect());
        let competition_owners: BTreeMap<_,_> = self.lua.running_ids().into_iter().filter(|id| {
            let entry=&self.lua.installed()[id];
            entry.grants.contains("resource.competition") && entry.manifest.capabilities.iter().any(|c|c=="resource.competition")
        }).filter_map(|id|self.lua.generation(&id).map(|generation|(id,generation))).collect();
        self.competition.sync_resources(competition_owners.clone());
        self.native_authority.sync_resources(competition_owners);
        let stale: Vec<_> = self
            .owners
            .iter()
            .filter(|(id, (generation, _))| {
                !self.lua.running(id) || self.lua.generation(id) != Some(*generation)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some((_, owner)) = self.owners.remove(&id) {
                self.services.revoke(&owner);
            }
            self.requests.retain(|_, (resource, _, _)| resource != &id);
        }
        for id in self.lua.running_ids() {
            if self.owners.contains_key(&id) {
                continue;
            }
            let Some(entry) = self.lua.installed().get(&id) else {
                continue;
            };
            let grants = &entry.grants;
            let requested = &entry.manifest.capabilities;
            let allowed = |cap: &str| grants.contains(cap) && requested.iter().any(|c| c == cap);
            let service_grants = skate_services::Grants {
                database: allowed("resource.database"),
                http_origins: if allowed("resource.http") {
                    self.config
                        .http_origins
                        .get(&id)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    BTreeSet::new()
                },
            };
            let generation = self.lua.generation(&id).unwrap();
            match self.services.activate(&id, generation, service_grants) {
                Ok(owner) => {
                    self.owners.insert(id, (generation, owner));
                }
                Err(error) => eprintln!("Backend activation {id}: {error}"),
            }
        }
    }
    fn service_error(
        &mut self,
        resource: &str,
        generation: u64,
        key: &str,
        code: &str,
        error: String,
    ) {
        let _ = self.lua.service_result(
            resource,
            generation,
            key,
            serde_json::json!({"ok":false,"error":{"code":code,"message":error}}),
        );
        self.sync_services();
    }
    fn poll_services(&mut self) {
        self.sync_services();
        while let Some(completion) = self.services.poll() {
            let Some((resource, generation, key)) = self.requests.remove(&completion.id) else {
                continue;
            };
            let result = match completion.result {
                Ok(value) => serde_json::json!({"ok":true,"value":value}),
                Err(error) => serde_json::json!({"ok":false,"error":error}),
            };
            let maximum = self.config.runtime_limits.max_storage_value_bytes;
            let result = if serde_json::to_vec(&result)
                .is_ok_and(|bytes| bytes.len() + key.len() + 64 <= maximum)
            {
                result
            } else {
                serde_json::json!({"ok":false,"error":{"code":"too_large","message":"Backend result exceeds resource callback byte budget"}})
            };
            if let Err(error) = self.lua.service_result(&resource, generation, &key, result) {
                eprintln!("Backend callback {resource}: {error}");
                self.sync_services();
            }
        }
    }
    pub fn step(&mut self, dt: f64, server: &mut Server) {
        let (players, entities) = server.resource_scope_targets();
        self.lua.prune_scoped_targets(&players, &entities);
        self.poll_services();
        self.sync_services();
        let mut players=server.player_observations();
        if let Some(players)=players.as_array_mut() {
            for player in players {
                let account=player.get("id").and_then(serde_json::Value::as_str).and_then(|id|id.parse::<u64>().ok()).and_then(|id|self.authenticated_accounts.get(&id));
                if let Some(object)=player.as_object_mut() {object.insert("account_id".into(),serde_json::json!(account));}
            }
        }
        self.lua.tick(dt,serde_json::json!({"players":players,"entities":server.entity_observations(),"network":{"active":true,"is_host":true}}));
        for event in server.drain_resource_events() {
            let m = event.message;
            if let Err(error) =
                self.lua
                    .receive(event.sender, &m.resource, m.generation, &m.name, m.value)
            {
                eprintln!(
                    "Resource {} rejected event from {}: {error}",
                    m.resource, event.sender
                );
            }
        }
        self.lua
            .dispatch("on_fixed_update", serde_json::json!({"dt":dt}));
        self.flush(server);
        self.entities.step(dt as f32, server);
        for event in self.competition.step(server,&self.entities.snapshot()) {
            let _=self.lua.host_event(&event.resource,event.generation,"competition_result",event.value);
        }
        let blocked=self.native_blocked_instances();
        for event in self.native_authority.step(server,&blocked) {
            let _=self.lua.host_event(&event.resource,event.generation,"competition_result",event.value);
        }
        let (players, entities) = server.resource_scope_targets();
        self.lua.prune_scoped_targets(&players, &entities);
        // A callback failure stops dependent resources and updates the advertised
        // set, so clients unload them rather than keep a dead game mode alive.
        let retired = self.lua.drain_retired();
        if !retired.is_empty() {
            let metrics = self.lua.runtime_metrics();
            for id in retired {
                let error = self.lua.diagnostics.iter().rev()
                    .find(|line| line.starts_with(&format!("{id}:")) || line.starts_with(&format!("{id} export:")))
                    .map(|line| line.chars().take(2048).collect::<String>())
                    .unwrap_or_else(|| "dependency or owner retired".into());
                let generation = metrics.get(&id).map_or(0, |value| value.generation);
                let text = format!("Resource stopped generation={generation}: {error}");
                eprintln!("[{id}] {text}");
                if self.logs.len() >= 128 { self.logs.pop_front(); }
                self.logs.push_back((id, text));
            }
            if let Err(error) = self.republish(server) {
                eprintln!("Resource failure publication: {error}");
                self.fail_closed(server);
            }
        }
    }
    fn flush(&mut self, server: &mut Server) {
        self.sync_services();
        self.poll_transfers(server);
        for output in self.lua.drain_outputs() {
            let live = match &output {
                Output::Event {
                    resource,
                    generation,
                    ..
                }
                | Output::Entity { resource, generation, .. }
                | Output::Voice { resource, generation, .. }
                | Output::World { resource, generation, .. }
                | Output::Competition { resource, generation, .. }
                | Output::Transfer { resource, generation, .. }
                | Output::CancelTransfer { resource, generation, .. }
                | Output::State {
                    resource,
                    generation,
                    ..
                }
                | Output::Teleport {
                    resource,
                    generation,
                    ..
                }
                | Output::Service {
                    resource,
                    generation,
                    ..
                }
                | Output::CancelService {
                    resource,
                    generation,
                    ..
                } => {
                    self.lua.running(resource) && self.lua.generation(resource) == Some(*generation)
                }
                Output::Log { .. } => true,
            };
            if !live {
                self.sync_services();
                continue;
            }

            let (recipient, message) = match output {
                Output::Event {
                    resource,
                    generation,
                    recipient,
                    name,
                    payload,
                    scope,
                } => {
                    let Ok(scope) = serde_json::from_value(scope) else {
                        eprintln!("Resource {resource}: invalid event visibility scope");
                        continue;
                    };
                    (
                    recipient,
                    Message {
                        scope,
                        id: 1,
                        resource,
                        generation,
                        kind: Kind::Event,
                        name,
                        value: payload,
                    },
                )},
                Output::State {
                    resource,
                    generation,
                    key,
                    value,
                    scope,
                } => {
                    let Ok(scope) = serde_json::from_value(scope) else {
                        eprintln!("Resource {resource}: invalid state visibility scope");
                        continue;
                    };
                    (
                    None,
                    Message {
                        scope,
                        id: 1,
                        resource,
                        generation,
                        kind: Kind::State,
                        name: key,
                        value,
                    },
                )},
                Output::Entity {resource,generation,command} => {
                    let result=serde_json::from_value::<skate_net::entities::Command>(command)
                        .map_err(|error|format!("Invalid shared entity operation: {error}"))
                        .and_then(|command|self.entities.command(&resource,generation,command,server));
                    if let Err(error)=result {eprintln!("Shared entity {resource}: {error}");}
                    continue;
                }
                Output::Voice {resource,generation,operation} => {
                    if self.voice_operations.len() < 128 {
                        self.voice_operations.push_back((resource,generation,operation));
                    } else {
                        let _=self.lua.host_event(&resource,generation,"voice_result",serde_json::json!({"ok":false,"error":"Voice operation queue exhausted"}));
                    }
                    continue;
                }
                Output::World {resource,generation,operation} => {
                    let result=self.rails.command(&resource,generation,operation,server);
                    let _=self.lua.host_event(&resource,generation,"world_result",serde_json::json!({"ok":result.is_ok(),"error":result.err()}));
                    continue;
                }
                Output::Competition {resource,generation,operation} => {
                    let native=operation.get("kind").and_then(|v|v.as_str()).is_some_and(|kind|kind.starts_with("native_"));
                    let actor=operation.get("player").and_then(|v|v.as_str()).and_then(|v|v.parse::<u64>().ok());
                    let result=if native {
                        let blocked=self.native_blocked_instances();
                        if actor.is_some_and(|actor|self.competition.active(actor)) {
                            Err("Cancel the player's course attempt before native authority".into())
                        } else {self.native_authority.command(&resource,generation,operation.clone(),server,&blocked)}
                    } else if actor.is_some_and(|actor|self.native_authority.active(actor)) {
                        Err("Cancel the player's native attempt before course operations".into())
                    } else {self.competition.command(&resource,generation,operation.clone(),server)};
                    let value=match result {Ok(value)=>serde_json::json!({"operation":operation.get("kind"),"ok":true,"value":value}),
                        Err(error)=>serde_json::json!({"operation":operation.get("kind"),"ok":false,"error":error})};
                    let _=self.lua.host_event(&resource,generation,"competition_result",value);
                    continue;
                }
                Output::Transfer {resource,generation,key,name,payload,recipient,timeout_ms} => {
                    let result=(|| {
                        self.transfers.available(&resource,generation,&key,timeout_ms)?;
                        let recipient=recipient.ok_or("Server large transfer requires an explicit recipient")?;
                        let ticket=server.start_large_resource(recipient,Message {scope:Default::default(),id:1,resource:resource.clone(),generation,kind:Kind::Event,name,value:payload})?;
                        let progress=server.large_resource_progress(ticket)?;
                        self.transfers.insert(&resource,generation,&key,timeout_ms,ticket,progress,std::time::Instant::now())
                    })();
                    self.transfer_event(result.unwrap_or_else(|error|skate_net::transfers::Transfers::failed(&resource,generation,&key,error)));
                    continue;
                }
                Output::CancelTransfer {resource,generation,key} => {
                    let event=self.transfers.cancel(&resource,generation,&key,|ticket,cancel| {
                        if cancel {server.cancel_large_resource(ticket)?;}
                        server.large_resource_progress(ticket)
                    });
                    self.transfer_event(event);
                    continue;
                }
                Output::Teleport {
                    resource,
                    generation,
                    player,
                    position,
                    heading,
                    velocity,
                    instance,
                } => {
                    if self.lua.running(&resource)
                        && self.lua.generation(&resource) == Some(generation)
                    {
                        let destination = skate_net::dedicated::TeleportDestination {
                            position,
                            heading: heading.unwrap_or(0.),
                            velocity: velocity.unwrap_or([0.; 3]),
                            instance: instance
                                .map(u64::from)
                                .unwrap_or_else(|| server.instance_of(player).unwrap_or(0)),
                        };
                        if let Err(error) = server.teleport_now(player, destination) {
                            eprintln!("Teleport {resource}: {error}");
                        }
                    }
                    continue;
                }
                Output::Service {
                    resource,
                    generation,
                    key,
                    operation,
                    timeout_ms,
                } => {
                    if self
                        .requests
                        .values()
                        .any(|(r, g, k)| r == &resource && *g == generation && k == &key)
                    {
                        self.service_error(
                            &resource,
                            generation,
                            &key,
                            "busy",
                            "Request key already pending".into(),
                        );
                        continue;
                    }
                    let result = serde_json::from_value::<skate_services::Operation>(operation)
                        .map_err(|_| skate_services::ServiceError {
                            code: skate_services::ErrorCode::Invalid,
                            message: "Invalid backend operation".into(),
                        })
                        .and_then(|operation| {
                            let (active, owner) = self.owners.get(&resource).ok_or_else(|| {
                                skate_services::ServiceError {
                                    code: skate_services::ErrorCode::Stale,
                                    message: "No live backend owner".into(),
                                }
                            })?;
                            if *active != generation {
                                return Err(skate_services::ServiceError {
                                    code: skate_services::ErrorCode::Stale,
                                    message: "Stale backend generation".into(),
                                });
                            }
                            self.services.submit(
                                owner,
                                operation,
                                std::time::Duration::from_millis(timeout_ms),
                            )
                        });
                    match result {
                        Ok(ticket) => {
                            self.requests.insert(ticket, (resource, generation, key));
                        }
                        Err(error) => {
                            let result = serde_json::json!({"ok":false,"error":error});
                            let _ = self.lua.service_result(&resource, generation, &key, result);
                            self.sync_services();
                        }
                    }
                    continue;
                }
                Output::CancelService {
                    resource,
                    generation,
                    key,
                } => {
                    if let Some((&ticket, _)) = self
                        .requests
                        .iter()
                        .find(|(_, (r, g, k))| r == &resource && *g == generation && k == &key)
                    {
                        if let Some((_, owner)) = self.owners.get(&resource) {
                            let _ = self.services.cancel(owner, ticket);
                        }
                    }
                    continue;
                }
                Output::Log { resource, text } => {
                    if self.logs.len()>=128 {self.logs.pop_front();}
                    self.logs.push_back((resource.clone(),text.clone()));
                    println!("[{resource}] {text}");
                    continue;
                }
            };
            if let Err(error) = server.send_resource(recipient, message) {
                eprintln!("Resource output: {error}");
            }
        }
        // Server VMs never grant native client commands. Drain defensively so a
        // future implementation mistake cannot accumulate unbounded native work.
        let commands = self.lua.drain_commands();
        if !commands.is_empty() {
            eprintln!("Discarded unexpected client engine commands on server");
        }
    }
    fn transfer_event(&mut self,event:skate_net::transfers::Event) {
        let _=self.lua.host_event(&event.resource,event.generation,"transfer_progress",event.value);
    }
    fn poll_transfers(&mut self,server:&mut Server) {
        let lua=&self.lua;
        let events=self.transfers.poll(std::time::Instant::now(),|id,generation|lua.running(id)&&lua.generation(id)==Some(generation),|ticket,cancel| {
            if cancel {server.cancel_large_resource(ticket)?;}
            server.large_resource_progress(ticket)
        });
        for event in events {self.transfer_event(event);}
    }
    fn republish(&mut self, server: &mut Server) -> Result<(), String> {
        let mut resources = Vec::new();
        let mut blobs = BTreeMap::new();
        for id in self.lua.running_ids() {
            let generation = self.lua.generation(&id).unwrap();
            // A lifecycle command for A must not hot-publish B's edited files
            // while B's VM and generation still describe its previous content.
            let old = self
                .published
                .set
                .resources
                .iter()
                .find(|r| r.manifest.id == id && r.generation == generation);
            let (resource, source) = if let Some(resource) = old {
                (resource.clone(), &self.published.blobs)
            } else {
                let selected = BTreeMap::from([(id.clone(), generation)]);
                let fresh = skate_resources::build_set_with_limits(
                    &self.config.root,
                    &selected,
                    self.config.content_limits,
                )
                .map_err(|e| e.to_string())?;
                let resource = fresh
                    .set
                    .resources
                    .iter()
                    .find(|r| r.manifest.id == id)
                    .unwrap()
                    .clone();
                for file in resource.files.values() {
                    blobs.insert(file.digest.clone(), fresh.blobs[&file.digest].clone());
                }
                resources.push(resource);
                continue;
            };
            for file in resource.files.values() {
                blobs.insert(file.digest.clone(), source[&file.digest].clone());
            }
            resources.push(resource);
        }
        let published =
            PublishedSet::from_resources(resources, blobs).map_err(|e| e.to_string())?;
        let terrain = crate::world::Terrain::from_published(&published)?;
        self.competition.set_terrain(terrain.as_ref())?;
        self.native_authority.set_world(&published)?;
        self.entities.set_terrain(terrain)?;
        self.http
            .replace(published.clone())
            .map_err(|e| e.to_string())?;
        self.advertise(server, &published)?;
        self.published = published;
        self.rails.republish(server)?;
        // Re-send only live authoritative targets after resetting transport epochs.
        let (players, entities) = server.resource_scope_targets();
        self.lua.prune_scoped_targets(&players, &entities);
        for (id, scope, key, value) in self.lua.scoped_states() {
            if self.known.contains_key(&id) && self.lua.running(&id) {
                server.send_resource(
                    None,
                    Message {
                        scope: serde_json::from_value(scope).map_err(|e|format!("Invalid persisted resource visibility: {e}"))?,
                        id: 1,
                        resource: id.clone(),
                        generation: self.lua.generation(&id).unwrap(),
                        kind: Kind::State,
                        name: key,
                        value,
                    },
                )?;
            }
        }
        Ok(())
    }
    fn fail_closed(&mut self, server: &mut Server) {
        self.lua.disconnect();
        self.sync_services();
        let _ = self.entities.set_terrain(None);
        let _ = self.competition.set_terrain(None);
        self.native_authority.stop();
        if let Ok(empty) = build_set(&self.config.root, &BTreeMap::new()) {
            if self.http.replace(empty.clone()).is_ok() {
                let _ = self.advertise(server, &empty);
                self.published = empty;
            }
        }
    }
    fn register_stopped(&mut self, id: &str) -> Result<(), String> {
        let selected = BTreeMap::from([(id.to_string(), self.lua.generation(id).unwrap_or(1))]);
        let published = skate_resources::build_set_with_limits(
            &self.config.root,
            &selected,
            self.config.content_limits,
        )
        .map_err(|e| e.to_string())?;
        let mut entries = Self::installed(&self.config, &published, &mut self.known)?;
        for entry in &mut entries {
            if let Some(generation) = self.lua.generation(&entry.manifest.id) {
                entry.generation = generation;
            }
        }
        self.lua.register(entries)
    }
    pub fn command(&mut self, line: &str, server: &mut Server) -> Result<String, String> {
        if line.len() > 4096 {
            return Err("Console command too long".into());
        }
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() == Some(&"metrics") && words.len() == 2 {
            let mut metrics = self.lua.runtime_metrics();
            let value = metrics.remove(words[1]).ok_or("No runtime measurements for that resource")?;
            return serde_json::to_string_pretty(&value).map_err(|e| e.to_string());
        }
        if words == ["resources"] {
            return Ok(self
                .known
                .keys()
                .map(|id| {
                    format!(
                        "{id}: {} generation={}",
                        if self.lua.running(id) {
                            "started"
                        } else {
                            "stopped"
                        },
                        self.lua.generation(id).unwrap_or(0)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        if words.first() == Some(&"command") && words.len() >= 2 {
            self.lua.command(
                0,
                words[1],
                words[2..].iter().map(|s| s.to_string()).collect(),
                &BTreeSet::from(["*".into()]),
            )?;
            self.flush(server);
            return Ok("Command executed".into());
        }
        if words.len() != 2 || !matches!(words[0], "start" | "stop" | "restart" | "ensure") {
            return Err(
                "Commands: resources | metrics ID | start/stop/restart/ensure ID | command NAME [args] | quit"
                    .into(),
            );
        }
        let id = words[1];
        if words[0] == "start" && self.lua.running(id) {
            return Ok(format!("{id} already started"));
        }
        if matches!(words[0], "start" | "ensure") && !self.lua.running(id) {
            self.register_stopped(id)?;
        }
        if matches!(words[0], "restart" | "ensure") && self.lua.running(id) {
            // Re-reading script bytes is supported by restart. Manifest changes
            // require an explicit stop/start so dependencies can be revalidated
            // before an installed contract changes under a running VM.
            let mut restarting = BTreeSet::from([id.to_string()]);
            loop {
                let before = restarting.len();
                for (name, entry) in self.lua.installed() {
                    if self.lua.running(name)
                        && entry
                            .manifest
                            .dependencies
                            .keys()
                            .any(|dep| restarting.contains(dep))
                    {
                        restarting.insert(name.clone());
                    }
                }
                if restarting.len() == before {
                    break;
                }
            }
            for name in restarting {
                let current =
                    Manifest::read(&self.config.root.join(&name)).map_err(|e| e.to_string())?;
                if self.lua.installed()[&name].manifest != current {
                    return Err(format!(
                        "{name} manifest changed; stop then start it to validate the new contract"
                    ));
                }
            }
            let selected = self
                .lua
                .running_ids()
                .iter()
                .map(|id| (id.clone(), self.lua.generation(id).unwrap()))
                .collect();
            skate_resources::build_set_with_limits(
                &self.config.root,
                &selected,
                self.config.content_limits,
            )
            .map_err(|e| e.to_string())?;
        }
        let before: BTreeMap<_, _> = self
            .lua
            .running_ids()
            .into_iter()
            .map(|id| {
                let generation = self.lua.generation(&id).unwrap();
                (id, generation)
            })
            .collect();
        let result = match words[0] {
            "start" => self.lua.start(id),
            "stop" => self.lua.stop(id),
            "restart" => self.lua.restart(id),
            _ => self.lua.ensure(id),
        };
        self.lua.drain_retired();
        let after: BTreeMap<_, _> = self
            .lua
            .running_ids()
            .into_iter()
            .map(|id| {
                let generation = self.lua.generation(&id).unwrap();
                (id, generation)
            })
            .collect();
        if before == after {
            return result.map(|()| format!("{} {id} (unchanged)", words[0]));
        }
        if let Err(error) = self.republish(server) {
            self.fail_closed(server);
            return Err(error);
        }
        self.flush(server);
        result?;
        Ok(format!("{} {id}", words[0]))
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn bounded_admin_status(mut status: serde_json::Value) -> serde_json::Value {
    // Serialized bytes include JSON escaping, unlike character or source-byte
    // counts. Leave half the account bridge budget for player inspection.
    for key in ["logs", "diagnostics", "runtime_metrics", "resources"] {
        while serde_json::to_vec(&status).expect("JSON status").len() > 64 * 1024 {
            if status[key].as_array_mut().and_then(|items| items.pop()).is_none() { break; }
            let counter = format!("{key}_omitted");
            let omitted = status[&counter].as_u64().unwrap_or(0) + 1;
            status[&counter] = omitted.into();
        }
    }
    status
}

#[cfg(test)]
mod status_tests {
    #[test]
    fn failed_tick_is_logged_once_before_its_resource_is_unpublished() {
        let root = std::env::temp_dir().join(format!("skate-runtime-log-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let source = root.join("resources/broken");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("resource.json"), r#"{"format":1,"api":1,"id":"broken","version":"1.0.0","language":"lua","server_scripts":["server.lua"]}"#).unwrap();
        std::fs::write(source.join("server.lua"), "return {on_update=function() error('deliberate diagnostic') end}").unwrap();
        let config = root.join("server.json");
        std::fs::write(&config, r#"{"root":"resources","storage":"store","ensure":["broken"]}"#).unwrap();
        let mut host = crate::Host::bind(crate::Options {accounts:None, bind:"127.0.0.1:0".parse().unwrap(),
            session:7, max_players:16, map:crate::Map::TestWorld, resources:Some(config)}).unwrap();
        let platform = host.resources.as_mut().unwrap();
        platform.step(0.016, &mut host.server);
        assert!(!platform.lua.running("broken"));
        assert_eq!(platform.logs.len(), 1);
        assert!(platform.logs[0].1.contains("deliberate diagnostic"));
        platform.step(0.016, &mut host.server);
        assert_eq!(platform.logs.len(), 1, "retired failures must not flood later ticks");
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn escaped_resource_logs_cannot_exhaust_administration_snapshot_budget() {
        let status = serde_json::json!({
            "resources": (0..256).map(|n|serde_json::json!({"id":format!("resource-{n}"),"running":true,"generation":"1"})).collect::<Vec<_>>(),
            "logs": (0..16).map(|_|("source", "\u{0001}".repeat(2048))).collect::<Vec<_>>(),
            "runtime_metrics": [], "runtime_metrics_omitted": 3,
            "diagnostics": []
        });
        let status = super::bounded_admin_status(status);
        assert!(serde_json::to_vec(&status).unwrap().len() <= 64 * 1024);
        assert_eq!(status["resources"].as_array().unwrap().len(), 256);
        assert!(status["logs_omitted"].as_u64().unwrap() > 0);
        assert_eq!(status["runtime_metrics_omitted"], 3);
        assert_eq!(status["logs"][0][0], "source");
    }
}
