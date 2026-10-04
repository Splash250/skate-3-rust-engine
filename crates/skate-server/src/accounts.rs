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
    admitted: BTreeSet<u64>,
    completions: VecDeque<(u64, skate_accounts::Result<String>)>,
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
            admitted: BTreeSet::new(),
            completions: VecDeque::new(),
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
        Some(plain)
    }
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
            platform.set_authenticated_accounts(
                self.peers
                    .values()
                    .filter(|s| s.is_active())
                    .map(|s| (s.actor(), s.account_id().to_string()))
                    .collect(),
            );
        }
    }
    pub fn step(
        &mut self,
        server: &mut skate_net::dedicated::Server,
        resources: &mut Option<crate::resources::Platform>,
        now: u64,
    ) {
        self.peers.retain(|peer, session| {
            if !session.is_active()
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
        self.admitted
            .retain(|actor| self.peers.values().any(|session| session.actor() == *actor));
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
            let result = if !command.session.permits(command.action.permission()) {
                Err(skate_accounts::Error {
                    code: "denied".into(),
                    message: "session or permission revoked before execution".into(),
                })
            } else {
                match command.action {
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
            let status = bounded_status(serde_json::json!({"attached":true,"players":players,"resource_platform":resource_status}));
            // Busy is retried on the next sampling tick; valid snapshots are
            // bounded before publication so they cannot leave stale status.
            let _ = self.bridge.set_status(status);
            self.last_status = Instant::now();
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
