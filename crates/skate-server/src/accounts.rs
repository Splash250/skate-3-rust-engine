//! Host boundary for local identities. No password verification or SQL runs in `step`.
use serde::Deserialize;
use skate_accounts::{
    AccountStore, AdminBridge, AdminConfig, AdminServer, HostAction, ServerTransport,
    VerifiedSession,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    database: PathBuf,
    bind: SocketAddr,
    certificate: PathBuf,
    key: PathBuf,
}
pub(crate) struct Accounts {
    pub bridge: AdminBridge,
    transport: ServerTransport,
    listener: AdminServer,
    peers: BTreeMap<u64, VerifiedSession>,
    peer_seen: BTreeMap<u64, Instant>,
    admitted: BTreeSet<u64>,
    completions: VecDeque<(u64, skate_accounts::Result<String>)>,
    gameplay_admin: BTreeMap<u64, (crate::resources::admin::Request, VerifiedSession)>,
    last_status: Instant,
}
impl Accounts {
    pub fn load(path: &Path) -> Result<Self, String> {
        let metadata =
            std::fs::metadata(path).map_err(|e| format!("Account configuration: {e}"))?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 {
            return Err("Account configuration must be a regular file at most 64 KiB".into());
        }
        let config: Config =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Invalid account configuration: {e}"))?;
        let parent = path.parent().unwrap_or(Path::new("."));
        let resolve = |p: PathBuf| if p.is_absolute() { p } else { parent.join(p) };
        let store = AccountStore::open(resolve(config.database)).map_err(|e| e.to_string())?;
        let transport = ServerTransport::new(store.clone()).map_err(|e| e.to_string())?;
        let bridge = AdminBridge::new();
        let listener = AdminServer::bind(
            store,
            AdminConfig {
                bind: config.bind,
                certificate: resolve(config.certificate),
                key: resolve(config.key),
            },
            bridge.clone(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            bridge,
            transport,
            listener,
            peers: BTreeMap::new(),
            peer_seen: BTreeMap::new(),
            admitted: BTreeSet::new(),
            completions: VecDeque::new(),
            gameplay_admin: BTreeMap::new(),
            last_status: Instant::now() - Duration::from_secs(1),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.listener.local_addr()
    }
    pub fn decode(&mut self, peer: u64, packet: &[u8]) -> Option<Vec<u8>> {
        let (session, plain) = self.transport.decode(packet).ok()?;
        let (_, actor, _, _) = skate_net::packed::envelope(&plain)?;
        // The stable account is issued by TLS login; Info.id is only an envelope
        // field and can never authenticate a player or choose their permissions.
        if actor != session.actor() {
            return None;
        }
        if self
            .peers
            .get(&peer)
            .is_some_and(|current| current.actor() != actor)
        {
            return None;
        }
        if self
            .peers
            .iter()
            .any(|(address, current)| *address != peer && current.actor() == actor)
        {
            return None;
        }
        if self.peers.len() >= 256 && !self.peers.contains_key(&peer) {
            return None;
        }
        self.peers.insert(peer, session);
        self.peer_seen.insert(peer,Instant::now());
        Some(plain)
    }
    pub fn verified(&self, peer:u64)->Option<VerifiedSession> {self.peers.get(&peer).filter(|s|s.is_active()).cloned()}
    pub fn encode(&mut self, peer: u64, payload: &[u8]) -> Option<Vec<u8>> {
        self.transport.encode(self.peers.get(&peer)?, payload).ok()
    }
    pub fn sync_identity(
        &mut self,
        resources: &mut Option<crate::resources::Platform>,
        server: &skate_net::dedicated::Server,
    ) {
        for (peer, session) in &self.peers {
            if server.peer_for_actor(session.actor()) == Some(*peer) {
                self.admitted.insert(session.actor());
            }
        }
        if let Some(platform) = resources {
            if let Err(error) = platform.set_authenticated_sessions(
                self.peers
                    .iter()
                    .filter(|(peer, session)| session.is_active() && server.peer_for_actor(session.actor()) == Some(**peer))
                    .map(|(_,session)| (session.actor(), session.clone()))
                    .collect(),
            ) { eprintln!("Resource authorization update: {error}"); }
        }
    }
    pub fn step(
        &mut self,
        server: &mut skate_net::dedicated::Server,
        resources: &mut Option<crate::resources::Platform>,
        operations: &mut crate::operations::Operations,
        now: u64,
    ) {
        self.peers.retain(|peer, session| {
            if !session.is_active()
                || (!self.admitted.contains(&session.actor()) && self.peer_seen.get(peer).is_some_and(|seen|seen.elapsed()>Duration::from_secs(15)))
                || (self.admitted.contains(&session.actor())
                    && server.peer_for_actor(session.actor()) != Some(*peer))
            {
                session.revoke_connection();
                server.kick(session.actor(), now);
                false
            } else {
                true
            }
        });
        self.peer_seen.retain(|peer,_|self.peers.contains_key(peer));
        self.admitted
            .retain(|actor| self.peers.values().any(|session| session.actor() == *actor));
        self.resource_admin(server, resources);
        // Retain a failed try-lock completion; execute each host action only once.
        while let Some((ticket, result)) = self.completions.front() {
            if self.bridge.complete(*ticket, result.clone()).is_err() {
                break;
            }
            self.completions.pop_front();
        }
        for _ in 0..4 {
            if self.completions.len() >= 64 {
                break;
            }
            let Some(command) = self.bridge.try_command() else {
                break;
            };
            let live_interface = self.gameplay_admin.get(&command.ticket).is_none_or(|(request, session)| {
                session.is_active() && resources.as_ref().is_some_and(|platform|platform.admin_request_live(request))
                    && server.peer_for_actor(request.sender).is_some_and(|peer|self.peers.get(&peer).is_some_and(|active|active.actor()==session.actor()))
            });
            let result = if !live_interface || !command.session.permits(command.action.permission()) {
                Err(skate_accounts::Error {
                    code: "denied".into(),
                    message: "session or permission revoked before execution".into(),
                })
            } else {
                let host_error = |message:String| skate_accounts::Error {code:"host".into(),message};
                match command.action {
                    HostAction::StatusRead {} => {
                        let resource_status=resources.as_ref().map(|platform|platform.admin_status()).unwrap_or_else(||serde_json::json!({"resources":[]}));
                        serde_json::to_string(&resource_status).map_err(|error|host_error(error.to_string()))
                    }
                    HostAction::Maintenance {reason,delay_ms,restart} => {
                        if crate::supervision::pending() {Err(host_error("A stopped-store operation is already draining".into()))}
                        else if restart && !crate::supervision::available() {Err(host_error("Restart requires the external supervisor".into()))}
                        else {
                            let result=if crate::supervision::available() {crate::supervision::request(if restart {"restart"} else {"shutdown"},None).map(|_|())}else{Ok(())};
                            result.and_then(|()|operations.begin_maintenance(reason,delay_ms,restart,now)).map_err(host_error)
                        }
                    },
                    HostAction::Resume {} => {
                        if crate::supervision::pending_store() {Err(host_error("A stopped-store operation is already draining".into()))}
                        else {crate::supervision::cancel_restart();operations.resume(now).map_err(host_error)}
                    },
                    HostAction::Capacity {players} => {
                        if players<=operations.config.reserved_slots {Err(host_error("Capacity must exceed reserved slots".into()))}
                        else {server.set_capacity(players).map(|()|"Capacity updated without disconnecting existing sessions".into()).map_err(host_error)}
                    }
                    HostAction::Backup {snapshot} => {
                        let kind = "backup";
                        crate::supervision::request(kind,Some(&snapshot)).and_then(|message| {
                            operations.begin_maintenance(format!("Server {kind}; reconnect after restart"),10_000,true,now)?; Ok(message)
                        }).map_err(host_error)
                    }
                    HostAction::Restore {snapshot} => {
                        crate::supervision::request("restore",Some(&snapshot)).and_then(|message| {
                            operations.begin_maintenance("Server restore; reconnect after restart".into(),10_000,true,now)?; Ok(message)
                        }).map_err(host_error)
                    }
                    HostAction::SettingsRead {resource} => resources.as_ref().ok_or_else(||"Resources are not configured".to_string()).and_then(|p|p.settings_read(&resource)).map_err(host_error),
                    HostAction::SettingsSet {resource,key,value} => resources.as_mut().ok_or_else(||"Resources are not configured".to_string()).and_then(|p|p.settings_set(&resource,&key,value)).map_err(host_error),
                    HostAction::ProfileRead {resource} => resources.as_ref().ok_or_else(||"Resources are not configured".to_string()).and_then(|p|p.profile_read(resource.as_deref())).map_err(host_error),
                    HostAction::ProfileExport {} => resources.as_ref().ok_or_else(||"Resources are not configured".to_string()).and_then(|p|p.profile_export()).map_err(host_error),
                    HostAction::Kick { actor } => {
                        if server.kick(actor, now) {
                            for session in self.peers.values().filter(|s| s.actor() == actor) {
                                session.revoke_connection();
                            }
                            Ok("Player disconnected".into())
                        } else {
                            Err(skate_accounts::Error {
                                code: "missing".into(),
                                message: "Player is not connected".into(),
                            })
                        }
                    }
                    action => {
                        let (verb, resource) = match action {
                            HostAction::ResourceStart { resource } => ("start", resource),
                            HostAction::ResourceStop { resource } => ("stop", resource),
                            HostAction::ResourceRestart { resource } => ("restart", resource),
                            _ => unreachable!(),
                        };
                        resources
                            .as_mut()
                            .ok_or_else(|| "Resources are not configured".to_string())
                            .and_then(|p| p.command(&format!("{verb} {resource}"), server))
                            .map_err(|message| skate_accounts::Error {
                                code: "host".into(),
                                message,
                            })
                    }
                }
            };
            self.completions.push_back((command.ticket, result));
        }
        if self.last_status.elapsed() >= Duration::from_millis(250) {
            let mut players = server.player_observations();
            if let Some(players) = players.as_array_mut() {
                for player in players {
                    let actor = player
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u64>().ok());
                    if let Some(session) = self.peers.values().find(|s| Some(s.actor()) == actor) {
                        player["account_id"] = serde_json::json!(session.account_id());
                    }
                }
            }
            let resource_status = resources
                .as_ref()
                .map(|p| p.admin_status())
                .unwrap_or_else(|| serde_json::json!({"resources":[],"logs":[]}));
            let status = bounded_status(serde_json::json!({"attached":true,"players":players,"resource_platform":resource_status,"operations":operations.status(server),"supervisor":crate::supervision::status()}));
            // Busy is retried on the next sampling tick; valid snapshots are
            // bounded before publication so they cannot leave stale status.
            let _ = self.bridge.set_status(status);
            self.last_status = Instant::now();
        }
    }

    fn resource_admin(&mut self, server: &mut skate_net::dedicated::Server, resources: &mut Option<crate::resources::Platform>) {
        let Some(platform)=resources.as_mut() else { return; };
        for request in platform.take_admin_requests() {
            let session=server.peer_for_actor(request.sender).and_then(|peer|self.peers.get(&peer))
                .filter(|session|session.actor()==request.sender && session.is_active()).cloned();
            let Some(session)=session else {
                platform.finish_admin_request(&request,Err(crate::resources::admin::error("Verified gameplay session required.")),server);
                continue;
            };
            if !platform.admin_request_live(&request) {
                platform.finish_admin_request(&request,Err(crate::resources::admin::error("Interface generation retired.")),server);
                continue;
            }
            if let Some(action)=request.action.host_action() {
                if self.gameplay_admin.len()>=64 {
                    platform.finish_admin_request(&request,Err(crate::resources::admin::error("Administration request queue is full.")),server);
                    continue;
                }
                match self.bridge.enqueue_gameplay(session.clone(),action) {
                    Ok(ticket)=>{self.gameplay_admin.insert(ticket,(request,session));},
                    Err(error)=>platform.finish_admin_request(&request,Err(error),server),
                }
            } else {
                platform.finish_admin_request(&request,Ok(crate::resources::admin::permissions(&session,&request.action)),server);
            }
        }
        let tickets:Vec<_>=self.gameplay_admin.keys().copied().collect();
        for ticket in tickets {
            let (request,session)=&self.gameplay_admin[&ticket];
            let live=platform.admin_request_live(request) && session.is_active()
                && server.peer_for_actor(request.sender).is_some_and(|peer|self.peers.get(&peer).is_some_and(|active|active.actor()==session.actor()));
            let completion = if !live {
                if self.bridge.cancel_gameplay(session,ticket).is_err() {continue;}
                Some(Err(crate::resources::admin::error("Request session or interface retired.")))
            } else {
                match self.bridge.try_gameplay_result(session,ticket) {
                    Ok(None)=>None,
                    Ok(Some(result))=>Some(result.map(|text|serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)))),
                    Err(error) if error.code=="busy"=>None,
                    Err(error)=>{
                        if self.bridge.cancel_gameplay(session,ticket).is_err() {continue;}
                        Some(Err(error))
                    }
                }
            };
            if let Some(result)=completion {
                let (request,_)=self.gameplay_admin.remove(&ticket).unwrap();
                platform.finish_admin_request(&request,result,server);
            }
        }
    }
}

