use crate::*;
use skate_net::packed;

#[derive(Debug, Clone)]
pub struct Frame {
    pub session: u64,
    pub actor: u64,
    pub epoch: u64,
    pub revision: u64,
    pub sequence: u64,
    pub channel: String,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct Playback {
    pub session: u64,
    pub actor: u64,
    pub recipient_epoch: u64,
    pub sender_epoch: u64,
    pub revision: u64,
    pub sequence: u64,
    pub gain: f32,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub session: u64,
    pub recipient_epoch: u64,
    pub instance: u64,
    pub revision: u64,
}
pub fn is_voice(bytes: &[u8]) -> bool {
    packed::envelope(bytes).is_some_and(|(_, _, kind, _)| kind == VOICE || kind == CONTROL)
}
fn u64_field(bytes: &mut &[u8]) -> Option<u64> {
    let (value, rest) = bytes.split_at_checked(8)?;
    *bytes = rest;
    Some(u64::from_le_bytes(value.try_into().ok()?))
}
fn byte(bytes: &mut &[u8]) -> Option<u8> {
    let (&v, rest) = bytes.split_first()?;
    *bytes = rest;
    Some(v)
}
fn payload(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.len() <= MAX_ENCODED
}
pub(crate) fn valid_channel(channel: &str) -> bool {
    channel
        .split_once('/')
        .is_some_and(|(owner, name)| crate::valid_name(owner) && crate::valid_name(name))
}

impl Frame {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.actor == 0
            || self.session == 0
            || self.epoch == 0
            || self.revision == 0
            || self.sequence == 0
            || (!self.channel.is_empty() && !valid_channel(&self.channel))
            || !payload(&self.data)
        {
            return Err("invalid voice frame".into());
        }
        let mut out = packed::header(self.session, self.actor, VOICE, self.sequence as u32);
        out.push(0);
        out.extend(self.epoch.to_le_bytes());
        out.extend(self.revision.to_le_bytes());
        out.extend(self.sequence.to_le_bytes());
        out.push(self.channel.len() as u8);
        out.extend(self.channel.as_bytes());
        out.extend(&self.data);
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (session, actor, kind, _) = packed::envelope(bytes)?;
        if kind != VOICE || session == 0 || actor == 0 {
            return None;
        }
        let mut tail = &bytes[packed::HEADER..];
        if byte(&mut tail)? != 0 {
            return None;
        }
        let epoch = u64_field(&mut tail)?;
        let revision = u64_field(&mut tail)?;
        let sequence = u64_field(&mut tail)?;
        let n = byte(&mut tail)? as usize;
        let (raw, rest) = tail.split_at_checked(n)?;
        let channel = std::str::from_utf8(raw).ok()?.to_string();
        if epoch == 0
            || revision == 0
            || sequence == 0
            || (!channel.is_empty() && !valid_channel(&channel))
            || !payload(rest)
        {
            return None;
        }
        Some(Self {
            session,
            actor,
            epoch,
            revision,
            sequence,
            channel,
            data: rest.to_vec(),
        })
    }
}
impl Playback {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = packed::header(self.session, self.actor, VOICE, self.sequence as u32);
        out.push(1);
        out.extend(self.recipient_epoch.to_le_bytes());
        out.extend(self.sender_epoch.to_le_bytes());
        out.extend(self.revision.to_le_bytes());
        out.extend(self.sequence.to_le_bytes());
        out.extend(((self.gain.clamp(0., 1.) * 65535.) as u16).to_le_bytes());
        out.extend(&self.data);
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (session, actor, kind, _) = packed::envelope(bytes)?;
        if kind != VOICE || actor == 0 {
            return None;
        }
        let mut tail = &bytes[packed::HEADER..];
        if byte(&mut tail)? != 1 {
            return None;
        }
        let recipient_epoch = u64_field(&mut tail)?;
        let sender_epoch = u64_field(&mut tail)?;
        let revision = u64_field(&mut tail)?;
        let sequence = u64_field(&mut tail)?;
        let (gain, rest) = tail.split_at_checked(2)?;
        let gain = u16::from_le_bytes(gain.try_into().ok()?) as f32 / 65535.;
        if !payload(rest)
            || recipient_epoch == 0
            || sender_epoch == 0
            || revision == 0
            || sequence == 0
        {
            return None;
        }
        Some(Self {
            session,
            actor,
            recipient_epoch,
            sender_epoch,
            revision,
            sequence,
            gain,
            data: rest.to_vec(),
        })
    }
}
impl Policy {
    pub(crate) fn encode(&self, server: u64) -> Vec<u8> {
        let mut out = packed::header(self.session, server, CONTROL, self.revision as u32);
        out.extend(self.recipient_epoch.to_le_bytes());
        out.extend(self.instance.to_le_bytes());
        out.extend(self.revision.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (session, _, kind, _) = packed::envelope(bytes)?;
        if kind != CONTROL {
            return None;
        }
        let mut tail = &bytes[packed::HEADER..];
        let recipient_epoch = u64_field(&mut tail)?;
        let instance = u64_field(&mut tail)?;
        let revision = u64_field(&mut tail)?;
        if !tail.is_empty() || recipient_epoch == 0 || revision == 0 {
            return None;
        }
        Some(Self {
            session,
            recipient_epoch,
            instance,
            revision,
        })
    }
}
