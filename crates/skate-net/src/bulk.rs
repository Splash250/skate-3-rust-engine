//! Bounded, acknowledged large-message lane. One chunk is in flight at a time;
//! callers schedule this independently of movement and small control records.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
const CHUNK: usize = 192;
pub const HARD_MESSAGE_BYTES: usize = 256 * 1024 + 1024;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_message: usize,
    pub max_queued_bytes: usize,
    pub max_pending: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_message: 16 * 1024 + 1024,
            max_queued_bytes: 1024 * 1024,
            max_pending: 64,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub id: u64,
    pub total: usize,
    pub offset: usize,
    /// Hex keeps worst-case JSON expansion predictable, including binary input.
    pub data: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel: bool,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ack {
    pub id: u64,
    pub through: usize,
    pub cancelled: bool,
}
/// Progress measures acknowledged encoded message bytes. Delivery confirms the
/// remote transport accepted the message, not that its script callback succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Pending { acknowledged: usize, total: usize },
    Delivered { total: usize },
    Cancelled,
}
struct Transfer {
    id: u64,
    bytes: Vec<u8>,
    offset: usize,
    cancel: bool,
}
struct Partial {
    id: u64,
    total: usize,
    bytes: Vec<u8>,
}
pub struct Channel {
    limits: Limits,
    next: u64,
    queued_bytes: usize,
    pending: VecDeque<Transfer>,
    completed: VecDeque<(u64, Progress)>,
    partial: Option<Partial>,
    ack: Ack,
}
impl Default for Channel {
    fn default() -> Self {
        Self::new(Limits::default()).unwrap()
    }
}
impl Channel {
    pub fn new(limits: Limits) -> Result<Self, String> {
        if limits.max_message == 0
            || limits.max_message > HARD_MESSAGE_BYTES
            || limits.max_queued_bytes < limits.max_message
            || limits.max_queued_bytes > 16 * 1024 * 1024
            || !(1..=256).contains(&limits.max_pending)
        {
            return Err("Invalid bounded bulk-transfer limits".into());
        }
        Ok(Self {
            limits,
            next: 0,
            queued_bytes: 0,
            pending: VecDeque::new(),
            completed: VecDeque::new(),
            partial: None,
            ack: Ack::default(),
        })
    }
    pub fn send(&mut self, bytes: Vec<u8>) -> Result<u64, String> {
        if bytes.is_empty() || bytes.len() > self.limits.max_message {
            return Err("Large message exceeds negotiated byte limit".into());
        }
        if self.pending.len() >= self.limits.max_pending
            || self.queued_bytes.saturating_add(bytes.len()) > self.limits.max_queued_bytes
        {
            return Err("Large-message backpressure: queue byte/count budget exhausted".into());
        }
        let id = self.next.checked_add(1).ok_or("Bulk sequence exhausted")?;
        self.next = id;
        self.queued_bytes += bytes.len();
        self.pending.push_back(Transfer {
            id,
            bytes,
            offset: 0,
            cancel: false,
        });
        Ok(id)
    }
    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn progress(&self, id: u64) -> Result<Progress, String> {
        if let Some(p) = self.pending.iter().find(|p|p.id == id) {
            return Ok(if p.cancel {Progress::Cancelled} else {
                Progress::Pending {acknowledged:p.offset,total:p.bytes.len()}
            });
        }
        self.completed.iter().find(|(key,_)|*key == id).map(|(_,progress)|*progress)
            .ok_or_else(||"Unknown or expired large transfer".into())
    }
    fn complete(&mut self, progress: Progress) {
        let p = self.pending.pop_front().unwrap();
        if self.completed.len() >= 256 {self.completed.pop_front();}
        self.completed.push_back((p.id,progress));
    }
    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        let transfer = self
            .pending
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Unknown or completed transfer")?;
        transfer.cancel = true;
        // Retain a bounded tombstone until acknowledged so following IDs have no gap.
        self.queued_bytes -= transfer.bytes.len();
        transfer.bytes.clear();
        Ok(())
    }
    pub fn frame(&self) -> Option<Frame> {
        let p = self.pending.front()?;
        let bytes = if p.cancel {
            &[][..]
        } else {
            &p.bytes[p.offset..(p.offset + CHUNK).min(p.bytes.len())]
        };
        let mut data = String::with_capacity(bytes.len() * 2);
        const HEX: &[u8] = b"0123456789abcdef";
        for b in bytes {
            data.push(HEX[(b >> 4) as usize] as char);
            data.push(HEX[(b & 15) as usize] as char);
        }
        Some(Frame {
            id: p.id,
            total: p.bytes.len(),
            offset: p.offset,
            data,
            cancel: p.cancel,
        })
    }
    pub fn ack(&self) -> Ack {
        self.ack
    }
    pub fn acknowledge(&mut self, ack: Ack) -> Result<(), String> {
        let Some(p) = self.pending.front_mut() else {
            return if ack.id <= self.next {
                Ok(())
            } else {
                Err("Bulk ACK exceeds sent transfer".into())
            };
        };
        if ack.id < p.id {
            return Ok(());
        }
        if ack.id > p.id {
            return Err("Bulk ACK exceeds sent transfer".into());
        }
        if p.cancel {
            if ack.cancelled {
                self.complete(Progress::Cancelled);
            }
            return Ok(());
        }
        if ack.cancelled || ack.through > (p.offset + CHUNK).min(p.bytes.len()) {
            return Err("Bulk ACK exceeds transmitted bytes".into());
        }
        if ack.through <= p.offset {
            return Ok(());
        }
        // Partial chunk ACKs would splice a different stream; accept only exact boundaries.
        if ack.through != (p.offset + CHUNK).min(p.bytes.len()) {
            return Err("Bulk ACK is not a chunk boundary".into());
        }
        p.offset = ack.through;
        if p.offset == p.bytes.len() {
            let total = p.bytes.len();
            self.queued_bytes -= total;
            self.complete(Progress::Delivered {total});
        }
        Ok(())
    }
    pub fn receive(&mut self, frame: &Frame) -> Result<Option<Vec<u8>>, String> {
        if frame.id == 0 || frame.data.len() > CHUNK * 2 {
            return Err("Invalid bulk chunk".into());
        }
        if frame.id < self.ack.id {
            return Ok(None);
        }
        if frame.id == self.ack.id && self.partial.is_none() {
            // A cancellation arriving after completion still needs acknowledgement.
            if frame.cancel {
                self.ack.cancelled = true;
            }
            return Ok(None);
        }
        if let Some(p) = &self.partial {
            if frame.id != p.id {
                return Err("Bulk transfer sequence gap".into());
            }
        } else if self.ack.id.checked_add(1) != Some(frame.id) {
            return Err("Bulk transfer sequence gap".into());
        }
        if frame.cancel {
            if !frame.data.is_empty() || frame.total != 0 {
                return Err("Invalid bulk cancellation".into());
            }
            self.partial = None;
            self.ack = Ack {
                id: frame.id,
                through: 0,
                cancelled: true,
            };
            return Ok(None);
        }
        if frame.total == 0
            || frame.total > self.limits.max_message
            || frame.data.is_empty()
            || frame.data.len() % 2 != 0
            || frame.offset >= frame.total
            || frame.offset % CHUNK != 0
            || frame
                .offset
                .checked_add(frame.data.len() / 2)
                .is_none_or(|end| end > frame.total)
        {
            return Err("Bulk chunk exceeds negotiated bounds".into());
        }
        let size = frame.data.len() / 2;
        if size != CHUNK.min(frame.total - frame.offset) {
            return Err("Truncated bulk chunk".into());
        }
        if self.partial.is_none() && frame.offset != 0 {
            return Err("Bulk transfer must begin at zero".into());
        }
        let decode = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let bytes = frame
            .data
            .as_bytes()
            .chunks_exact(2)
            .map(|p| Some(decode(p[0])? * 16 + decode(p[1])?))
            .collect::<Option<Vec<_>>>()
            .ok_or("Invalid bulk chunk encoding")?;
        let p = self.partial.get_or_insert_with(|| Partial {
            id: frame.id,
            total: frame.total,
            bytes: Vec::with_capacity(frame.total),
        });
        if p.total != frame.total {
            return Err("Bulk size changed mid-transfer".into());
        }
        if frame.offset < p.bytes.len() {
            if p.bytes.get(frame.offset..frame.offset + bytes.len()) != Some(&bytes[..]) {
                return Err("Conflicting duplicate bulk chunk".into());
            }
            return Ok(None);
        }
        if frame.offset != p.bytes.len() {
            return Err("Bulk chunk offset gap".into());
        }
        p.bytes.extend(bytes);
        self.ack = Ack {
            id: p.id,
            through: p.bytes.len(),
            cancelled: false,
        };
        if p.bytes.len() == p.total {
            return Ok(self.partial.take().map(|p| p.bytes));
        }
        Ok(None)
    }
}
