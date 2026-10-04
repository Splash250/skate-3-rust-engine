//! Resource-owned shared dynamics. Each instance has a separate Rapier world;
//! client controller leases never permit unvalidated transform publication.
use skate_dynamics::{BodyDesc, DynamicsWorld};
use skate_net::{
    dedicated::Server,
    entities::{self, Command, Entity},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerEntityContact {
    pub entity: u64,
    pub actor: u64,
    pub contact_steps: u64,
    pub first_tick: u64,
    pub last_tick: u64,
}
#[derive(Clone, Debug, Default)]
pub struct ContactMetrics {
    pub pairs: Vec<PlayerEntityContact>,
    pub omitted_contact_steps: u64,
}

#[derive(Default)]
pub struct ServerWorld {
    generations: BTreeMap<String, u64>,
    terrain: Option<crate::world::Terrain>,
    instances: BTreeMap<u64, DynamicsWorld>,
    objects: BTreeMap<u64, (u64, Entity)>,
    proxies: BTreeMap<(u64, u64), (u64, u64)>,
    next: u64,
    tick: u64,
    contacts: BTreeMap<(u64,u64),PlayerEntityContact>,
    omitted_contact_steps: u64,
}
impl ServerWorld {
    /// At most 256 live entity/player pairs. Counts refer to solver-contact
    /// steps, not broadphase proximity or client-submitted contact claims.
    pub fn contact_metrics(&self) -> ContactMetrics {
        ContactMetrics {pairs:self.contacts.values().cloned().collect(),omitted_contact_steps:self.omitted_contact_steps}
    }
    pub fn terrain(&self) -> Option<&crate::world::Terrain> {self.terrain.as_ref()}
    /// Stage a complete set of instance grounds and bodies before replacing
    /// the live worlds. IDs remain stable; pose epochs retire interpolation.
    pub fn set_terrain(&mut self, terrain: Option<crate::world::Terrain>) -> Result<(),String> {
        if self.terrain.as_ref().map(|t|&t.revision)==terrain.as_ref().map(|t|&t.revision) {return Ok(());}
        let mut instances=BTreeMap::new();
        let mut objects=BTreeMap::new();
        let tick=self.tick.checked_add(1).ok_or("Shared entity tick exhausted")?;
        for (&id,(_,entity)) in &self.objects {
            let world=match instances.entry(entity.instance) {
                std::collections::btree_map::Entry::Occupied(e)=>e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e)=>e.insert(terrain_world(terrain.as_ref())?),
            };
            let body=world.spawn(description(&entity.definition,entity.position))?;
            world.set_pose(body,entity.position,entity.rotation);
            world.set_linvel(body,entity.velocity);world.set_angvel(body,entity.angular);
            if entity.definition.body_type==entities::BodyType::Kinematic {world.set_kinematic_motion(body,entity.velocity,entity.angular);}
            let mut snapshot=entity.clone();snapshot.epoch=snapshot.epoch.checked_add(1).ok_or("Entity pose epoch exhausted")?;snapshot.tick=tick;
            objects.insert(id,(body,snapshot));
        }
        self.instances=instances;self.objects=objects;self.proxies.clear();self.terrain=terrain;self.tick=tick;
        Ok(())
    }

    pub fn sync_resources(&mut self, generations: BTreeMap<String, u64>) {
        let stale: Vec<_> = self
            .objects
            .iter()
            .filter(|(_, (_, e))| generations.get(&e.resource) != Some(&e.generation))
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            self.remove(id);
        }
        self.generations = generations;
        self.trim_instances();
    }
    pub fn command(
        &mut self,
        resource: &str,
        generation: u64,
        command: Command,
        server: &mut Server,
    ) -> Result<u64, String> {
        if !entities::label(resource)
            || generation == 0
            || self.generations.get(resource) != Some(&generation)
        {
            return Err("Shared entity owner generation is not active".into());
        }
        if let Command::Spawn(spawn) = command {
            if !spawn.valid()
                || self.objects.len() >= entities::MAX_ENTITIES
                || self
                    .objects
                    .values()
                    .any(|(_, e)| e.resource == resource && e.key == spawn.key)
                || (self.instances.len() >= entities::MAX_INSTANCES
                    && !self.instances.contains_key(&spawn.instance))
            {
                return Err("Invalid, duplicate or over-budget shared entity".into());
            }
            validate_controller(spawn.controller, spawn.instance, server)?;
            let id = self
                .next
                .checked_add(1)
                .ok_or("Shared entity IDs exhausted")?;
            let tick = self.advance()?;
            if !self.instances.contains_key(&spawn.instance) {
                self.instances.insert(spawn.instance,terrain_world(self.terrain.as_ref())?);
            }
            let world = self.instances.get_mut(&spawn.instance).unwrap();
            let body = world.spawn(description(&spawn.definition, spawn.position))?;
            world.set_pose(body, spawn.position, spawn.rotation);
            world.set_linvel(body, spawn.velocity);
            if spawn.definition.body_type == entities::BodyType::Kinematic {
                world.set_kinematic_motion(body, spawn.velocity, [0.; 3]);
            }
            self.objects.insert(
                id,
                (
                    body,
                    Entity {
                        id,
                        resource: resource.into(),
                        generation,
                        key: spawn.key,
                        instance: spawn.instance,
                        controller: spawn.controller,
                        definition: spawn.definition,
                        tick,
                        epoch: 1,
                        position: spawn.position,
                        rotation: spawn.rotation,
                        velocity: spawn.velocity,
                        angular: [0.; 3],
                    },
                ),
            );
            self.next = id;
            return Ok(id);
        }
        let key = match &command {
            Command::Remove { key }
            | Command::Impulse { key, .. }
            | Command::Velocity { key, .. }
            | Command::Pose { key, .. }
            | Command::Transfer { key, .. } => key,
            Command::Spawn(_) => unreachable!(),
        };
        if !entities::label(key) {
            return Err("Invalid shared entity key".into());
        }
        let id = self
            .objects
            .iter()
            .find(|(_, (_, e))| {
                e.resource == resource && e.generation == generation && &e.key == key
            })
            .map(|(&id, _)| id)
            .ok_or("Unknown owned shared entity")?;
        if matches!(command, Command::Remove { .. }) {
            self.remove(id);
            self.trim_instances();
            return Ok(id);
        }
        let tick = self.advance()?;
        let (body, entity) = self.objects.get_mut(&id).unwrap();
        let world = self.instances.get_mut(&entity.instance).unwrap();
        match command {
            Command::Impulse { impulse, .. }
                if entities::vector(impulse, 100_000.)
                    && entity.definition.body_type == entities::BodyType::Dynamic =>
            {
                world.apply_impulse(*body, impulse, None);
            }
            Command::Velocity { velocity, .. }
                if entities::vector(velocity, 200.)
                    && entity.definition.body_type != entities::BodyType::Static =>
            {
                world.set_linvel(*body, velocity);
                world.set_kinematic_motion(*body, velocity, [0.; 3]);
            }
            Command::Pose {
                position, rotation, ..
            } if entities::vector(position, 99_999.) && entities::quaternion(rotation) => {
                entity.epoch = entity
                    .epoch
                    .checked_add(1)
                    .ok_or("Entity pose epoch exhausted")?;
                world.set_pose(*body, position, rotation);
                world.set_linvel(*body, [0.; 3]);
                world.set_angvel(*body, [0.; 3]);
                world.set_kinematic_motion(*body, [0.; 3], [0.; 3]);
            }
            Command::Transfer { controller, .. } => {
                validate_controller(controller, entity.instance, server)?;
                entity.controller = controller;
            }
            _ => return Err("Invalid shared entity operation".into()),
        }
        refresh(world, *body, entity, tick);
        Ok(id)
    }
    pub fn step(&mut self, dt: f32, server: &mut Server) {
        if !dt.is_finite() || dt <= 0. {
            return;
        }
        let dt = dt.min(0.1);
        let players = server.entity_players();
        for (_, e) in self.objects.values_mut() {
            if e.controller.is_some_and(|id| {
                server.instance_of(id) != Some(e.instance) || !server.resource_ready(id)
            }) {
                e.controller = None;
            }
        }
        let live: BTreeSet<_> = players
            .iter()
            .filter(|p| self.instances.contains_key(&p.instance))
            .map(|p| (p.instance, p.actor))
            .collect();
        let stale: Vec<_> = self
            .proxies
            .keys()
            .filter(|key| !live.contains(key))
            .copied()
            .collect();
        for key in stale {
            if let Some((body, _)) = self.proxies.remove(&key) {
                if let Some(world) = self.instances.get_mut(&key.0) {
                    world.remove(body);
                }
            }
        }
        for p in players {
            let Some(world) = self.instances.get_mut(&p.instance) else {
                continue;
            };
            let key = (p.instance, p.actor);
            let prior = self.proxies.get(&key).copied();
            let previous = prior.and_then(|(body, epoch)| {
                if epoch == p.epoch {
                    Some(body)
                } else {
                    world.remove(body);
                    None
                }
            });
            // Current-frame placement, not a kinematic target across the map:
            // teleports never turn the old-to-new path into object contact.
            if let Ok(body) = world.upsert_kinematic_proxy(
                previous,
                skate_dynamics::Shape::Capsule {
                    half_height: 0.55,
                    radius: 0.35,
                },
                [p.position[0], p.position[1] + 0.9, p.position[2]],
                [0., 0., 0., 1.],
                0.7,
            ) {
                world.set_kinematic_motion(body, p.velocity, [0.; 3]);
                self.proxies.insert(key, (body, p.epoch));
            }
        }
        let Ok(tick) = self.advance() else {
            return;
        };
        for world in self.instances.values_mut() {
            world.step(dt);
            // DynamicsWorld retains edge events until consumed. This host uses
            // the stricter solver query below, but must also release that queue.
            world.drain_contacts();
        }
        self.contacts.retain(|(entity,actor),_|self.objects.get(entity).is_some_and(|(_,e)|server.instance_of(*actor)==Some(e.instance)));
        // Both indexes are bounded by the existing 256 entities and 64 admitted
        // players; solver pairs are traversed without allocating another list.
        let objects:BTreeMap<_,_>=self.objects.iter().map(|(&id,(body,e))|((e.instance,*body),id)).collect();
        let players:BTreeMap<_,_>=self.proxies.iter().map(|(&(instance,actor),&(body,_))|((instance,body),actor)).collect();
        for (&instance,world) in &self.instances {
            for (a,b) in world.active_solver_contact_pairs() {
                let pair=objects.get(&(instance,a)).zip(players.get(&(instance,b)))
                    .or_else(||objects.get(&(instance,b)).zip(players.get(&(instance,a))));
                let Some((&entity,&actor))=pair else {continue;};
                if let Some(contact)=self.contacts.get_mut(&(entity,actor)) {
                    if contact.last_tick!=tick {
                        contact.contact_steps=contact.contact_steps.saturating_add(1);
                        contact.last_tick=tick;
                    }
                } else if self.contacts.len()<256 {
                    self.contacts.insert((entity,actor),PlayerEntityContact {entity,actor,contact_steps:1,first_tick:tick,last_tick:tick});
                    if std::env::var("SKATE_RESOURCE_DIAGNOSTICS").is_ok_and(|value|value=="1") {
                        let e=&self.objects[&entity].1;
                        eprintln!("RESOURCE_ENTITY_CONTACT entity={entity} actor={actor} resource={} generation={} key={} instance={instance} tick={tick}",e.resource,e.generation,e.key);
                    }
                } else {
                    self.omitted_contact_steps=self.omitted_contact_steps.saturating_add(1);
                }
            }
        }
        let mut invalid = Vec::new();
        for (&id, (body, e)) in &mut self.objects {
            refresh(&self.instances[&e.instance], *body, e, tick);
            if !e.valid() {
                invalid.push(id);
            }
        }
        for id in invalid {
            self.remove(id);
        }
        self.trim_instances();
        let _ = server.publish_entities(self.snapshot());
    }
    pub fn snapshot(&self) -> Vec<Entity> {
        self.objects.values().map(|(_, e)| e.clone()).collect()
    }
    fn advance(&mut self) -> Result<u64, String> {
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or("Entity sample sequence exhausted")?;
        Ok(self.tick)
    }
    fn remove(&mut self, id: u64) {
        self.contacts.retain(|(entity,_),_|*entity!=id);
        if let Some((body, e)) = self.objects.remove(&id) {
            if let Some(world) = self.instances.get_mut(&e.instance) {
                world.remove(body);
            }
        }
    }
    fn trim_instances(&mut self) {
        let live: BTreeSet<_> = self.objects.values().map(|(_, e)| e.instance).collect();
        self.instances.retain(|id, _| live.contains(id));
        self.proxies
            .retain(|(instance, _), _| live.contains(instance));
    }
}
fn terrain_world(terrain:Option<&crate::world::Terrain>) -> Result<DynamicsWorld,String> {
    let mut world=DynamicsWorld::default();
    if let Some(terrain)=terrain {world.set_ground(terrain.triangles.iter().copied())?;}
    Ok(world)
}
fn validate_controller(
    controller: Option<u64>,
    instance: u64,
    server: &Server,
) -> Result<(), String> {
    if controller.is_some_and(|id| {
        id == 0 || server.instance_of(id) != Some(instance) || !server.resource_ready(id)
    }) {
        Err("Entity controller is not admitted to its instance".into())
    } else {
        Ok(())
    }
}
fn refresh(world: &DynamicsWorld, body: u64, e: &mut Entity, tick: u64) {
    if let Some(s) = world.read(body) {
        e.position = s.position;
        e.rotation = s.rotation;
        e.velocity = s.linvel;
        e.angular = s.angvel;
        e.tick = tick;
    }
}
fn description(def: &entities::Definition, position: [f32; 3]) -> BodyDesc {
    let shape = match def.shape {
        entities::Shape::Box { half_extents } => skate_dynamics::Shape::Box { half_extents },
        entities::Shape::Sphere { radius } => skate_dynamics::Shape::Sphere { radius },
        entities::Shape::Capsule {
            half_height,
            radius,
        } => skate_dynamics::Shape::Capsule {
            half_height,
            radius,
        },
    };
    let body_type = match def.body_type {
        entities::BodyType::Dynamic => skate_dynamics::BodyType::Dynamic,
        entities::BodyType::Kinematic => skate_dynamics::BodyType::Kinematic,
        entities::BodyType::Static => skate_dynamics::BodyType::Static,
    };
    BodyDesc {
        shape,
        body_type,
        mass: def.mass,
        friction: def.friction,
        position,
        ccd: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_net::entities::{BodyType, Definition, Shape, Spawn};
    fn server() -> Server {
        Server::new(skate_net::dedicated::Config {
            session: 1,
            server_id: 99,
            map: 1,
            max_players: 16,
        })
        .unwrap()
    }
    fn spawn(key: &str, instance: u64, body_type: BodyType, position: [f32; 3]) -> Command {
        Command::Spawn(Spawn {
            key: key.into(),
            instance,
            controller: None,
            definition: Definition {
                shape: Shape::Box {
                    half_extents: [0.5; 3],
                },
                body_type,
                mass: 2.,
                friction: 0.7,
                color: [0.5, 0.5, 0.5, 1.],
            },
            position,
            rotation: [0., 0., 0., 1.],
            velocity: [0.; 3],
        })
    }
    #[test]
    fn dynamic_object_collides_with_server_owned_floor_only_in_its_instance() {
        let mut net = server();
        let mut world = ServerWorld::default();
        world.sync_resources(BTreeMap::from([("objects".into(), 1)]));
        world
            .command(
                "objects",
                1,
                spawn("floor", 0, BodyType::Static, [0., 0., 0.]),
                &mut net,
            )
            .unwrap();
        let supported = world
            .command(
                "objects",
                1,
                spawn("supported", 0, BodyType::Dynamic, [0., 2., 0.]),
                &mut net,
            )
            .unwrap();
        let isolated = world
            .command(
                "objects",
                1,
                spawn("isolated", 1, BodyType::Dynamic, [0., 2., 0.]),
                &mut net,
            )
            .unwrap();
        for _ in 0..120 {
            world.step(1. / 60., &mut net);
        }
        assert!(world.instances.values_mut().all(|instance|instance.drain_contacts().is_empty()),"server consumes physics contact edges each tick instead of accumulating history");
        let values = world.snapshot();
        let a = values.iter().find(|e| e.id == supported).unwrap();
        let b = values.iter().find(|e| e.id == isolated).unwrap();
        assert!(
            (a.position[1] - 1.).abs() < 0.03,
            "floor contact: {:?}",
            a.position
        );
        assert!(
            b.position[1] < -5.,
            "another instance must not collide with floor"
        );
    }
    #[test]
    fn restart_retires_objects_and_stale_or_cross_resource_commands_cannot_change_them() {
        let mut net = server();
        let mut world = ServerWorld::default();
        world.sync_resources(BTreeMap::from([("objects".into(), 1), ("other".into(), 1)]));
        let old = world
            .command(
                "objects",
                1,
                spawn("box", 0, BodyType::Dynamic, [0., 2., 0.]),
                &mut net,
            )
            .unwrap();
        assert!(
            world
                .command("other", 1, Command::Remove { key: "box".into() }, &mut net)
                .is_err()
        );
        world.sync_resources(BTreeMap::from([("objects".into(), 2)]));
        assert!(world.snapshot().is_empty());
        assert!(
            world
                .command(
                    "objects",
                    1,
                    Command::Remove { key: "box".into() },
                    &mut net
                )
                .is_err()
        );
        let new = world
            .command(
                "objects",
                2,
                spawn("box", 0, BodyType::Dynamic, [0., 2., 0.]),
                &mut net,
            )
            .unwrap();
        assert_ne!(old, new);
    }
    #[test]
    fn required_world_ground_replaces_atomically_and_preserves_entity_identity() {
        let mut net=server();let mut world=ServerWorld::default();
        world.sync_resources(BTreeMap::from([("objects".into(),1)]));
        let id=world.command("objects",1,spawn("box",7,BodyType::Dynamic,[0.,3.,0.]),&mut net).unwrap();
        let terrain=crate::world::Terrain {revision:"first".into(),spawn:[0.,1.,0.],heading:0.,
            triangles:std::sync::Arc::new(vec![[[-10.,0.,-10.],[-10.,0.,10.],[10.,0.,10.]],
                [[-10.,0.,-10.],[10.,0.,10.],[10.,0.,-10.]]])};
        world.set_terrain(Some(terrain.clone())).unwrap();
        let epoch=world.snapshot()[0].epoch;
        for _ in 0..120 {world.step(1./60.,&mut net);}
        assert_eq!(world.snapshot()[0].id,id);
        assert!((world.snapshot()[0].position[1]-0.5).abs()<0.03);
        let bad=crate::world::Terrain {revision:"invalid".into(),triangles:Default::default(),..terrain};
        assert!(world.set_terrain(Some(bad)).is_err());
        assert_eq!(world.terrain().unwrap().revision,"first");
        assert_eq!(world.snapshot()[0].epoch,epoch);
        world.set_terrain(None).unwrap();
        assert!(world.snapshot()[0].epoch>epoch);
        for _ in 0..120 {world.step(1./60.,&mut net);}
        assert_eq!(world.snapshot()[0].id,id);
        assert!(world.snapshot()[0].position[1] < -5.);
    }

}
