//! Admission and maintenance state. Authentication is supplied only by the host transport.
use serde::{Deserialize, Serialize};
use skate_accounts::VerifiedSession;
use skate_net::{
    dedicated::Server,
    discovery::{JoinStatus, ServerInfo},
    lobby::{DEDICATED_HELLO, GOODBYE, Outgoing},
    packed,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::mpsc,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub name: String,
    pub map: String,
    pub mode: String,
    pub queue_capacity: usize,
    pub queue_timeout_ms: u64,
    pub reserved_slots: usize,
    pub required_permission: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            name: "Skate dedicated server".into(),
            map: "Current map".into(),
            mode: "free-skate".into(),
            queue_capacity: 128,
            queue_timeout_ms: 120_000,
            reserved_slots: 0,
            required_permission: None,
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > 16_384 {
            return Err("Operations configuration must be a regular file <=16 KiB".into());
        }
        let value: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), String> {
        if [&self.name, &self.map, &self.mode]
            .iter()
            .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
            || !(1..=256).contains(&self.queue_capacity)
            || !(1000..=600_000).contains(&self.queue_timeout_ms)
            || self.reserved_slots > 63
            || self.required_permission.as_ref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 128
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
        {
            return Err(
                "Invalid operations name, queue bounds, reserved slots or permission".into(),
            );
        }
        Ok(())
    }
}
struct Entry {
    peer: u64,
    actor: u64,
    packet: Vec<u8>,
    key: String,
    session: Option<VerifiedSession>,
    started: u64,
    seen: u64,
    policy: Option<bool>,
    ticket: u64,
}
struct Check {
    ticket: u64,
    session: Option<VerifiedSession>,
    permission: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Maintenance {
    pub reason: String,
    pub deadline_ms: u64,
    pub restart: bool,
}

pub struct Operations {
    pub config: Config,
    waiting: VecDeque<Entry>,
    rejected: BTreeMap<String, (u64, JoinStatus)>,
    admitted_accounts: BTreeMap<u64, String>,
    next: u64,
    checks: mpsc::SyncSender<Check>,
    results: mpsc::Receiver<(u64, bool)>,
    pub maintenance: Option<Maintenance>,
    pub exit_requested: Option<bool>,
    last_warning: u64,
    clear_warning: bool,
    history: VecDeque<serde_json::Value>,
    discovery_last: BTreeMap<u64, u64>,
}
impl Operations {
    pub fn new(config: Config, capacity: usize) -> Result<Self, String> {
        config.validate()?;
        if config.reserved_slots >= capacity {
            return Err("reserved_slots must be smaller than capacity".into());
        }
        let (send, receive) = mpsc::sync_channel::<Check>(256);
        let (done, results) = mpsc::sync_channel(256);
        std::thread::Builder::new()
            .name("admission-policy".into())
            .spawn(move || {
                while let Ok(check) = receive.recv() {
                    let allowed = check
                        .session
                        .as_ref()
                        .is_some_and(|s| s.permits(&check.permission));
                    if done.send((check.ticket, allowed)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            config,
            waiting: VecDeque::new(),
            rejected: BTreeMap::new(),
            admitted_accounts: BTreeMap::new(),
            next: 1,
            checks: send,
            results,
            maintenance: None,
            exit_requested: None,
            last_warning: 0,
            clear_warning: false,
            history: VecDeque::new(),
            discovery_last: BTreeMap::new(),
        })
    }
    pub fn begin_maintenance(
        &mut self,
        reason: String,
        delay_ms: u64,
        restart: bool,
        now: u64,
    ) -> Result<String, String> {
        if reason.is_empty()
            || reason.len() > 256
            || reason.chars().any(char::is_control)
            || delay_ms > 86_400_000
        {
            return Err("Maintenance requires a bounded reason and delay <=24 hours".into());
        }
        self.maintenance = Some(Maintenance {
            reason,
            deadline_ms: now.saturating_add(delay_ms),
            restart,
        });
        self.last_warning = 0;
        self.record(now, "maintenance_scheduled");
        Ok(
            "Admission closed; existing sessions continue until the deadline or the server empties"
                .into(),
        )
    }
    pub fn resume(&mut self, now: u64) -> Result<String, String> {
        if self.exit_requested.is_some() {
            return Err("Shutdown has already begun".into());
        }
        self.maintenance = None;
        self.clear_warning = true;
        self.record(now, "maintenance_cancelled");
        Ok("Admission reopened".into())
    }
    fn record(&mut self, now: u64, event: &str) {
        self.history
            .push_back(serde_json::json!({"time_ms":now,"event":event}));
        while self.history.len() > 64 {
            self.history.pop_front();
        }
    }
    pub fn status(&self, server: &Server) -> serde_json::Value {
        serde_json::json!({"connections":server.connection_count(),"capacity":server.capacity(),"queued":self.waiting.len(),"reserved_slots":self.config.reserved_slots,"maintenance":self.maintenance,"history":self.history})
    }
    pub fn info(&self, server: &Server, accounts: bool, preview: serde_json::Value) -> ServerInfo {
        let resource_names: Vec<String> = preview["resource_names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .take(8)
            .map(|s| s.chars().take(64).collect())
            .collect();
        let mut info = ServerInfo {
            public_settings: serde_json::from_value(preview["settings_public"].clone())
                .unwrap_or_default(),
            public_settings_omitted: preview["settings_public_omitted"]
                .as_u64()
                .unwrap_or(0)
                .min(16384) as usize,
            session: server.session_id(),
            map_fingerprint: server.map_fingerprint(),
            resources_omitted: preview["resources"].as_u64().unwrap_or(0).min(256) as usize
                - resource_names
                    .len()
                    .min(preview["resources"].as_u64().unwrap_or(0) as usize),
            resource_names,
            name: self.config.name.clone(),
            map: preview["world"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(128).collect())
                .unwrap_or_else(|| self.config.map.clone()),
            mode: self.config.mode.clone(),
            protocol: skate_net::discovery::PROTOCOL,
            build: env!("CARGO_PKG_VERSION").into(),
            players: server.connection_count(),
            capacity: server.capacity(),
            queued: self.waiting.len(),
            maintenance: self.maintenance.is_some(),
            accounts_required: accounts,
            content_revision: preview["revision"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(128)
                .collect(),
            content_bytes: preview["bytes"].as_u64().unwrap_or(0),
            content_resources: preview["resources"].as_u64().unwrap_or(0).min(256) as usize,
        };
        while skate_net::discovery::response(0, &info).is_none() && !info.resource_names.is_empty()
        {
            info.resource_names.pop();
            info.resources_omitted += 1;
        }
        while skate_net::discovery::response(0, &info).is_none() && !info.public_settings.is_empty()
        {
            let resource = info.public_settings.keys().next_back().unwrap().clone();
            let values = info.public_settings.get_mut(&resource).unwrap();
            if let Some(key) = values.keys().next_back().cloned() {
                values.remove(&key);
                info.public_settings_omitted += 1;
            }
            if values.is_empty() {
                info.public_settings.remove(&resource);
            }
        }
        info
    }
    pub fn allow_discovery(&mut self, peer: u64, now: u64) -> bool {
        // Per-source-IP, not source-port, plus a bounded table. No response amplification.
        let ip = peer >> 16;
        self.discovery_last
            .retain(|_, at| now.saturating_sub(*at) < 1000);
        if self.discovery_last.contains_key(&ip) || self.discovery_last.len() >= 128 {
            return false;
        }
        self.discovery_last.insert(ip, now);
        true
    }
    fn key(peer: u64, session: &Option<VerifiedSession>) -> String {
        session
            .as_ref()
            .map(|s| format!("account:{}", s.account_id()))
            .unwrap_or_else(|| format!("endpoint:{peer}"))
    }
    fn notice(server: &Server, peer: u64, actor: u64, status: JoinStatus) -> Outgoing {
        let mut data = packed::header(
            server.session_id(),
            server.server_id(),
            skate_net::discovery::JOIN_STATUS,
            0,
        );
        data.extend(actor.to_le_bytes());
        data.extend(serde_json::to_vec(&status).unwrap());
        Outgoing { peer, data }
    }
    fn status_value(state: &str, position: usize, remaining: u64, reason: &str) -> JoinStatus {
        JoinStatus {
            state: state.into(),
            position,
            expires_in_ms: remaining,
            reason: reason.into(),
        }
    }
    /// Returns true only for packets permitted to enter the existing authority path.
    pub fn receive(
        &mut self,
        server: &Server,
        peer: u64,
        packet: &[u8],
        session: Option<VerifiedSession>,
        now: u64,
        out: &mut Vec<Outgoing>,
    ) -> bool {
        let Some((sid, actor, kind, _)) = packed::envelope(packet) else {
            return false;
        };
        if sid != server.session_id() {
            return false;
        }
        if kind == GOODBYE {
            self.waiting.retain(|e| e.peer != peer || e.actor != actor);
            return true;
        }
        if kind != DEDICATED_HELLO || server.peer_for_actor(actor) == Some(peer) {
            return true;
        }
        // Invalid HELLOs never consume queue or policy capacity. Existing compatibility
        // checks run again when handed to Server::receive.
        if actor == 0 || packet.len() != packed::HEADER + 40 {
            return false;
        }
        let mut reader = packed::Reader(&packet[packed::HEADER..]);
        let (Some(id), Some(_map), Some(rig), Some(physics), Some(_appearance)) = (
            reader.u64(),
            reader.u64(),
            reader.u64(),
            reader.u64(),
            reader.u64(),
        ) else {
            return false;
        };
        if id != actor || rig == 0 || physics == 0 {
            return false;
        }
        if _map != server.map_fingerprint() {
            let mut data = packed::header(
                server.session_id(),
                server.server_id(),
                skate_net::lobby::REJECT,
                0,
            );
            data.push(1);
            data.extend(actor.to_le_bytes());
            out.push(Outgoing { peer, data });
            return false;
        }
        let key = Self::key(peer, &session);
        self.rejected.retain(|_, (until, _)| *until > now);
        if let Some((_, status)) = self.rejected.get(&format!("{key}:{actor}")) {
            out.push(Self::notice(server, peer, actor, status.clone()));
            return false;
        }
        if let Some(maintenance) = &self.maintenance {
            out.push(Self::notice(
                server,
                peer,
                actor,
                Self::status_value("maintenance", 0, 0, &maintenance.reason),
            ));
            return false;
        }
        if let Some(index) = self
            .waiting
            .iter()
            .position(|e| e.key == key && e.peer == peer && e.actor == actor)
        {
            let entry = &mut self.waiting[index];
            entry.seen = now;
            let state = if entry.policy.is_none() {
                "policy"
            } else {
                "queued"
            };
            out.push(Self::notice(
                server,
                peer,
                actor,
                Self::status_value(
                    state,
                    index + 1,
                    self.config
                        .queue_timeout_ms
                        .saturating_sub(now.saturating_sub(entry.started)),
                    "Waiting for admission",
                ),
            ));
            return false;
        }
        // Connections that fit currently free public slots are admission work,
        // not waiting backlog. A burst of local clients must still fill capacity.
        let public_free = server
            .capacity()
            .saturating_sub(self.config.reserved_slots)
            .saturating_sub(server.connection_count());
        if session.is_some()
            && self.waiting.len() >= public_free
            && self.waiting.iter().skip(public_free).any(|e| e.key == key)
        {
            out.push(Self::notice(
                server,
                peer,
                actor,
                Self::status_value("rejected", 0, 0, "Identity already has a queued connection"),
            ));
            return false;
        }
        let anonymous_count = self
            .waiting
            .iter()
            .skip(public_free)
            .filter(|e| e.session.is_none() && e.peer >> 16 == peer >> 16)
            .count();
        if self.waiting.len() >= self.config.queue_capacity
            || (session.is_none() && self.waiting.len() >= public_free && anonymous_count >= 4)
        {
            out.push(Self::notice(
                server,
                peer,
                actor,
                Self::status_value(
                    "rejected",
                    0,
                    0,
                    "Join queue is full or source limit reached",
                ),
            ));
            return false;
        }
        let ticket = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        let policy = if let Some(permission) = &self.config.required_permission {
            if self
                .checks
                .try_send(Check {
                    ticket,
                    session: session.clone(),
                    permission: permission.clone(),
                })
                .is_err()
            {
                out.push(Self::notice(
                    server,
                    peer,
                    actor,
                    Self::status_value("rejected", 0, 0, "Admission policy is busy"),
                ));
                return false;
            }
            None
        } else {
            Some(true)
        };
        self.waiting.push_back(Entry {
            peer,
            actor,
            packet: packet.to_vec(),
            key,
            session,
            started: now,
            seen: now,
            policy,
            ticket,
        });
        out.push(Self::notice(
            server,
            peer,
            actor,
            Self::status_value(
                if policy.is_none() { "policy" } else { "queued" },
                self.waiting.len(),
                self.config.queue_timeout_ms,
                "Waiting for admission",
            ),
        ));
        false
    }
    pub fn step(&mut self, server: &mut Server, now: u64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        self.admitted_accounts
            .retain(|actor, _| server.peer_for_actor(*actor).is_some());
        for _ in 0..256 {
            let Ok((ticket, allowed)) = self.results.try_recv() else {
                break;
            };
            if let Some(entry) = self.waiting.iter_mut().find(|e| e.ticket == ticket) {
                entry.policy = Some(allowed);
            }
        }
        let mut keep = VecDeque::new();
        while let Some(entry) = self.waiting.pop_front() {
            let revoked = entry.session.as_ref().is_some_and(|s| {
                !s.is_active()
                    || self
                        .config
                        .required_permission
                        .as_ref()
                        .is_some_and(|p| !s.permits(p))
            });
            let failure = if self.maintenance.is_some() {
                Some(("maintenance", "Server entered maintenance"))
            } else if revoked || entry.policy == Some(false) {
                Some(("rejected", "Pre-admission policy denied this identity"))
            } else if now.saturating_sub(entry.started) >= self.config.queue_timeout_ms
                || now.saturating_sub(entry.seen) > 5000
            {
                Some(("expired", "Join queue expired; reconnect to try again"))
            } else if entry.policy.is_none() && now.saturating_sub(entry.started) > 3000 {
                Some(("rejected", "Pre-admission policy timed out"))
            } else {
                None
            };
            if let Some((state, reason)) = failure {
                let status = Self::status_value(state, 0, 0, reason);
                out.push(Self::notice(
                    server,
                    entry.peer,
                    entry.actor,
                    status.clone(),
                ));
                if self.rejected.len() >= 256 {
                    if let Some(key) = self.rejected.keys().next().cloned() {
                        self.rejected.remove(&key);
                    }
                }
                self.rejected.insert(
                    format!("{}:{}", entry.key, entry.actor),
                    (now.saturating_add(60_000), status),
                );
            } else {
                keep.push_back(entry);
            }
        }
        self.waiting = keep;
        loop {
            if server.connection_count() >= server.capacity() {
                break;
            }
            // FIFO within each class. Reserved identities may use withheld slots only.
            let public = server.connection_count()
                < server.capacity().saturating_sub(self.config.reserved_slots);
            let candidate = self.waiting.iter().position(|e| {
                e.policy == Some(true)
                    && (public
                        || e.session.as_ref().is_some_and(|s| {
                            s.permits("admission.reserved")
                                && !self
                                    .admitted_accounts
                                    .values()
                                    .any(|account| account == s.account_id())
                        }))
            });
            let Some(index) = candidate else {
                break;
            };
            let entry = self.waiting.remove(index).unwrap();
            out.push(Self::notice(
                server,
                entry.peer,
                entry.actor,
                Self::status_value(
                    "admitting",
                    0,
                    0,
                    "Admission accepted; checking compatibility and content",
                ),
            ));
            server.receive(entry.peer, &entry.packet, now);
            if server.peer_for_actor(entry.actor) == Some(entry.peer) {
                if let Some(session) = entry.session {
                    self.admitted_accounts
                        .insert(entry.actor, session.account_id().into());
                }
            }
        }
        if self.clear_warning {
            for actor in server
                .player_observations()
                .as_array()
                .into_iter()
                .flatten()
            {
                if let Some(peer) = actor["id"]
                    .as_str()
                    .and_then(|v| v.parse().ok())
                    .and_then(|id| server.peer_for_actor(id))
                {
                    out.push(Self::notice(
                        server,
                        peer,
                        actor["id"].as_str().unwrap().parse().unwrap(),
                        Self::status_value("clear", 0, 0, ""),
                    ));
                }
            }
            self.clear_warning = false;
        }
        if let Some(maintenance) = &self.maintenance {
            if self.last_warning == 0 || now.saturating_sub(self.last_warning) >= 1000 {
                for actor in server
                    .player_observations()
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(peer) = actor["id"]
                        .as_str()
                        .and_then(|v| v.parse().ok())
                        .and_then(|id| server.peer_for_actor(id))
                    {
                        out.push(Self::notice(
                            server,
                            peer,
                            actor["id"].as_str().unwrap().parse().unwrap(),
                            Self::status_value(
                                "warning",
                                0,
                                maintenance.deadline_ms.saturating_sub(now),
                                &maintenance.reason,
                            ),
                        ));
                    }
                }
                self.last_warning = now.max(1);
            }
            if now >= maintenance.deadline_ms || server.connection_count() == 0 {
                self.exit_requested = Some(maintenance.restart);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(capacity: usize) -> Server {
        Server::new(skate_net::dedicated::Config {
            session: 99,
            server_id: 1,
            map: 5,
            max_players: capacity,
        })
        .unwrap()
    }
    fn hello(actor: u64) -> Vec<u8> {
        let mut bytes = packed::header(99, actor, DEDICATED_HELLO, 0);
        for value in [actor, 5, 2, 3, 0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }
    fn enqueue(ops: &mut Operations, server: &Server, peer: u64, actor: u64, now: u64) {
        assert!(!ops.receive(server, peer, &hello(actor), None, now, &mut vec![]));
    }
    #[test]
    fn fifo_cancellation_capacity_changes_and_expiry() {
        let mut server = server(1);
        let mut ops = Operations::new(Config::default(), 1).unwrap();
        enqueue(&mut ops, &server, 100, 10, 0);
        ops.step(&mut server, 0);
        assert_eq!(server.connection_count(), 1);
        enqueue(&mut ops, &server, 200, 20, 1);
        enqueue(&mut ops, &server, 300, 30, 2);
        assert_eq!(ops.waiting.len(), 2);
        ops.receive(
            &server,
            200,
            &packed::header(99, 20, GOODBYE, 0),
            None,
            3,
            &mut vec![],
        );
        assert_eq!(ops.waiting.len(), 1);
        server.set_capacity(2).unwrap();
        ops.step(&mut server, 4);
        assert_eq!(server.peer_for_actor(30), Some(300));
        enqueue(&mut ops, &server, 400, 40, 5);
        ops.step(&mut server, 6000);
        assert!(ops.waiting.is_empty());
        assert!(
            ops.rejected
                .values()
                .any(|(_, status)| status.state == "expired")
        );
    }
    #[test]
    fn anonymous_cannot_claim_reservation_and_maintenance_closes_admission() {
        let mut server = server(2);
        let mut config = Config::default();
        config.reserved_slots = 1;
        let mut ops = Operations::new(config, 2).unwrap();
        enqueue(&mut ops, &server, 100, 10, 0);
        enqueue(&mut ops, &server, 200, 20, 0);
        ops.step(&mut server, 0);
        assert_eq!(server.connection_count(), 1);
        assert_eq!(ops.waiting.len(), 1);
        ops.begin_maintenance("Restart warning".into(), 1000, true, 1)
            .unwrap();
        ops.step(&mut server, 2);
        assert!(ops.waiting.is_empty());
        assert_eq!(ops.exit_requested, None);
        ops.step(&mut server, 1001);
        assert_eq!(ops.exit_requested, Some(true));
    }
    #[test]
    fn resume_clears_existing_warning_and_reopens_admission() {
        let mut server = server(2);
        let mut ops = Operations::new(Config::default(), 2).unwrap();
        enqueue(&mut ops, &server, 100, 10, 0);
        ops.step(&mut server, 0);
        ops.begin_maintenance("Restart warning".into(), 1000, true, 1)
            .unwrap();
        let notices = ops.step(&mut server, 2);
        assert!(!notices.is_empty());
        ops.resume(3).unwrap();
        let notices = ops.step(&mut server, 3);
        let clear = notices.iter().find(|n| n.peer == 100).unwrap();
        let value: JoinStatus = serde_json::from_slice(&clear.data[packed::HEADER + 8..]).unwrap();
        assert_eq!(value.state, "clear");
        assert!(ops.maintenance.is_none());
        enqueue(&mut ops, &server, 200, 20, 4);
        ops.step(&mut server, 4);
        assert_eq!(server.connection_count(), 2);
        assert_eq!(ops.exit_requested, None);
    }
    #[test]
    fn queue_and_anonymous_source_are_bounded_and_map_mismatch_uses_no_slot() {
        let mut server = server(2);
        let mut ops = Operations::new(Config::default(), 2).unwrap();
        server.receive(100, &hello(100), 0);
        server.receive(200, &hello(200), 0);
        for actor in 10..20 {
            enqueue(&mut ops, &server, actor, actor, 0);
        }
        assert_eq!(ops.waiting.len(), 4);
        let mut mismatch = hello(99);
        mismatch[packed::HEADER + 8] = 6;
        let mut out = vec![];
        assert!(!ops.receive(&server, 999999, &mismatch, None, 0, &mut out));
        assert_eq!(out[0].data[packed::HEADER], 1);
        assert_eq!(ops.waiting.len(), 4);
    }
    #[test]
    fn discovery_uses_current_world_and_bounds_public_settings() {
        let server = server(2);
        let ops = Operations::new(Config::default(), 2).unwrap();
        let preview = serde_json::json!({"world":"rotated-park","resources":1,"resource_names":["rotated-park"],
            "settings_public":{"rounds":{"map":"rotated-park","duration":120}},"revision":"abc","bytes":100});
        let info = ops.info(&server, false, preview);
        assert_eq!(info.map, "rotated-park");
        assert_eq!(info.public_settings["rounds"]["duration"], 120);
        assert!(
            skate_net::discovery::response(1, &info).unwrap().len()
                <= skate_net::discovery::QUERY_BYTES
        );
    }
    #[test]
    fn verified_reservations_async_policy_and_revocation_are_rechecked() {
        let path = std::env::temp_dir().join(format!(
            "skate-admission-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        let store = skate_accounts::AccountStore::open(path.join("accounts.sqlite3")).unwrap();
        store
            .bootstrap_admin("administrator", "test-password-12345")
            .unwrap();
        let credentials = store.login("administrator", "test-password-12345").unwrap();
        let verified = store.authenticate(&credentials.token).unwrap();
        let mut config = Config::default();
        config.reserved_slots = 2;
        config.required_permission = Some("join.allowed".into());
        let mut ops = Operations::new(config, 3).unwrap();
        let mut server = server(3);
        server.receive(100, &hello(10), 0);
        assert!(!ops.receive(
            &server,
            200,
            &hello(verified.actor()),
            Some(verified.clone()),
            1,
            &mut vec![]
        ));
        for now in 2..100 {
            ops.step(&mut server, now);
            if server.connection_count() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            server.connection_count(),
            2,
            "verified permission grants the reserved slot after async policy"
        );
        let credentials2 = store.login("administrator", "test-password-12345").unwrap();
        let revoked = store.authenticate(&credentials2.token).unwrap();
        ops.receive(
            &server,
            300,
            &hello(revoked.actor()),
            Some(revoked.clone()),
            101,
            &mut vec![],
        );
        for now in 102..120 {
            ops.step(&mut server, now);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            server.connection_count(),
            2,
            "one account cannot consume multiple reserved admissions"
        );
        assert_eq!(ops.waiting.len(), 1);
        revoked.revoke_connection();
        ops.step(&mut server, 102);
        assert!(ops.waiting.is_empty());
        assert_eq!(server.peer_for_actor(revoked.actor()), None);
        drop(ops);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
