//! Local, bounded dedicated endpoint book and asynchronous UDP discovery.
use super::{Multiplayer, transport};
use serde::{Deserialize, Serialize};
use skate_net::{
    discovery::{self, ServerInfo},
    lobby::Session,
};
use std::{
    collections::BTreeMap,
    io::Read,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const MAX_SERVERS: usize = 64;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub endpoint: SocketAddr,
    pub favorite: bool,
    pub last_joined: u64,
    /// A local credential profile pathname, never passwords or transport tokens.
    pub account_profile: Option<PathBuf>,
}
pub(crate) struct Browser {
    path: PathBuf,
    pub entries: Vec<Saved>,
    pub selected: usize,
    pub endpoint: String,
    pub account_profile: String,
    pub status: String,
    pub info: BTreeMap<SocketAddr, (ServerInfo, Instant)>,
    probe: Option<JoinHandle<Result<Vec<(SocketAddr, ServerInfo)>, String>>>,
    joining: Option<(
        SocketAddr,
        Option<PathBuf>,
        u64,
        JoinHandle<Result<Box<dyn transport::Transport>, String>>,
    )>,
    cancelled: bool,
}
fn endpoint(text: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = text
        .trim()
        .parse()
        .map_err(|_| "Enter a numeric IPv4 address and port, e.g. 127.0.0.1:31030")?;
    match address {
        SocketAddr::V4(v)
            if !v.ip().is_unspecified()
                && !v.ip().is_multicast()
                && !v.ip().is_broadcast()
                && v.port() != 0 =>
        {
            Ok(address)
        }
        _ => Err("A unicast IPv4 endpoint with a nonzero port is required".into()),
    }
}
impl Browser {
    pub fn new(path: PathBuf) -> Self {
        let loaded = (|| -> Result<Vec<Saved>, String> {
            let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > 65536 {
                return Err("Endpoint book must be a regular file under 64 KiB".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .map_err(|e| e.to_string())?
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let entries: Vec<Saved> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if entries.len() > MAX_SERVERS {
                return Err("Endpoint book exceeds 64 servers".into());
            }
            let mut seen = std::collections::BTreeSet::new();
            for item in &entries {
                endpoint(&item.endpoint.to_string())?;
                if !seen.insert(item.endpoint)
                    || item
                        .account_profile
                        .as_ref()
                        .is_some_and(|p| p.as_os_str().len() > 1024)
                {
                    return Err("Invalid or duplicate saved endpoint".into());
                }
            }
            Ok(entries)
        })();
        let status = if path.exists() {
            loaded.as_ref().err().cloned().unwrap_or_default()
        } else {
            String::new()
        };
        let mut browser = Self {
            path,
            entries: loaded.unwrap_or_default(),
            selected: 0,
            endpoint: "127.0.0.1:31030".into(),
            account_profile: String::new(),
            status,
            info: BTreeMap::new(),
            probe: None,
            joining: None,
            cancelled: false,
        };
        browser.select(0);
        browser
    }
    fn persist(&mut self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if self.path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
            return Err("Endpoint book is not a regular file".into());
        }
        let temporary = self.path.with_extension(format!("{}.tmp", super::unique()));
        let bytes = serde_json::to_vec_pretty(&self.entries).map_err(|e| e.to_string())?;
        let result = (|| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, &self.path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    pub(super) fn cancel_pending(&mut self) {
        self.cancelled = true;
    }
    fn take_login(
        &mut self,
    ) -> Option<(
        SocketAddr,
        u64,
        Result<Box<dyn transport::Transport>, String>,
    )> {
        if !self
            .joining
            .as_ref()
            .is_some_and(|(_, _, _, worker)| worker.is_finished())
        {
            return None;
        }
        let (address, _, session, worker) = self.joining.take().unwrap();
        let result = worker
            .join()
            .unwrap_or_else(|_| Err("Login worker failed".into()));
        if self.cancelled {
            None
        } else {
            Some((address, session, result))
        }
    }
    pub fn save(&mut self) {
        let result = (|| {
            let address = endpoint(&self.endpoint)?;
            if self.account_profile.len() > 1024 {
                return Err("Account profile path exceeds 1024 bytes".into());
            }
            let profile = (!self.account_profile.trim().is_empty())
                .then(|| PathBuf::from(self.account_profile.trim()));
            if let Some(i) = self.entries.iter().position(|e| e.endpoint == address) {
                self.entries[i].account_profile = profile;
                self.selected = i;
            } else {
                if self.entries.len() >= MAX_SERVERS {
                    return Err("64 saved servers: remove an entry first".into());
                }
                self.entries.push(Saved {
                    endpoint: address,
                    favorite: false,
                    last_joined: 0,
                    account_profile: profile,
                });
                self.selected = self.entries.len() - 1;
            }
            self.persist()
        })();
        self.status = result
            .err()
            .unwrap_or_else(|| "Endpoint saved. Refresh to inspect required content.".into());
    }
    pub fn select(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index) {
            self.selected = index;
            self.endpoint = entry.endpoint.to_string();
            self.account_profile = entry
                .account_profile
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
    }
    pub fn favorite(&mut self) {
        if let Some(e) = self.entries.get_mut(self.selected) {
            e.favorite = !e.favorite;
            if let Err(e) = self.persist() {
                self.status = e;
            }
        }
    }
    pub fn remove(&mut self) {
        if self.selected < self.entries.len() {
            let old = self.entries.remove(self.selected);
            self.info.remove(&old.endpoint);
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            if let Err(e) = self.persist() {
                self.status = e;
            }
        }
    }
    pub fn refresh(&mut self) {
        if self.probe.is_some() {
            return;
        }
        let endpoints: Vec<_> = self.entries.iter().map(|e| e.endpoint).collect();
        self.status = "Querying saved endpoints…".into();
        self.probe = Some(std::thread::spawn(move || {
            let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            socket
                .set_read_timeout(Some(Duration::from_millis(25)))
                .map_err(|e| e.to_string())?;
            let mut pending = BTreeMap::new();
            for address in endpoints {
                let nonce = super::unique();
                socket
                    .send_to(&discovery::query(nonce), address)
                    .map_err(|e| e.to_string())?;
                pending.insert(address, nonce);
            }
            let mut output = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut bytes = [0; 1201];
            while !pending.is_empty() && Instant::now() < deadline {
                match socket.recv_from(&mut bytes) {
                    Ok((n, from)) => {
                        if let Some(nonce) = pending.get(&from)
                            && let Some(info) = discovery::parse_response(&bytes[..n], *nonce)
                        {
                            output.push((from, info));
                            pending.remove(&from);
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionRefused
                        ) => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(output)
        }));
    }
    pub fn preview(&self) -> String {
        let Some(entry) = self.entries.get(self.selected) else {
            return "Save a direct endpoint to begin. No Steam or directory account is needed."
                .into();
        };
        let Some((info, at)) = self.info.get(&entry.endpoint) else {
            return "No current response. Refresh; the endpoint may be offline or discovery disabled.".into();
        };
        let settings = info
            .public_settings
            .iter()
            .flat_map(|(id, values)| {
                values
                    .iter()
                    .map(move |(key, value)| format!("{id}.{key}={value}"))
            })
            .take(4)
            .collect::<Vec<_>>()
            .join(" · ");
        format!(
            "{} · {} / {} · {}/{} players · {} queued{}\n{} | {} | {} resources, {:.1} MiB required\n{}{}\n{}\nMetadata is untrusted; downloaded bytes are verified before activation.",
            info.name,
            info.map,
            info.mode,
            info.players,
            info.capacity,
            info.queued,
            if info.maintenance {
                " · MAINTENANCE"
            } else {
                ""
            },
            if at.elapsed() > Duration::from_secs(30) {
                "STALE: refresh"
            } else {
                "Fresh response"
            },
            if info.compatible() {
                "Compatible protocol/build"
            } else {
                "INCOMPATIBLE: matching build required"
            },
            info.content_resources,
            info.content_bytes as f64 / 1048576.,
            info.resource_names.join(", "),
            if info.accounts_required {
                " · Account profile required"
            } else {
                " · Anonymous development server"
            },
            settings
        )
    }
}
impl Multiplayer {
    pub(crate) fn join_dedicated(&mut self) {
        if self.server_browser.joining.is_some() {
            self.server_browser.status = "Login is already in progress; cancel or wait.".into();
            return;
        }
        let Some(entry) = self
            .server_browser
            .entries
            .get(self.server_browser.selected)
            .cloned()
        else {
            return;
        };
        let Some((info, at)) = self.server_browser.info.get(&entry.endpoint) else {
            self.server_browser.status = "Refresh this server before joining.".into();
            return;
        };
        let error = if at.elapsed() > Duration::from_secs(30) {
            Some("Server details are stale; refresh before joining.")
        } else if !info.compatible() {
            Some("Incompatible server build/protocol.")
        } else if info.map_fingerprint != self.info.map {
            Some("Select the server's base map locally first (usually Test world).")
        } else if info.maintenance {
            Some("Server is in maintenance. Refresh after it reopens.")
        } else if info.accounts_required && entry.account_profile.is_none() {
            Some("Select an account profile file for this authenticated server.")
        } else {
            None
        };
        if let Some(error) = error {
            self.server_browser.status = error.into();
            return;
        }
        let session = info.session;
        let profile = entry.account_profile.clone();
        let address = entry.endpoint;
        self.leave();
        self.server_browser.cancelled = false;
        self.server_browser.status =
            "Opening connection and authenticating… Cancel remains available.".into();
        let worker = std::thread::spawn(move || {
            if let Some(path) = profile {
                transport::SecureDirect::login(&path, address)
                    .map(|t| Box::new(t) as Box<dyn transport::Transport>)
            } else {
                transport::Direct::new("0.0.0.0:0".parse().unwrap())
                    .map(|t| Box::new(t) as Box<dyn transport::Transport>)
                    .map_err(|e| e.to_string())
            }
        });
        self.server_browser.joining = Some((address, entry.account_profile, session, worker));
    }
    pub(crate) fn cancel_dedicated(&mut self) {
        self.server_browser.cancelled = true;
        self.leave();
        self.server_browser.status =
            "Join cancelled. A pending bounded login will be discarded.".into();
    }
    pub(super) fn poll_browser(&mut self) {
        if self
            .server_browser
            .probe
            .as_ref()
            .is_some_and(|w| w.is_finished())
        {
            let result = self
                .server_browser
                .probe
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err("Discovery worker failed".into()));
            match result {
                Ok(rows) => {
                    let count = rows.len();
                    self.server_browser.info.clear();
                    for (address, info) in rows {
                        self.server_browser
                            .info
                            .insert(address, (info, Instant::now()));
                    }
                    self.server_browser.status = format!("{count} saved servers responded.");
                }
                Err(e) => self.server_browser.status = e,
            }
        }
        if let Some((address, session, result)) = self.server_browser.take_login() {
            match result {
                Ok(t) => {
                    self.start(t, session, Some(transport::endpoint(address).unwrap()));
                    self.lobby = Some(Session::dedicated_client(
                        session,
                        self.info,
                        transport::endpoint(address).unwrap(),
                    ));
                    self.loopback = address.ip().is_loopback();
                    self.lobby.as_mut().unwrap().set_loopback(self.loopback);
                    self.dedicated_endpoint = Some(address);
                    self.dedicated_launch = true;
                    self.status = format!("Connecting to {address}…");
                    self.server_browser.status = self.status.clone();
                }
                Err(e) => {
                    self.server_browser.status = format!("Connection failed: {e}");
                    self.status = self.server_browser.status.clone();
                }
            }
        }
        if self.connected()
            && let Some(address) = self.dedicated_endpoint
        {
            // Update recency once per admitted session, not on attempted joins.
            if !self.browser_recorded {
                self.browser_recorded = true;
                if let Some(e) = self
                    .server_browser
                    .entries
                    .iter_mut()
                    .find(|e| e.endpoint == address)
                {
                    e.last_joined = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if let Err(e) = self.server_browser.persist() {
                        self.server_browser.status = e;
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_login_is_discarded_after_leaving_or_switching_transport() {
        let mut browser = Browser::new(std::env::temp_dir().join(format!(
            "skate-browser-cancel-{}.json",
            super::super::unique()
        )));
        let (send, recv) = std::sync::mpsc::channel();
        browser.joining = Some((
            "127.0.0.1:31030".parse().unwrap(),
            None,
            42,
            std::thread::spawn(move || {
                recv.recv().unwrap();
                transport::Direct::new("127.0.0.1:0".parse().unwrap())
                    .map(|t| Box::new(t) as Box<dyn transport::Transport>)
                    .map_err(|e| e.to_string())
            }),
        ));
        assert!(browser.take_login().is_none());
        browser.cancel_pending();
        send.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while !browser.joining.as_ref().unwrap().3.is_finished() {
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(browser.take_login().is_none());
        assert!(browser.joining.is_none());
    }
    #[test]
    fn rejects_non_unicast_endpoints() {
        for bad in [
            "evil.example:31030",
            "0.0.0.0:1",
            "255.255.255.255:2",
            "224.1.2.3:9",
            "127.0.0.1:0",
            "[::1]:31030",
        ] {
            assert!(endpoint(bad).is_err(), "{bad}");
        }
        assert!(endpoint("127.0.0.1:31030").is_ok());
    }
    #[test]
    fn favorites_profile_and_recency_round_trip_with_bounds() {
        let path =
            std::env::temp_dir().join(format!("skate-endpoints-{}.json", super::super::unique()));
        let mut b = Browser::new(path.clone());
        b.account_profile = "/private/account.json".into();
        b.save();
        b.favorite();
        b.entries[0].last_joined = 123;
        b.persist().unwrap();
        let loaded = Browser::new(path.clone());
        assert!(loaded.entries[0].favorite);
        assert_eq!(loaded.entries[0].last_joined, 123);
        assert_eq!(
            loaded.entries[0].account_profile.as_deref(),
            Some(std::path::Path::new("/private/account.json"))
        );
        std::fs::write(&path, b"[] trailing junk").unwrap();
        let invalid = Browser::new(path.clone());
        assert!(invalid.entries.is_empty());
        assert!(!invalid.status.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
