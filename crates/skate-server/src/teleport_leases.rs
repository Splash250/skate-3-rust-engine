//! Host-owned reversible instance travel. Unload callbacks cannot restore actors:
//! resource retirement intentionally discards their queued commands.
use skate_net::dedicated::{Server, TeleportDestination};
use std::collections::BTreeMap;

#[derive(Clone)]
struct Lease {
    resource: String,
    generation: u64,
    original: TeleportDestination,
    instance: u64,
    epoch: u64,
    return_deadline: Option<u64>,
}
#[derive(Default)]
pub(super) struct Leases(BTreeMap<u64, Lease>);
impl Leases {
    pub(super) fn world_changed(&mut self, server: &mut Server) {
        // Saved coordinates belong to the old world. Rejoining chooses the new
        // approved spawn and public instance; never strand private-instance actors.
        for (actor, lease) in &self.0 {
            if Self::current(server, *actor, lease) {
                eprintln!(
                    "Temporary travel {} generation {} ended for actor {}: required world changed; reconnect for approved spawn",
                    lease.resource, lease.generation, actor
                );
                server.kick(*actor, server.now_ms());
            }
        }
        self.0.clear();
    }
    fn current(server: &Server, actor: u64, lease: &Lease) -> bool {
        server.movement_epoch_of(actor) == Some(lease.epoch)
            && server.instance_of(actor) == Some(lease.instance)
    }
    pub(super) fn command(
        &mut self,
        server: &mut Server,
        resource: &str,
        generation: u64,
        actor: u64,
        destination: TeleportDestination,
        restore_on_stop: bool,
    ) -> Result<(), String> {
        self.0
            .retain(|actor, lease| Self::current(server, *actor, lease));
        let previous = self.0.get(&actor).cloned();
        if restore_on_stop
            && previous
                .as_ref()
                .is_some_and(|lease| lease.resource != resource || lease.generation != generation)
        {
            return Err("Player already has another resource's temporary instance lease".into());
        }
        let original = if restore_on_stop {
            if let Some(lease) = &previous {
                Some(lease.original)
            } else {
                if self.0.len() >= 64 {
                    return Err("Temporary instance lease capacity reached".into());
                }
                let player = server
                    .competition_players()
                    .into_iter()
                    .find(|player| player.actor == actor)
                    .ok_or("Temporary travel requires a current admitted movement observation")?;
                Some(TeleportDestination {
                    position: player.position,
                    heading: 0.,
                    velocity: [0.; 3],
                    instance: player.instance,
                })
            }
        } else {
            None
        };
        if original.is_some_and(|point| !point.valid()) {
            return Err(
                "Current position cannot be used as a bounded temporary return destination".into(),
            );
        }
        let epoch = server.teleport_now(actor, destination)?;
        if let Some(original) = original {
            self.0.insert(
                actor,
                Lease {
                    resource: resource.into(),
                    generation,
                    original,
                    instance: destination.instance,
                    epoch,
                    return_deadline: None,
                },
            );
        } else {
            // A normal successful teleport is a deliberate new authority decision.
            // It consumes the old lease, including a same-owner normal return.
            self.0.remove(&actor);
        }
        Ok(())
    }
    pub(super) fn request_return(
        &mut self,
        server: &Server,
        resource: &str,
        generation: u64,
        actor: u64,
    ) -> Result<(), String> {
        let Some(lease) = self.0.get_mut(&actor) else {
            return Ok(());
        };
        if lease.resource != resource || lease.generation != generation {
            return Err("Temporary travel return belongs to another resource generation".into());
        }
        if !Self::current(server, actor, lease) {
            self.0.remove(&actor);
            return Ok(());
        }
        lease
            .return_deadline
            .get_or_insert(server.now_ms().saturating_add(30_000));
        Ok(())
    }
    pub(super) fn sync(&mut self, server: &mut Server, owners: &BTreeMap<String, u64>) {
        for (actor, (previous, next)) in server.drain_resource_readmission_resets() {
            if let Some(lease) = self.0.get_mut(&actor) {
                if lease.epoch == previous
                    && server.movement_epoch_of(actor) == Some(next)
                    && server.instance_of(actor) == Some(lease.instance)
                {
                    lease.epoch = next;
                }
            }
        }
        self.0.retain(|actor, lease| {
            if !Self::current(server, *actor, lease) {
                return false;
            }
            if lease.return_deadline.is_none()
                && owners.get(&lease.resource) == Some(&lease.generation)
            {
                return true;
            }
            let deadline = *lease
                .return_deadline
                .get_or_insert(server.now_ms().saturating_add(30_000));
            // A content transition hides movement until readiness, but ownership
            // stays retired. Retry once admission is ready; never forge readiness.
            if !server.resource_ready(*actor) {
                if server.now_ms() >= deadline {
                    server.kick(*actor, server.now_ms());
                    return false;
                }
                return true;
            }
            if server.teleport_now(*actor, lease.original).is_ok() {
                return false;
            }
            if server.now_ms() >= deadline {
                server.kick(*actor, server.now_ms());
                return false;
            }
            true
        });
    }
    pub(super) fn verifier_snapshot(&self, server: &Server) -> BTreeMap<u64, u64> {
        self.0
            .iter()
            .filter(|(actor, lease)| Self::current(server, **actor, lease))
            .map(|(actor, lease)| (*actor, lease.epoch))
            .collect()
    }
    /// Only call around the trusted verifier itself. Baseline resets/corrections
    /// inside a competition retain the original return point; outside teleports
    /// and reconnects must never advance the stored lease epoch.
    pub(super) fn verifier_finished(&mut self, before: BTreeMap<u64, u64>, server: &Server) {
        self.advance_owned_epochs(before, server);
    }
    /// Resource content publication deliberately rotates each connection epoch.
    /// The host snapshots only still-current leases immediately around that
    /// operation, so a previous outside teleport cannot become eligible again.
    pub(super) fn readmission_finished(&mut self, before: BTreeMap<u64, u64>, server: &Server) {
        self.advance_owned_epochs(before, server);
    }
    fn advance_owned_epochs(&mut self, before: BTreeMap<u64, u64>, server: &Server) {
        for (actor, epoch) in before {
            if let Some(lease) = self.0.get_mut(&actor) {
                if lease.epoch == epoch && server.instance_of(actor) == Some(lease.instance) {
                    if let Some(next) = server.movement_epoch_of(actor) {
                        lease.epoch = next;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_net::{
        Body, Pose,
        dedicated::Config,
        lobby::{Info, Session},
        packed::{self, BodyState, Packed},
    };
    fn server() -> Server {
        let mut server = Server::new(Config {
            session: 7,
            server_id: 99,
            map: 1,
            max_players: 4,
        })
        .unwrap();
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 10,
                map: 1,
                rig: 2,
                physics: 3,
                appearance: 4,
            },
            99,
        );
        client.set_loopback(true);
        for now in (0..300).step_by(20) {
            if let Some(reset) = client.pending_movement_reset() {
                client.complete_movement_reset(reset.epoch);
            }
            let pose = Pose {
                p: [-8., 1., 0.],
                q: [0., 0., 0., 1.],
            };
            client.publish(
                packed::BODY,
                Packed::body(&BodyState {
                    root: pose,
                    enabled: (1 << 33) - 1,
                    bodies: vec![
                        Body {
                            pose,
                            velocity: [0.; 3],
                            angular: [0.; 3]
                        };
                        33
                    ],
                })
                .unwrap(),
                now,
            );
            for packet in client.service(now) {
                server.receive(11, &packet.data, now);
            }
            for packet in server.service(now) {
                client.receive(99, &packet.data, now);
            }
        }
        assert_eq!(server.competition_players().len(), 1);
        server
    }
    fn destination(instance: u64) -> TeleportDestination {
        TeleportDestination {
            position: [1., 1., 1.],
            heading: 0.,
            velocity: [0.; 3],
            instance,
        }
    }
    #[test]
    fn retiring_or_replacing_owner_restores_and_conflicting_lease_is_denied() {
        for owners in [BTreeMap::new(), BTreeMap::from([("tournament".into(), 2)])] {
            let mut server = server();
            let mut leases = Leases::default();
            leases
                .command(&mut server, "tournament", 1, 10, destination(1000), true)
                .unwrap();
            assert!(
                leases
                    .command(&mut server, "other", 1, 10, destination(2000), true)
                    .is_err()
            );
            assert_eq!(server.instance_of(10), Some(1000));
            leases.sync(&mut server, &owners);
            assert_eq!(server.instance_of(10), Some(0));
            assert!(leases.0.is_empty());
        }
    }
    #[test]
    fn explicit_return_requires_owner_and_deadline_never_bypasses_readiness() {
        let mut host = server();
        let mut leases = Leases::default();
        leases
            .command(&mut host, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        assert!(leases.request_return(&host, "other", 1, 10).is_err());
        leases.request_return(&host, "tournament", 1, 10).unwrap();
        leases.sync(&mut host, &BTreeMap::from([("tournament".into(), 1)]));
        assert_eq!(host.instance_of(10), Some(0));
        let mut host = server();
        leases
            .command(&mut host, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        let before = leases.verifier_snapshot(&host);
        host.configure_resources(
            "a".repeat(64),
            12345,
            BTreeMap::from([("tournament".into(), 1)]),
        )
        .unwrap();
        leases.readmission_finished(before, &host);
        leases.request_return(&host, "tournament", 1, 10).unwrap();
        leases.0.get_mut(&10).unwrap().return_deadline = Some(0);
        leases.sync(&mut host, &BTreeMap::from([("tournament".into(), 1)]));
        assert!(host.instance_of(10).is_none());
        assert!(leases.0.is_empty());
    }
    #[test]
    fn actual_course_start_retains_temporary_travel_return_point() {
        let mut server = server();
        let mut leases = Leases::default();
        leases
            .command(&mut server, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        let terrain = crate::world::Terrain {
            revision: "test".into(),
            spawn: [0., 1., 0.],
            heading: 0.,
            triangles: std::sync::Arc::new(vec![
                [[-30., 0., -20.], [30., 0., 20.], [30., 0., -20.]],
                [[-30., 0., -20.], [-30., 0., 20.], [30., 0., 20.]],
            ]),
        };
        let mut verifier = crate::competition::Competition::default();
        verifier.set_terrain(Some(&terrain)).unwrap();
        verifier.sync_resources(BTreeMap::from([("ranked".into(), 1)]));
        verifier.command("ranked",1,serde_json::json!({"kind":"define","course":{"name":"lease","instance":"1000","checkpoints":[{"position":[10,1,0],"radius":1,"points":10}]}}),&mut server).unwrap();
        let before = leases.verifier_snapshot(&server);
        verifier
            .command(
                "ranked",
                1,
                serde_json::json!({"kind":"start","name":"lease","player":"10"}),
                &mut server,
            )
            .unwrap();
        leases.verifier_finished(before, &server);
        leases.sync(&mut server, &BTreeMap::new());
        assert_eq!(server.instance_of(10), Some(0));
        assert_eq!(server.competition_players()[0].position, [-8., 1., 0.]);
    }
    #[test]
    fn disconnect_and_world_replacement_invalidate_without_restoring() {
        let mut host = server();
        let mut leases = Leases::default();
        leases
            .command(&mut host, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        assert!(host.kick(10, host.now_ms()));
        leases.sync(&mut host, &BTreeMap::new());
        assert!(leases.0.is_empty());
        let mut host = server();
        leases
            .command(&mut host, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        leases.world_changed(&mut host);
        leases.sync(&mut host, &BTreeMap::new());
        assert_eq!(host.instance_of(10), None);
        assert!(leases.0.is_empty());
    }
    #[test]
    fn external_authority_change_invalidates_lease_but_verifier_reset_retains_it() {
        let mut server = server();
        let mut leases = Leases::default();
        leases
            .command(&mut server, "tournament", 1, 10, destination(1000), true)
            .unwrap();
        let before = leases.verifier_snapshot(&server);
        server.teleport_now(10, destination(1000)).unwrap();
        leases.verifier_finished(before, &server);
        leases.sync(&mut server, &BTreeMap::new());
        assert_eq!(server.instance_of(10), Some(0));
        // Capture fresh accepted movement before a later independent lease.
        let mut server = super::tests::server();
        leases
            .command(&mut server, "tournament", 2, 10, destination(1000), true)
            .unwrap();
        server.teleport_now(10, destination(9)).unwrap();
        leases.sync(&mut server, &BTreeMap::new());
        assert_eq!(server.instance_of(10), Some(9));
        assert!(leases.0.is_empty());
    }
}
