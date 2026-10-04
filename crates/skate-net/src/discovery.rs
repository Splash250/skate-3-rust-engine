//! Optional direct-endpoint discovery. Metadata is untrusted and contains no credentials.
use serde::{Deserialize, Serialize};

const QUERY: &[u8; 8] = b"SK8QUERY";
const REPLY: &[u8; 8] = b"SK8INFO1";
pub const PROTOCOL: u32 = 1;
pub const JOIN_STATUS: u8 = 24;
pub const QUERY_BYTES: usize = 1200;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServerInfo {
    pub session: u64,
    pub map_fingerprint: u64,
    pub resource_names: Vec<String>,
    #[serde(default)]
    pub public_settings:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    pub public_settings_omitted: usize,
    pub resources_omitted: usize,
    pub name: String,
    pub map: String,
    pub mode: String,
    pub protocol: u32,
    pub build: String,
    pub players: usize,
    pub capacity: usize,
    pub queued: usize,
    pub maintenance: bool,
    pub accounts_required: bool,
    pub content_revision: String,
    pub content_bytes: u64,
    pub content_resources: usize,
}
impl ServerInfo {
    pub fn valid(&self) -> bool {
        [
            &self.name,
            &self.map,
            &self.mode,
            &self.build,
            &self.content_revision,
        ]
        .iter()
        .all(|s| s.len() <= 128 && !s.chars().any(char::is_control))
            && self.session != 0
            && self.resource_names.len() <= 8
            && self
                .resource_names
                .iter()
                .all(|s| s.len() <= 64 && !s.chars().any(char::is_control))
            && self.resources_omitted <= 256
            && self.public_settings_omitted <= 16384
            && serde_json::to_vec(&self.public_settings).is_ok_and(|v| v.len() <= 2048)
            && self.public_settings.iter().all(|(resource, values)| {
                resource.len() <= 64
                    && !resource.chars().any(char::is_control)
                    && values.iter().all(|(key, value)| {
                        key.len() <= 64
                            && !key.chars().any(char::is_control)
                            && matches!(
                                value,
                                serde_json::Value::Bool(_)
                                    | serde_json::Value::Number(_)
                                    | serde_json::Value::String(_)
                            )
                    })
            })
            && (1..=64).contains(&self.capacity)
            && self.players <= 64
            && self.queued <= 256
            && self.content_resources <= 256
    }
    pub fn compatible(&self) -> bool {
        self.protocol == PROTOCOL && self.build == env!("CARGO_PKG_VERSION")
    }
}

