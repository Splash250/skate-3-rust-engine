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
    pub root: PathBuf,
    pub storage: PathBuf,
    pub ensure: Vec<String>,
    #[serde(default)]
    pub grants: BTreeMap<String, BTreeSet<String>>,
}
pub struct Platform {
    config: Config,
    lua: Host,
    http: HttpServer,
    known: BTreeMap<String, Manifest>,
    published: PublishedSet,
}
impl Platform {
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
        config.root = parent.join(&config.root);
        config.storage = parent.join(&config.storage);
        if config.ensure.len() > 32
            || config.ensure.iter().collect::<BTreeSet<_>>().len() != config.ensure.len()
        {
            return Err("Duplicate or too many ensured resources (maximum 32)".into());
        }
        let selected = config.ensure.iter().map(|id| (id.clone(), 1)).collect();
        let published = build_set(&config.root, &selected).map_err(|e| e.to_string())?;
        let scope = std::fs::canonicalize(path)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let mut lua = Host::new(Side::Server, config.storage.clone(), &scope)?;
        let mut known = BTreeMap::new();
        let installed = Self::installed(&config, &published, &mut known)?;
        lua.install(installed)?;
        lua.start_all()?;
        let http = HttpServer::bind(bind, published.clone()).map_err(|e| e.to_string())?;
        let mut platform = Self {
            config,
            lua,
            http,
            known,
            published: published.clone(),
        };
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
    pub fn address(&self) -> SocketAddr {
        self.http.local_addr()
    }
    pub fn shutdown(&mut self) {
        self.lua.disconnect();
    }
    pub fn step(&mut self, dt: f64, server: &mut Server) {
        self.lua.tick(dt,serde_json::json!({"players":server.player_observations(),"network":{"active":true,"is_host":true}}));
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
        // A callback failure stops dependent resources and updates the advertised
        // set, so clients unload them rather than keep a dead game mode alive.
        if !self.lua.drain_retired().is_empty() {
            if let Err(error) = self.republish(server) {
                eprintln!("Resource failure publication: {error}");
                self.fail_closed(server);
            }
        }
    }
    fn flush(&mut self, server: &mut Server) {
        for output in self.lua.drain_outputs() {
            let (recipient, message) = match output {
                Output::Event {
                    resource,
                    generation,
                    recipient,
                    name,
                    payload,
                } => (
                    recipient,
                    Message {
                        id: 1,
                        resource,
                        generation,
                        kind: Kind::Event,
                        name,
                        value: payload,
                    },
                ),
                Output::State {
                    resource,
                    generation,
                    key,
                    value,
                } => (
                    None,
                    Message {
                        id: 1,
                        resource,
                        generation,
                        kind: Kind::State,
                        name: key,
                        value,
                    },
                ),
                Output::Log { resource, text } => {
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
                let fresh = build_set(&self.config.root, &selected).map_err(|e| e.to_string())?;
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
        self.http
            .replace(published.clone())
            .map_err(|e| e.to_string())?;
        self.advertise(server, &published)?;
        self.published = published;
        // Re-send the current authoritative state after resetting transport epochs.
        let mut states = self.lua.states();
        for id in self.known.keys().filter(|id| self.lua.running(id)) {
            for (key, value) in states.remove(id).unwrap_or_default() {
                server.send_resource(
                    None,
                    Message {
                        id: 1,
                        resource: id.clone(),
                        generation: self.lua.generation(id).unwrap(),
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
        if let Ok(empty) = build_set(&self.config.root, &BTreeMap::new()) {
            if self.http.replace(empty.clone()).is_ok() {
                let _ = self.advertise(server, &empty);
                self.published = empty;
            }
        }
    }
    fn register_stopped(&mut self, id: &str) -> Result<(), String> {
        let selected = BTreeMap::from([(id.to_string(), self.lua.generation(id).unwrap_or(1))]);
        let published = build_set(&self.config.root, &selected).map_err(|e| e.to_string())?;
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
                "Commands: resources | start/stop/restart/ensure ID | command NAME [args] | quit"
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
            build_set(&self.config.root, &selected).map_err(|e| e.to_string())?;
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