fn bounded_status(mut status: serde_json::Value) -> serde_json::Value {
    if serde_json::to_vec(&status).expect("JSON status").len() <= 128 * 1024 { return status; }
    let mut omitted = 0;
    if let Some(players) = status["players"].as_array_mut() {
        for player in players {
            if player.as_object_mut().and_then(|p|p.remove("gameplay")).is_some() { omitted += 1; }
        }
    }
    status["player_gameplay_omitted"] = omitted.into();
    if serde_json::to_vec(&status).expect("JSON status").len() > 128 * 1024 {
        // Defensive fallback for future additions: report a current bounded
        // error instead of silently retaining an obsolete status snapshot.
        return serde_json::json!({"attached":true,"status_error":"snapshot exceeded administration budget",
            "player_count":status["players"].as_array().map_or(0,Vec::len)});
    }
    status
}

#[cfg(test)]
mod tests {
    #[test]
    fn large_optional_gameplay_details_do_not_hide_current_player_identities() {
        let value = serde_json::json!({"attached":true,
            "players":(0..64).map(|id|serde_json::json!({"id":id.to_string(),"instance":"0",
                "gameplay":{"trick":"\u{0001}".repeat(256),"landed_trick":"\u{0001}".repeat(256)}})).collect::<Vec<_>>(),
            "resource_platform":{"logs":["x".repeat(60000)]}});
        let status = super::bounded_status(value);
        assert!(serde_json::to_vec(&status).unwrap().len() <= 128 * 1024);
        assert_eq!(status["players"].as_array().unwrap().len(),64);
        assert_eq!(status["players"][63]["id"],"63");
        assert_eq!(status["player_gameplay_omitted"],64);
        assert_eq!(status["attached"],true);
    }
}
