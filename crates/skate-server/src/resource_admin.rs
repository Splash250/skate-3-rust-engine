//! Narrow in-game administration transport. The native host intercepts this
//! reserved lane before Lua; identity always comes from the authenticated sender.
use serde::Deserialize;
use serde_json::{Value, json};
use skate_accounts::{HostAction, VerifiedSession};
use skate_net::{
    dedicated::Server,
    resources::{Kind, Message},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

const MAX_RESPONSE: usize = 15 * 1024;
const PERMISSIONS: &[&str] = &[
    "status.read",
    "settings.read",
    "settings.write",
    "resources.manage",
    "profile.read",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Permissions {
        #[serde(default)]
        check: Vec<String>,
    },
    Status,
    SettingsRead {
        resource: String,
    },
    SettingsSet {
        resource: String,
        key: String,
        value: Value,
    },
    ResourceStart {
        resource: String,
    },
    ResourceStop {
        resource: String,
    },
    ResourceRestart {
        resource: String,
    },
    ProfileRead {
        resource: Option<String>,
    },
}
impl Action {
    pub(crate) fn host_action(&self) -> Option<HostAction> {
        Some(match self {
            Self::Permissions { .. } => return None,
            Self::Status => HostAction::StatusRead {},
            Self::SettingsRead { resource } => HostAction::SettingsRead {
                resource: resource.clone(),
            },
            Self::SettingsSet {
                resource,
                key,
                value,
            } => HostAction::SettingsSet {
                resource: resource.clone(),
                key: key.clone(),
                value: value.clone(),
            },
            Self::ResourceStart { resource } => HostAction::ResourceStart {
                resource: resource.clone(),
            },
            Self::ResourceStop { resource } => HostAction::ResourceStop {
                resource: resource.clone(),
            },
            Self::ResourceRestart { resource } => HostAction::ResourceRestart {
                resource: resource.clone(),
            },
            Self::ProfileRead { resource } => HostAction::ProfileRead {
                resource: resource.clone(),
            },
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    seq: String,
    action: Action,
}
#[derive(Clone)]
pub(crate) struct Request {
    pub sender: u64,
    pub resource: String,
    pub generation: u64,
    pub seq: String,
    pub action: Action,
}
type Key = (u64, String, u64, String);
impl Request {
    fn key(&self) -> Key {
        (
            self.sender,
            self.resource.clone(),
            self.generation,
            self.seq.clone(),
        )
    }
}
#[derive(Default)]
pub(super) struct State {
    queue: VecDeque<Request>,
    pending: BTreeSet<Key>,
    rates: BTreeMap<u64, (Instant, u8)>,
}
fn parse(sender: u64, message: &Message) -> Result<Request, String> {
    let wire: WireRequest = serde_json::from_value(message.value.clone())
        .map_err(|_| "Invalid administration request.".to_string())?;
    if let Action::Permissions { check } = &wire.action {
        if check.len() > 128
            || check.iter().any(|name| {
                name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-*".contains(&b))
            })
        {
            return Err("Permission checks require at most 128 bounded permission names.".into());
        }
    }
    if wire.seq.is_empty()
        || wire.seq.len() > 32
        || !wire
            .seq
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(
            "Request sequence must be 1–32 letters, digits, underscores or hyphens.".into(),
        );
    }
    Ok(Request {
        sender,
        resource: message.resource.clone(),
        generation: message.generation,
        seq: wire.seq,
        action: wire.action,
    })
}
pub(crate) fn permissions(session: &VerifiedSession, action: &Action) -> Value {
    let mut permitted: BTreeSet<&str> = PERMISSIONS
        .iter()
        .copied()
        .filter(|permission| session.permits(permission))
        .collect();
    if let Action::Permissions { check } = action {
        permitted.extend(
            check
                .iter()
                .map(String::as_str)
                .filter(|permission| session.permits(permission)),
        );
    }
    json!({"permissions":permitted})
}
fn bounded_view(action: &Action, mut value: Value) -> Value {
    // Interactive status remains usable when legal log/profile history fills the
    // larger native administration budget. Never truncate typed settings data.
    let fields: &[(&str, &str)] = match action {
        Action::Status => &[
            ("logs", "logs_omitted"),
            ("diagnostics", "diagnostics_omitted"),
            ("runtime_metrics", "runtime_metrics_omitted"),
            ("resources", "resources_omitted"),
        ],
        Action::ProfileRead { .. } => &[
            ("spans", "history_omitted"),
            ("summaries", "summaries_omitted"),
        ],
        _ => return value,
    };
    for (field, counter) in fields {
        while serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > 14 * 1024) {
            let Some(items) = value[*field].as_array_mut() else {
                break;
            };
            if items.is_empty() {
                break;
            }
            if *field == "spans" {
                items.remove(0);
            } else {
                items.pop();
            }
            value[*counter] = json!(value[*counter].as_u64().unwrap_or(0) + 1);
        }
    }
    value
}
pub(crate) fn response(seq: &str, result: skate_accounts::Result<Value>) -> Value {
    let value = match result {
        Ok(value) => json!({"seq":seq,"ok":true,"value":value}),
        Err(error) => json!({"seq":seq,"ok":false,"error":error}),
    };
    if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= MAX_RESPONSE) {
        value
    } else {
        json!({"seq":seq,"ok":false,"error":{"code":"too_large","message":"Response exceeds the in-game 16 KiB limit. Select one resource or use the local administration interface."}})
    }
}
pub(crate) fn error(message: &str) -> skate_accounts::Error {
    skate_accounts::Error {
        code: "denied".into(),
        message: message.into(),
    }
}

impl super::Platform {
    pub(crate) fn admin_request_live(&self, request: &Request) -> bool {
        self.lua.running(&request.resource)
            && self.lua.generation(&request.resource) == Some(request.generation)
            && self
                .lua
                .installed()
                .get(&request.resource)
                .is_some_and(|installed| {
                    ["resource.admin", "resource.network"]
                        .iter()
                        .all(|capability| {
                            installed.grants.contains(*capability)
                                && installed
                                    .manifest
                                    .capabilities
                                    .iter()
                                    .any(|value| value.as_str() == *capability)
                        })
                })
            && self.authenticated_accounts.contains_key(&request.sender)
    }
    pub(super) fn intercept_admin(
        &mut self,
        sender: u64,
        message: &Message,
        server: &mut Server,
    ) -> bool {
        if message.name == "__host_admin_result" {
            return true;
        }
        if message.name != "__host_admin" {
            return false;
        }
        let request = match parse(sender, message) {
            Ok(request) => request,
            Err(_) => return true, // Invalid/unbounded correlation IDs are never echoed.
        };
        let failure = if !self.admin_request_live(&request) {
            Some(
                "Verified login and a live resource with explicit administration grants are required.",
            )
        } else if self.admin.pending.len() >= 64
            || self
                .admin
                .pending
                .iter()
                .filter(|key| key.0 == sender)
                .count()
                >= 4
        {
            Some("Administration queue is busy. Wait for outstanding requests.")
        } else if self.admin.pending.contains(&request.key()) {
            Some("That administration request is already pending.")
        } else {
            None
        };
        if let Some(message) = failure {
            self.send_admin_response(
                &request,
                response(&request.seq, Err(error(message))),
                server,
            );
            return true;
        }
        self.admin
            .rates
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(1));
        if !self.admin.rates.contains_key(&sender) && self.admin.rates.len() >= 256 {
            return true;
        }
        let rate = self
            .admin
            .rates
            .entry(sender)
            .or_insert((Instant::now(), 0));
        if rate.1 >= 8 {
            self.send_admin_response(
                &request,
                response(
                    &request.seq,
                    Err(error(
                        "Administration request rate exceeded. Retry in one second.",
                    )),
                ),
                server,
            );
            return true;
        }
        rate.1 += 1;
        self.admin.pending.insert(request.key());
        self.admin.queue.push_back(request);
        true
    }
    pub(crate) fn take_admin_requests(&mut self) -> Vec<Request> {
        let count = self.admin.queue.len().min(4);
        self.admin.queue.drain(..count).collect()
    }
    pub(crate) fn finish_admin_request(
        &mut self,
        request: &Request,
        result: skate_accounts::Result<Value>,
        server: &mut Server,
    ) {
        self.admin.pending.remove(&request.key());
        if self.admin_request_live(request) {
            self.send_admin_response(
                request,
                response(
                    &request.seq,
                    result.map(|value| bounded_view(&request.action, value)),
                ),
                server,
            );
        }
    }
    fn send_admin_response(&self, request: &Request, mut value: Value, server: &mut Server) {
        let limit = self
            .config
            .network_budgets
            .value_bytes
            .min(self.config.runtime_limits.max_payload_bytes)
            .min(MAX_RESPONSE);
        if serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > limit) {
            value = json!({"seq":request.seq,"ok":false,"error":{"code":"too_large","message":"Response exceeds the configured in-game message limit."}});
        }
        // A single actor recipient is mandatory, even for errors and permissions.
        let _ = server.send_resource(
            Some(request.sender),
            Message {
                scope: Default::default(),
                id: 1,
                resource: request.resource.clone(),
                generation: request.generation,
                kind: Kind::Event,
                name: "__host_admin_result".into(),
                value,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(value: Value) -> Message {
        Message {
            scope: Default::default(),
            id: 1,
            resource: "interface".into(),
            generation: 7,
            kind: Kind::Event,
            name: "__host_admin".into(),
            value,
        }
    }
    #[test]
    fn sender_is_host_owned_and_payload_cannot_choose_an_identity() {
        let request = parse(
            42,
            &message(json!({"seq":"a1","action":{"kind":"settings_read","resource":"example"}})),
        )
        .unwrap();
        assert_eq!(request.sender, 42);
        assert_eq!(request.generation, 7);
        for field in ["sender", "account_id", "role"] {
            let mut value = json!({"seq":"a1","action":{"kind":"permissions"}});
            value[field] = json!("administrator");
            assert!(parse(42, &message(value)).is_err());
        }
    }
    #[test]
    fn only_bounded_supported_operations_cross_the_bridge() {
        for kind in ["eval", "execute", "kick", "maintenance", "profile_export"] {
            assert!(parse(1, &message(json!({"seq":"1","action":{"kind":kind}}))).is_err());
        }
        assert!(
            parse(
                1,
                &message(json!({"seq":"x".repeat(33),"action":{"kind":"permissions"}}))
            )
            .is_err()
        );
        assert!(parse(1,&message(json!({"seq":"1","action":{"kind":"settings_read","resource":"x","command":"whoami"}}))).is_err());
    }
    #[test]
    fn normal_log_and_profile_history_do_not_break_interactive_diagnostics() {
        let resources = json!([{"id":"test","running":true,"generation":"1"}]);
        let status = bounded_view(
            &Action::Status,
            json!({"resources":resources,"logs":vec!["x".repeat(2048);16]}),
        );
        assert_eq!(status["resources"], resources);
        assert!(status["logs_omitted"].as_u64().unwrap() > 0);
        assert!(serde_json::to_vec(&status).unwrap().len() <= 14 * 1024);
        let profile = bounded_view(
            &Action::ProfileRead {
                resource: Some("test".into()),
            },
            json!({"summaries":[{"resource":"test","count":128}],"spans":(0..128).map(|i|json!({"id":i,"source":"x".repeat(512)})).collect::<Vec<_>>(),"history_omitted":0}),
        );
        assert_eq!(profile["summaries"][0]["count"], 128);
        assert!(profile["history_omitted"].as_u64().unwrap() > 0);
        assert_eq!(
            profile["spans"].as_array().unwrap().last().unwrap()["id"],
            127
        );
    }
    #[test]
    fn permission_checks_are_bounded_and_cannot_request_credential_material() {
        assert!(
            parse(
                1,
                &message(
                    json!({"seq":"1","action":{"kind":"permissions","check":["custom.manage"]}})
                )
            )
            .is_ok()
        );
        for check in [
            json!(vec!["custom.manage"; 129]),
            json!(["x".repeat(65)]),
            json!(["bad permission"]),
        ] {
            assert!(
                parse(
                    1,
                    &message(json!({"seq":"1","action":{"kind":"permissions","check":check}}))
                )
                .is_err()
            );
        }
    }
    #[test]
    fn oversized_private_data_is_replaced_without_leaking_a_prefix() {
        let value = response("1", Ok(json!({"private":"secret".repeat(6000)})));
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "too_large");
        assert!(!value.to_string().contains("secret"));
        assert!(value.to_string().len() < 1024);
    }
}
