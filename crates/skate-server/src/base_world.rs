//! Read only trusted local spawn metadata; dedicated hosting never needs to
//! decode the base map's render assets, textures or collision to restore it.
use crate::Map;
use skate_net::dedicated::TeleportDestination;
use std::io::Read;

pub(crate) fn spawn(map: &Map) -> Result<TeleportDestination, String> {
    let Map::File(path) = map else {
        return Ok(TeleportDestination {
            position: skate_data::resource_world::TEST_WORLD_SPAWN,
            heading: 0., velocity: [0.; 3], instance: 0,
        });
    };
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Base-world spawn {}: {e}", path.display()))?;
    read_spawn(&mut file).map_err(|e| format!("Base-world spawn {}: {e}", path.display()))
}

fn read_spawn(reader: &mut impl Read) -> Result<TeleportDestination, String> {
    let mut header = [0; 16];
    reader.read_exact(&mut header).map_err(|_| "Truncated SKATE map header")?;
    if &header[..5] != b"SKATE" || header[7] != 0
        || !header[5].is_ascii_digit() || !header[6].is_ascii_digit()
        || !(1..=15).contains(&((header[5] - b'0') * 10 + header[6] - b'0'))
        || u32::from_le_bytes(header[8..12].try_into().unwrap()) != 0x12345678
    { return Err("Unsupported SKATE magic, version or endian marker".into()); }
    let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if length == 0 || length > 64 * 1024 { return Err("SKATE map name exceeds 1..65536 bytes".into()); }
    let mut name = vec![0; length];
    reader.read_exact(&mut name).map_err(|_| "Truncated SKATE map name")?;
    std::str::from_utf8(&name).map_err(|_| "Invalid SKATE map name UTF-8")?;
    let mut bytes = [0; 16];
    reader.read_exact(&mut bytes).map_err(|_| "Truncated SKATE spawn metadata")?;
    let values: [f32; 4] = std::array::from_fn(|i| f32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap()));
    let spawn = TeleportDestination {
        position: [values[0], values[1], values[2]], heading: values[3], velocity: [0.; 3], instance: 0,
    };
    if !spawn.valid() { return Err("SKATE base spawn is not finite or exceeds movement bounds".into()); }
    Ok(spawn)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(version: u8) -> Vec<u8> {
        let mut bytes = format!("SKATE{version:02}\0").into_bytes();
        bytes.extend(0x12345678u32.to_le_bytes());
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(b"Base");
        for value in [12f32, 3., -4., 0.5] { bytes.extend(value.to_le_bytes()); }
        bytes
    }
    #[test]
    fn bounded_spawn_header_matches_supported_versions_and_shared_test_world() {
        for version in 1..=15 {
            // No geometry follows. A metadata-only reader must never decode it.
            let mut reader = std::io::Cursor::new(header(version));
            let spawn = read_spawn(&mut reader).unwrap();
            assert_eq!(spawn.position, [12., 3., -4.]);
            assert_eq!(spawn.heading, 0.5);
            assert_eq!(reader.position(), 36);
        }
        assert_eq!(spawn(&Map::TestWorld).unwrap().position, skate_data::resource_world::TEST_WORLD_SPAWN);
        assert_eq!(spawn(&Map::TestWorld).unwrap().velocity, [0.; 3]);
    }
    #[test]
    fn malformed_or_unbounded_spawn_headers_fail_closed() {
        let valid = header(15);
        for length in 0..valid.len() { assert!(read_spawn(&mut &valid[..length]).is_err()); }
        for length in [0u32, 65_537, u32::MAX] {
            let mut bytes = valid.clone(); bytes[12..16].copy_from_slice(&length.to_le_bytes());
            assert!(read_spawn(&mut bytes.as_slice()).is_err());
        }
        for bad in [f32::NAN, f32::INFINITY, 100_000.] {
            let mut bytes = valid.clone(); bytes[20..24].copy_from_slice(&bad.to_le_bytes());
            assert!(read_spawn(&mut bytes.as_slice()).is_err());
        }
        for version in [0, 16, 99] { assert!(read_spawn(&mut header(version).as_slice()).is_err()); }
        let mut bad_name = valid.clone(); bad_name[16] = 255;
        assert!(read_spawn(&mut bad_name.as_slice()).is_err());
        let mut bad_endian = valid; bad_endian[8] ^= 1;
        assert!(read_spawn(&mut bad_endian.as_slice()).is_err());
    }
    #[test]
    fn ordinary_host_keeps_hash_only_base_map_behavior() {
        let path = std::env::temp_dir().join(format!("skate-base-header-{}.skate", std::process::id()));
        std::fs::write(&path, b"hash-only fixture, deliberately not a SKATE header").unwrap();
        let map = Map::File(path.clone());
        assert!(spawn(&map).is_err());
        let host = crate::Host::bind(crate::Options { locations:None,
            bind: "127.0.0.1:0".parse().unwrap(), session: 91, max_players: 2,
            map, resources: None, accounts: None,
        operations: None,
        });
        std::fs::remove_file(path).unwrap();
        assert!(host.is_ok());
    }
}