/// Fixed-size request prevents the server being a UDP amplification service.
pub fn query(nonce: u64) -> Vec<u8> {
    let mut out = vec![0; QUERY_BYTES];
    out[..8].copy_from_slice(QUERY);
    out[8..16].copy_from_slice(&nonce.to_le_bytes());
    out
}
pub fn query_nonce(bytes: &[u8]) -> Option<u64> {
    (bytes.len() == QUERY_BYTES && bytes.get(..8)? == QUERY && bytes[16..].iter().all(|b| *b == 0))
        .then(|| u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
}
pub fn response(nonce: u64, info: &ServerInfo) -> Option<Vec<u8>> {
    if !info.valid() {
        return None;
    }
    let mut out = REPLY.to_vec();
    out.extend(nonce.to_le_bytes());
    out.extend(serde_json::to_vec(info).ok()?);
    (out.len() <= QUERY_BYTES).then_some(out)
}
pub fn parse_response(bytes: &[u8], nonce: u64) -> Option<ServerInfo> {
    if bytes.len() > QUERY_BYTES
        || bytes.len() < 16
        || &bytes[..8] != REPLY
        || bytes[8..16] != nonce.to_le_bytes()
    {
        return None;
    }
    let value: ServerInfo = serde_json::from_slice(&bytes[16..]).ok()?;
    value.valid().then_some(value)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JoinStatus {
    pub state: String,
    pub position: usize,
    pub expires_in_ms: u64,
    pub reason: String,
}
impl JoinStatus {
    pub fn valid(&self) -> bool {
        matches!(
            self.state.as_str(),
            "policy"
                | "queued"
                | "admitting"
                | "rejected"
                | "expired"
                | "maintenance"
                | "warning"
                | "clear"
        ) && self.position <= 256
            && self.expires_in_ms <= 86_400_000
            && self.reason.len() <= 256
            && !self.reason.chars().any(char::is_control)
    }
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "rejected" | "expired" | "maintenance")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_are_bounded_nonce_bound_and_untrusted() {
        let info = ServerInfo {
            session: 1,
            map_fingerprint: 2,
            resource_names: vec![],
            public_settings: Default::default(),
            public_settings_omitted: 0,
            resources_omitted: 0,
            name: "Local".into(),
            map: "park".into(),
            mode: "free-skate".into(),
            protocol: PROTOCOL,
            build: env!("CARGO_PKG_VERSION").into(),
            players: 0,
            capacity: 16,
            queued: 0,
            maintenance: false,
            accounts_required: false,
            content_revision: String::new(),
            content_bytes: 0,
            content_resources: 0,
        };
        assert_eq!(query_nonce(&query(7)), Some(7));
        assert!(query_nonce(&query(7)[..16]).is_none());
        let bytes = response(7, &info).unwrap();
        assert!(bytes.len() <= query(7).len());
        assert_eq!(parse_response(&bytes, 7), Some(info.clone()));
        assert!(parse_response(&bytes, 8).is_none());
        let mut invalid = info;
        invalid.name = "untrusted\nlabel".into();
        assert!(response(7, &invalid).is_none());
    }
    #[test]
    fn cancellation_stops_hello_and_status_requires_the_selected_endpoint() {
        use crate::{
            lobby::{DEDICATED_HELLO, GOODBYE, Info, Session},
            packed,
        };
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 9,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 0,
            },
            55,
        );
        let status = JoinStatus {
            state: "queued".into(),
            position: 4,
            expires_in_ms: 1000,
            reason: "Waiting".into(),
        };
        let mut bytes = packed::header(7, 8, JOIN_STATUS, 0);
        bytes.extend(9u64.to_le_bytes());
        bytes.extend(serde_json::to_vec(&status).unwrap());
        client.receive(56, &bytes, 0);
        assert!(client.join_status.is_none());
        client.receive(55, &bytes, 0);
        assert_eq!(client.join_status, Some(status));
        let mut reconnect = Session::dedicated_client(
            7,
            Info {
                id: 10,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 0,
            },
            55,
        );
        reconnect.receive(55, &bytes, 0);
        assert!(
            reconnect.join_status.is_none(),
            "old join replies cannot control a new actor attempt"
        );
        client.cancel_join();
        let packets = client.service(1);
        assert!(
            packets
                .iter()
                .any(|p| packed::envelope(&p.data).unwrap().2 == GOODBYE)
        );
        assert!(
            !packets
                .iter()
                .any(|p| packed::envelope(&p.data).unwrap().2 == DEDICATED_HELLO)
        );
        assert!(
            !client
                .service(2000)
                .iter()
                .any(|p| packed::envelope(&p.data).unwrap().2 == DEDICATED_HELLO)
        );
    }
    #[test]
    fn admitted_session_ignores_delayed_queue_replies_but_keeps_maintenance_warnings() {
        use crate::{
            dedicated::{Config, Server},
            lobby::{Info, Session},
            packed,
        };
        let mut server = Server::new(Config {
            session: 7,
            server_id: 8,
            map: 1,
            max_players: 1,
        })
        .unwrap();
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 9,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 0,
            },
            55,
        );
        for outgoing in client.service(0) {
            server.receive(100, &outgoing.data, 0);
        }
        for outgoing in server.service(1) {
            client.receive(55, &outgoing.data, 1);
        }
        assert!(client.connected());
        for state in ["queued", "admitting", "expired", "rejected", "maintenance"] {
            let status = JoinStatus {
                state: state.into(),
                position: 1,
                expires_in_ms: 1000,
                reason: "Stale reply".into(),
            };
            let mut bytes = packed::header(7, 8, JOIN_STATUS, 0);
            bytes.extend(9u64.to_le_bytes());
            bytes.extend(serde_json::to_vec(&status).unwrap());
            client.receive(55, &bytes, 2);
            assert!(client.join_status.is_none());
        }
        for state in ["warning", "clear"] {
            let status = JoinStatus {
                state: state.into(),
                position: 0,
                expires_in_ms: 1000,
                reason: "Maintenance warning".into(),
            };
            let mut bytes = packed::header(7, 8, JOIN_STATUS, 0);
            bytes.extend(9u64.to_le_bytes());
            bytes.extend(serde_json::to_vec(&status).unwrap());
            client.receive(55, &bytes, 3);
            assert_eq!(client.join_status.is_some(), state == "warning");
        }
    }
    #[test]
    fn cancelled_join_cannot_be_revived_by_an_inflight_roster() {
        use crate::{
            dedicated::{Config, Server},
            lobby::{Info, Session},
        };
        let mut server = Server::new(Config {
            session: 7,
            server_id: 8,
            map: 1,
            max_players: 1,
        })
        .unwrap();
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 9,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 0,
            },
            55,
        );
        for outgoing in client.service(0) {
            server.receive(100, &outgoing.data, 0);
        }
        let delayed = server.service(1);
        client.cancel_join();
        for outgoing in delayed {
            client.receive(55, &outgoing.data, 2);
        }
        assert!(
            !client.connected(),
            "late admission must not undo cancellation"
        );
        assert_eq!(client.notice, "Connection cancelled");
    }
    #[test]
    fn dedicated_rejection_targets_one_attempt_and_preserves_terminal_reason() {
        use crate::{
            lobby::{Info, REJECT, Session},
            packed,
        };
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 9,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 0,
            },
            55,
        );
        let mut bytes = packed::header(7, 8, REJECT, 0);
        bytes.push(2);
        bytes.extend(10u64.to_le_bytes());
        client.receive(55, &bytes, 1);
        assert!(client.join_status.is_none());
        bytes.truncate(packed::HEADER + 1);
        bytes.extend(9u64.to_le_bytes());
        client.receive(55, &bytes, 2);
        assert!(client.join_status.as_ref().unwrap().terminal());
        assert_eq!(client.notice, "Physics definition mismatch");
        client.service(10_000);
        assert_eq!(client.notice, "Physics definition mismatch");
    }
}
