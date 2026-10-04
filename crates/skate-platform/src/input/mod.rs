//! Raw platform device transport. Packets retain the XInput ranges consumed by
//! the TU3 conversion in `skate-core`.
use skate_core::input::xbox::XboxState;
use std::time::{Duration, Instant};

pub struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    UnsupportedPlatform,
}

#[derive(Default)]
pub struct CapabilityCache {
    value: Option<(u8, Instant)>,
}

impl CapabilityCache {
    pub fn invalidate(&mut self) {
        self.value = None;
    }

    pub(crate) fn get_at(
        &mut self,
        now: Instant,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        if let Some((subtype, expires)) = self.value {
            if now < expires {
                return Ok(subtype);
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype, now + Duration::from_secs(1)));
        Ok(subtype)
    }

    pub(crate) fn get(
        &mut self,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        self.get_at(Instant::now(), read)
    }
}

fn disconnected(cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
    cache.invalidate();
    Err(DeviceError::Disconnected)
}

#[cfg(target_os = "linux")]
mod gilrs_raw;
#[cfg(windows)]
mod xinput;

pub fn poll_cached(index: usize, cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    return xinput::poll(index as u32, cache);
    #[cfg(target_os = "linux")]
    return gilrs_raw::poll(index, cache);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(DeviceError::UnsupportedPlatform)
    }
}

pub fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}

#[cfg(test)]
mod tests {
    use super::{CapabilityCache, DeviceError, disconnected};
    use std::time::{Duration, Instant};

    #[test]
    fn capability_cache_reuses_until_expiry_then_refreshes() {
        let start = Instant::now();
        let mut cache = CapabilityCache::default();

        assert_eq!(cache.get_at(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get_at(start + Duration::from_millis(999), || panic!(
                "cache refreshed early"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get_at(start + Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
    }

    #[test]
    fn capability_cache_does_not_retain_errors_or_invalidated_values() {
        let start = Instant::now();
        let mut cache = CapabilityCache::default();

        assert_eq!(
            cache.get_at(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get_at(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get_at(start, || Ok(4)), Ok(4));
    }

    #[test]
    fn disconnected_result_invalidates_cached_capabilities() {
        let start = Instant::now();
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.get_at(start, || Ok(7)), Ok(7));

        assert!(matches!(
            disconnected(&mut cache),
            Err(DeviceError::Disconnected)
        ));
        assert_eq!(cache.get_at(start, || Ok(1)), Ok(1));
    }
}
