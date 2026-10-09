//! Native destination resolution. Clients supply keys and content identity, never coordinates.
use skate_resources::locations::{EntryIntent, PreparedCatalog};
#[cfg(test)]
mod tests {
    use super::*;
    struct Pair {
        server: skate_net::dedicated::Server,
        clients: Vec<(u64, skate_net::lobby::Session, [f32; 3])>,
        now: u64,
    }
    impl Pair {
        fn new() -> Self {
            use skate_net::{
                dedicated::{Config, Server},
                lobby::{Info, Session},
            };
            let server = Server::new(Config {
                session: 7,
                server_id: 99,
                map: 1,
                max_players: 4,
            })
            .unwrap();
            let clients = [10, 20]
                .into_iter()
                .map(|id| {
                    let mut c = Session::dedicated_client(
                        7,
                        Info {
                            id,
                            map: 1,
                            rig: 2,
                            physics: 3,
                            appearance: 4,
                        },
                        99,
                    );
                    c.set_loopback(true);
                    (id, c, [-8., 1., 0.])
                })
                .collect();
            let mut p = Self {
                server,
                clients,
                now: 0,
            };
            p.pump();
            assert_eq!(p.server.competition_players().len(), 2);
            p
        }
        fn pump(&mut self) {
            use skate_net::{
                Body, Pose,
                packed::{self, BodyState, Packed},
            };
            for _ in 0..20 {
                self.now += 20;
                for (id, c, position) in &mut self.clients {
                    if let Some(reset) = c.pending_movement_reset() {
                        *position = reset.destination.position;
                        c.complete_movement_reset(reset.epoch);
                    }
                    let pose = Pose {
                        p: *position,
                        q: [0., 0., 0., 1.],
                    };
                    c.publish(
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
                        self.now,
                    );
                    for packet in c.service(self.now) {
                        self.server.receive(*id + 1, &packet.data, self.now);
                    }
                }
                for packet in self.server.service(self.now) {
                    for (id, c, _) in &mut self.clients {
                        if packet.peer == *id + 1 {
                            c.receive(99, &packet.data, self.now);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn one_actor_exit_and_owner_stop_preserve_the_other_player() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sdk/examples/interior-catalog");
        let mut package = PreparedCatalog::read(&root, "catalog.json").unwrap();
        package.catalog.locations[0].position = [-8., 0., 0.];
        package.catalog.locations[0].return_position = [-5., 0., 0.];
        package.catalog.interiors[0].exit.position = package.catalog.interiors[0].spawn;
        // The builtin provider must satisfy the same public manifest rules.
        assert!(provider(&package).is_ok());
        let mut p = Pair::new();
        let mut locations = Locations::default();
        let mut leases = crate::resources::teleport_leases::Leases::default();
        locations
            .catalogs
            .insert(ENGINE_OWNER.into(), (1, package.clone()));
        let message = |exit: bool, request: &str| skate_net::resources::Message {
            scope: Default::default(),
            id: 1,
            resource: ENGINE_OWNER.into(),
            generation: 1,
            kind: skate_net::resources::Kind::Event,
            name: if exit {
                "location_exit"
            } else {
                "location_entry"
            }
            .into(),
            value: serde_json::json!({"intent":{"location":"example-building","floor":"studio","generation":"1","request":request},"revision":package.revision}),
        };
        let peer_epoch = p.server.movement_epoch_of(20);
        assert!(locations.event(10, &message(false, "1"), &mut p.server, &mut leases));
        assert!(locations.visits.contains_key(&10));
        p.pump();
        assert_eq!(p.clients[0].2, [64., 0., 64.]);
        assert_eq!(p.server.movement_epoch_of(20), peer_epoch);
        locations.event(10, &message(true, "2"), &mut p.server, &mut leases);
        p.pump();
        assert_eq!(p.clients[0].2, [-5., 0., 0.]);
        assert_eq!(p.clients[1].2, [-8., 1., 0.]);
        // Move back into the entrance through a trusted test host operation.
        p.server
            .teleport_now(
                10,
                skate_net::dedicated::TeleportDestination {
                    position: [-8., 1., 0.],
                    heading: 0.,
                    velocity: [0.; 3],
                    instance: 0,
                },
            )
            .unwrap();
        p.pump();
        locations.event(10, &message(false, "3"), &mut p.server, &mut leases);
        p.pump();
        leases.sync(&mut p.server, &Default::default());
        p.pump();
        assert_eq!(p.clients[0].2, [-5., 0., 0.]);
        assert_eq!(p.clients[1].2, [-8., 1., 0.]);
    }
    #[test]
    fn readmission_evacuates_before_rebuilding_unchanged_catalog() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sdk/examples/interior-catalog");
        let mut package = PreparedCatalog::read(&root, "catalog.json").unwrap();
        package.catalog.locations[0].position = [-8., 0., 0.];
        package.catalog.locations[0].return_position = [-5., 0., 0.];
        package.catalog.interiors[0].exit.position = package.catalog.interiors[0].spawn;
        // The builtin provider must satisfy the same public manifest rules.
        assert!(provider(&package).is_ok());
        let mut p = Pair::new();
        let mut locations = Locations::default();
        let mut leases = crate::resources::teleport_leases::Leases::default();
        locations
            .catalogs
            .insert(ENGINE_OWNER.into(), (1, package.clone()));
        let message = |exit: bool, request: &str| skate_net::resources::Message {
            scope: Default::default(),
            id: 1,
            resource: ENGINE_OWNER.into(),
            generation: 1,
            kind: skate_net::resources::Kind::Event,
            name: if exit {
                "location_exit"
            } else {
                "location_entry"
            }
            .into(),
            value: serde_json::json!({"intent":{"location":"example-building","floor":"studio","generation":"1","request":request},"revision":package.revision}),
        };
        locations.event(10, &message(false, "1"), &mut p.server, &mut leases);
        p.pump();
        assert_eq!(p.clients[0].2, [64., 0., 64.]);
        locations
            .publish(&provider(&package).unwrap(), &mut p.server, &mut leases)
            .unwrap();
        let published = provider(&package).unwrap();
        p.server
            .configure_resources_returning(
                published.set.revision,
                80,
                [(ENGINE_OWNER.into(), 1)].into_iter().collect(),
                [10].into_iter().collect(),
            )
            .unwrap();
        p.pump();
        assert_eq!(p.clients[0].2, [-5., 0., 0.]);
        assert!(locations.visits.is_empty());
        assert_eq!(p.clients[1].2, [-8., 1., 0.]);
    }
    #[test]
    fn forged_floor_far_entry_and_revision_mismatch_are_rejected() {
        let catalog=serde_json::from_value(serde_json::json!({"version":1,"map":"test-world","interiors":[{"key":"room","model":"room.glb","collision":"shell.json","transform":[[1,0,0,0],[0,1,0,0],[0,0,1,0],[4096,100,4096,1]],"spawn":[4096,100,4096],"heading":0,"exit":{"position":[4096,100,4097],"style":{"color":[1,1,0],"opacity":0.3,"radius":1,"height":2}}}],"locations":[{"key":"entry","label":"Apartment","position":[0,0,0],"return_position":[3,0,0],"return_heading":0,"style":{"color":[1,1,0],"opacity":0.3,"radius":1,"height":2},"floors":[{"key":"one","label":"Room","interior":"room"}]}]})).unwrap();
        let p = PreparedCatalog {
            catalog,
            files: Default::default(),
            revision: "matching".into(),
        };
        let mut intent = EntryIntent {
            location: "entry".into(),
            floor: "one".into(),
            generation: "1".into(),
            request: "1".into(),
        };
        assert_eq!(
            resolve(&p, 1, &intent, "matching", [0., 1., 0.], false)
                .unwrap()
                .0,
            [4096., 100., 4096.]
        );
        assert!(resolve(&p, 1, &intent, "different", [0., 1., 0.], false).is_err());
        assert!(resolve(&p, 1, &intent, "matching", [10., 1., 0.], false).is_err());
        intent.floor = "forged".into();
        assert!(resolve(&p, 1, &intent, "matching", [0., 1., 0.], false).is_err());
    }
}

pub fn resolve(
    package: &PreparedCatalog,
    generation: u64,
    intent: &EntryIntent,
    revision: &str,
    position: [f32; 3],
    exit: bool,
) -> Result<([f32; 3], f32), String> {
    intent.validate()?;
    if intent.generation != generation.to_string() || revision != package.revision {
        return Err("Interior content or generation mismatch".into());
    }
    let l = package
        .catalog
        .locations
        .iter()
        .find(|l| l.key == intent.location)
        .ok_or("Unknown entrance")?;
    let f = l
        .floors
        .iter()
        .find(|f| f.key == intent.floor)
        .ok_or("Unknown floor")?;
    let i = package
        .catalog
        .interiors
        .iter()
        .find(|i| i.key == f.interior)
        .ok_or("Unknown interior")?;
    let (at, style) = if exit {
        (i.exit.position, &i.exit.style)
    } else {
        (l.position, &l.style)
    };
    // Same conservative admitted root capsule used by ordinary dedicated movement.
    // Exact articulated collision remains the native verifier's responsibility.
    let horizontal = (position[0] - at[0]).hypot(position[2] - at[2]);
    if position.iter().any(|v| !v.is_finite())
        || horizontal > style.radius + 0.45
        || position[1] < at[1] - 0.5
        || position[1] > at[1] + style.height + 0.5
    {
        return Err("Player does not intersect the location marker".into());
    }
    Ok(if exit {
        (l.return_position, l.return_heading)
    } else {
        (i.spawn, i.heading)
    })
}
#[derive(Default)]
pub struct Locations {
    notifications: Vec<(String, u64, serde_json::Value)>,
    catalogs: std::collections::BTreeMap<String, (u64, PreparedCatalog)>,
    settings: std::collections::BTreeMap<String, skate_resources::locations::LocationSnapshot>,
    visits: std::collections::BTreeMap<u64, (String, EntryIntent, [f32; 3], f32)>,
    requests: std::collections::BTreeMap<(u64, String), (u64, u64)>,
}
impl Locations {
    pub(crate) fn notifications(&mut self) -> Vec<(String, u64, serde_json::Value)> {
        std::mem::take(&mut self.notifications)
    }
    pub(crate) fn publish(
        &mut self,
        published: &skate_resources::PublishedSet,
        server: &mut skate_net::dedicated::Server,
        leases: &mut crate::resources::teleport_leases::Leases,
    ) -> Result<Vec<u64>, String> {
        let mut catalogs = std::collections::BTreeMap::new();
        for resource in &published.set.resources {
            if let Some(p) = PreparedCatalog::from_resource(resource, &published.blobs)? {
                for i in &p.catalog.interiors {
                    skate_data::location_collision::decode(&p.files[&i.collision], i.transform)?;
                }
                catalogs.insert(resource.manifest.id.clone(), (resource.generation, p));
            }
        }
        // Every content publication triggers a new client readiness barrier.
        // Evacuate visits first, even if this owner's bytes did not change:
        // clients retire all resource-owned collision while rebuilding admission.
        let retired: Vec<_> = self.visits.iter().map(|(&a, v)| (a, v.2, v.3)).collect();
        for &(actor, _, _) in &retired {
            let (owner, intent, _, _) = &self.visits[&actor];
            leases.request_return(
                server,
                owner,
                skate_resources::locations::generation(&intent.generation)?,
                actor,
            )?;
            self.visits.remove(&actor);
        }
        leases.sync(
            server,
            &published
                .set
                .resources
                .iter()
                .map(|r| (r.manifest.id.clone(), r.generation))
                .collect(),
        );
        self.settings.retain(|id, s| {
            catalogs
                .get(id)
                .is_some_and(|(g, p)| s.validate_catalog(&p.catalog, *g).is_ok())
        });
        self.catalogs = catalogs;
        Ok(retired.into_iter().map(|(actor, _, _)| actor).collect())
    }
    pub fn state(
        &mut self,
        owner: &str,
        generation: u64,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        if value.is_null() {
            self.settings.remove(owner);
            return Ok(());
        }
        let (g, p) = self
            .catalogs
            .get(owner)
            .ok_or("Location owner has no catalog")?;
        if *g != generation {
            return Err("Stale location generation".into());
        }
        let settings = skate_resources::locations::LocationSnapshot::parse(value.clone())?;
        settings.validate_catalog(&p.catalog, generation)?;
        self.settings.insert(owner.into(), settings);
        Ok(())
    }
    pub fn prune(&mut self, server: &skate_net::dedicated::Server) {
        self.visits
            .retain(|actor, _| server.peer_for_actor(*actor).is_some());
        self.requests
            .retain(|(actor, _), _| server.peer_for_actor(*actor).is_some());
    }
    pub(crate) fn event(
        &mut self,
        actor: u64,
        m: &skate_net::resources::Message,
        server: &mut skate_net::dedicated::Server,
        leases: &mut crate::resources::teleport_leases::Leases,
    ) -> bool {
        if m.name != "location_entry" && m.name != "location_exit" {
            return false;
        }
        let result = (|| -> Result<u64, String> {
            if !server.resource_ready(actor) {
                return Err("Player has not completed interior admission".into());
            }
            let (generation, package) = self
                .catalogs
                .get(&m.resource)
                .ok_or("No admitted catalog for this owner")?;
            if m.generation != *generation {
                return Err("Stale catalog owner".into());
            }
            let intent: EntryIntent = serde_json::from_value(
                m.value
                    .get("intent")
                    .cloned()
                    .ok_or("Missing location intent")?,
            )
            .map_err(|e| e.to_string())?;
            let revision = m
                .value
                .get("revision")
                .and_then(|v| v.as_str())
                .ok_or("Missing interior revision")?;
            let player = server
                .competition_players()
                .into_iter()
                .find(|p| p.actor == actor)
                .ok_or("Fresh admitted player position required")?;
            let request = skate_resources::locations::generation(&intent.request)?;
            if self
                .requests
                .get(&(actor, m.resource.clone()))
                .is_some_and(|(epoch, old)| *epoch == player.epoch && request <= *old)
            {
                return Err("Replayed interior request".into());
            }
            let exit = m.name == "location_exit";
            if exit
                && !self.visits.get(&actor).is_some_and(|v| {
                    v.0 == m.resource
                        && v.1.location == intent.location
                        && v.1.floor == intent.floor
                })
            {
                return Err("Player does not occupy this destination".into());
            }
            if !exit && self.visits.contains_key(&actor) {
                return Err("Exit the current interior first".into());
            }
            let mut effective = package.clone();
            if !exit {
                if let Some(setting) = self
                    .settings
                    .get(&m.resource)
                    .and_then(|s| s.locations.iter().find(|s| s.key == intent.location))
                {
                    if !setting.enabled {
                        return Err("Entrance is disabled".into());
                    }
                    if let Some(entry) = effective
                        .catalog
                        .locations
                        .iter_mut()
                        .find(|l| l.key == intent.location)
                    {
                        entry.style = setting.style.clone();
                    }
                }
            }
            let (position, heading) = resolve(
                &effective,
                *generation,
                &intent,
                revision,
                player.position,
                exit,
            )?;
            let destination = skate_net::dedicated::TeleportDestination {
                position,
                heading,
                velocity: [0.; 3],
                instance: player.instance,
            };
            if exit {
                leases.command(server, &m.resource, *generation, actor, destination, false)?;
            } else {
                let l = package
                    .catalog
                    .locations
                    .iter()
                    .find(|l| l.key == intent.location)
                    .unwrap();
                leases.location_entry(
                    server,
                    &m.resource,
                    *generation,
                    actor,
                    destination,
                    skate_net::dedicated::TeleportDestination {
                        position: l.return_position,
                        heading: l.return_heading,
                        velocity: [0.; 3],
                        instance: player.instance,
                    },
                )?;
            }
            let epoch = server
                .movement_epoch_of(actor)
                .ok_or("Player disconnected during travel")?;
            self.requests
                .insert((actor, m.resource.clone()), (epoch, request));
            if exit {
                self.visits.remove(&actor);
            } else {
                let l = package
                    .catalog
                    .locations
                    .iter()
                    .find(|l| l.key == intent.location)
                    .unwrap();
                self.visits.insert(
                    actor,
                    (
                        m.resource.clone(),
                        intent,
                        l.return_position,
                        l.return_heading,
                    ),
                );
            }
            if let Some(name) = self
                .settings
                .get(&m.resource)
                .and_then(|s| {
                    s.locations.iter().find(|s| {
                        Some(s.key.as_str())
                            == m.value.pointer("/intent/location").and_then(|v| v.as_str())
                    })
                })
                .and_then(|s| s.interaction.as_ref())
            {
                if self.notifications.len() < 128 {
                    self.notifications.push((m.resource.clone(),m.generation,serde_json::json!({"name":name,"actor":actor.to_string(),"exit":exit,"intent":m.value.get("intent")})));
                }
            }
            Ok(epoch)
        })();
        let value = serde_json::json!({"request":m.value.pointer("/intent/request"),"exit":m.name=="location_exit","ok":result.is_ok(),"epoch":result.as_ref().ok().map(u64::to_string),"error":result.err()});
        let _ = server.send_resource(
            Some(actor),
            skate_net::resources::Message {
                scope: Default::default(),
                id: 1,
                resource: m.resource.clone(),
                generation: m.generation,
                kind: skate_net::resources::Kind::Event,
                name: "location_result".into(),
                value,
            },
        );
        true
    }
}

pub const ENGINE_OWNER: &str = "engine-locations";
pub fn provider(package: &PreparedCatalog) -> Result<skate_resources::PublishedSet, String> {
    let mut files = package.files.clone();
    files.insert(
        "catalog.json".into(),
        serde_json::to_vec(&package.catalog).map_err(|e| e.to_string())?,
    );
    let manifest:skate_resources::Manifest=serde_json::from_value(serde_json::json!({"format":1,"api":1,"id":ENGINE_OWNER,"version":"1.0.0","language":"lua","locations":"catalog.json","files":files.keys().collect::<Vec<_>>()})).map_err(|e|e.to_string())?;
    skate_resources::PublishedSet::from_memory(manifest, 1, files).map_err(|e| e.to_string())
}
