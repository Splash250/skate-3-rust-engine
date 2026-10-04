//! Voice uses the same admitted actor, UDP endpoint and movement epoch as gameplay.
use skate_net::dedicated::Server;

pub(crate) fn egress_budget() -> Result<usize, String> {
    match std::env::var("SKATE_VOICE_EGRESS_PACKETS") {
        Ok(value) => parse_egress(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_egress(None),
        Err(_) => Err("SKATE_VOICE_EGRESS_PACKETS must be an integer16..512".into()),
    }
}
fn parse_egress(value: Option<&str>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(512);
    };
    value
        .parse::<usize>()
        .ok()
        .filter(|n| (16..=512).contains(n))
        .ok_or_else(|| "SKATE_VOICE_EGRESS_PACKETS must be an integer16..512".into())
}

pub(crate) fn sync(router: &mut skate_voice::Router, server: &Server, now: u64) {
    let players = server
        .entity_players()
        .into_iter()
        .filter_map(|player| {
            Some(skate_voice::Player {
                actor: player.actor,
                peer: server.peer_for_actor(player.actor)?,
                instance: player.instance,
                epoch: player.epoch,
                position: player.position,
            })
        })
        .collect::<Vec<_>>();
    // The authority supplies at most64 validated finite observations. Failing
    // closed also retires buffered speech if this invariant ever changes.
    if router.sync(&players, now).is_err() {
        let _ = router.sync(&[], now);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn voice_egress_environment_is_bounded_without_mutating_process_environment() {
        assert_eq!(super::parse_egress(None).unwrap(), 512);
        for value in ["0", "15", "513", "99999999999", "-1", "", "NaN"] {
            assert!(super::parse_egress(Some(value)).is_err());
        }
        for value in [16, 256, 512] {
            assert_eq!(super::parse_egress(Some(&value.to_string())).unwrap(), value);
        }
    }
}
