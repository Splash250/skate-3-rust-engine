use crate::*;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"SK8AUTH1";
const HEADER: usize = 32;
pub const MAX_DATAGRAM: usize = 1200 + HEADER + 16;
struct Codec {
    key: LessSafeKey,
    id: [u8; 16],
    sending: u64,
    received: u64,
    window: u128,
    client: bool,
}
impl Codec {
    fn new(id: [u8; 16], key: &[u8; 32], client: bool) -> Result<Self> {
        Ok(Self {
            key: LessSafeKey::new(
                UnboundKey::new(&CHACHA20_POLY1305, key)
                    .map_err(|_| error("crypto", "invalid session key"))?,
            ),
            id,
            sending: 0,
            received: 0,
            window: 0,
            client,
        })
    }
    fn encode(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        if payload.len() > 1200 {
            return Err(error("limit", "gameplay datagram exceeds 1200 bytes"));
        }
        self.sending = self
            .sending
            .checked_add(1)
            .ok_or_else(|| error("expired", "session packet counter exhausted; reconnect"))?;
        let mut packet = Vec::with_capacity(HEADER + payload.len() + 16);
        packet.extend(MAGIC);
        packet.extend(self.id);
        packet.extend(self.sending.to_be_bytes());
        let aad = packet.clone();
        packet.extend(payload);
        let tag = self
            .key
            .seal_in_place_separate_tag(
                nonce(self.client, self.sending),
                Aad::from(aad.as_slice()),
                &mut packet[HEADER..],
            )
            .map_err(|_| error("crypto", "packet encryption failed"))?;
        packet.extend(tag.as_ref());
        Ok(packet)
    }
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<u8>> {
        if packet.len() < HEADER + 16
            || packet.len() > MAX_DATAGRAM
            || &packet[..8] != MAGIC
            || packet[8..24] != self.id
        {
            return Err(error("invalid", "invalid authenticated datagram"));
        }
        let sequence = u64::from_be_bytes(packet[24..32].try_into().unwrap());
        if sequence == 0
            || (sequence <= self.received
                && (self.received - sequence >= 128
                    || self.window & (1u128 << (self.received - sequence)) != 0))
        {
            return Err(error("replay", "duplicate or stale authenticated packet"));
        }
        let mut body = packet[HEADER..].to_vec();
        let decoded = self
            .key
            .open_in_place(
                nonce(!self.client, sequence),
                Aad::from(&packet[..HEADER]),
                &mut body,
            )
            .map_err(|_| error("unauthenticated", "packet authentication failed"))?
            .to_vec();
        // Commit the anti-replay window only after tag verification.
        if sequence > self.received {
            let shift = sequence - self.received;
            self.window = if shift >= 128 {
                1
            } else {
                (self.window << shift) | 1
            };
            self.received = sequence;
        } else {
            self.window |= 1u128 << (self.received - sequence);
        }
        Ok(decoded)
    }
}
fn nonce(client_to_server: bool, sequence: u64) -> Nonce {
    let mut bytes = [0; 12];
    bytes[3] = if client_to_server { 1 } else { 2 };
    bytes[4..].copy_from_slice(&sequence.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}
pub struct ClientSession {
    codec: Codec,
    pub account_id: String,
    pub actor: u64,
    expires: u64,
}
impl ClientSession {
    /// Construct once per login. Reconstructing a codec with the same key would
    /// reuse nonces; credentials are single connection state, not a reconnect file.
    pub fn new(credentials: SessionCredentials) -> Result<Self> {
        let id = unhex::<16>(&credentials.session_id)?;
        let key = unhex::<32>(&credentials.key)?;
        Ok(Self {
            codec: Codec::new(id, &key, true)?,
            account_id: credentials.account_id,
            actor: credentials.actor,
            expires: credentials.expires,
        })
    }
    pub fn encode(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        if now() >= self.expires {
            return Err(error("expired", "session expired; log in again"));
        }
        self.codec.encode(payload)
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<u8>> {
        if now() >= self.expires {
            return Err(error("expired", "session expired; log in again"));
        }
        self.codec.decode(packet)
    }
}
pub struct ServerTransport {
    store: AccountStore,
    sessions: BTreeMap<[u8; 16], (VerifiedSession, Codec)>,
}
impl ServerTransport {
    pub fn new(store: AccountStore) -> Result<Self> {
        store.claim_transport()?;
        Ok(Self {
            store,
            sessions: BTreeMap::new(),
        })
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<(VerifiedSession, Vec<u8>)> {
        if packet.len() < HEADER + 16 || packet.len() > MAX_DATAGRAM || &packet[..8] != MAGIC {
            return Err(error("invalid", "authenticated datagram required"));
        }
        let id: [u8; 16] = packet[8..24].try_into().unwrap();
        if !self.sessions.contains_key(&id) {
            let session = self.store.by_id(&id)?;
            self.sessions.retain(|_, (session, _)| session.is_active());
            if self.sessions.len() >= 256 {
                return Err(error("busy", "authenticated transport capacity reached"));
            }
            let codec = Codec::new(id, &session.0.key, false)?;
            self.sessions.insert(id, (session, codec));
        }
        let (session, codec) = self.sessions.get_mut(&id).unwrap();
        if !session.is_active() {
            return Err(error("unauthenticated", "session expired or revoked"));
        }
        Ok((session.clone(), codec.decode(packet)?))
    }
    pub fn encode(&mut self, session: &VerifiedSession, payload: &[u8]) -> Result<Vec<u8>> {
        if !session.is_active() {
            return Err(error("unauthenticated", "session expired or revoked"));
        }
        let (_, codec) = self
            .sessions
            .get_mut(&session.0.id)
            .ok_or_else(|| error("missing", "no authenticated incoming transport"))?;
        codec.encode(payload)
    }
}

impl Drop for ServerTransport {
    fn drop(&mut self) {
        self.store.release_transport();
    }
}
