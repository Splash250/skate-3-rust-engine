//! Dedicated server-owned object presentation and native finite-mass shadows.
use super::Multiplayer;
use bevy::prelude::*;
use skate_dynamics::{BodyDefinition, BodyDesc, DynamicsWorld, SolidBody};
use skate_net::entities::{BodyType, Definition, Entity as State, Shape};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

struct Object {
    body: u64,
    visual: Entity,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    samples: VecDeque<(f64, State)>,
}
#[derive(Resource, Default)]
pub(crate) struct SharedObjects {
    world: DynamicsWorld,
    objects: BTreeMap<u64, Object>,
    now: f64,
}
impl SharedObjects {
    pub(crate) fn diagnostic_summary(&self) -> String {
        let entries: Vec<_> = self.objects.iter().filter_map(|(&id, object)| {
            sample(&object.samples, self.now).map(|(position, _, _, _)| (id, position))
        }).collect();
        format!("registered:{} live_solids:{} positions:{entries:?}", self.objects.len(), self.solids().len())
    }
    pub(crate) fn solids(&self) -> Vec<SolidBody> {
        let live: BTreeMap<_, _> = self
            .objects
            .iter()
            .filter(|(_, o)| o.samples.back().is_some_and(|(at, _)| self.now - *at <= 1.))
            .map(|(&id, o)| (o.body, id))
            .collect();
        self.world
            .solid_bodies()
            .into_iter()
            .filter_map(|mut s| {
                let id = *live.get(&s.id)?;
                // Separate native query identity from the local mod DynamicsWorld.
                s.id = 0x4000_0000 | (id & 0x3fff_ffff);
                Some(s)
            })
            .collect()
    }
}
pub(super) fn sync(
    mut commands: Commands,
    net: Res<Multiplayer>,
    mut shared: ResMut<SharedObjects>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let now = net.started.elapsed().as_secs_f64();
    shared.now = now;
    let incoming = net
        .lobby
        .as_ref()
        .filter(|l| l.is_dedicated() && l.connected())
        .map(|l| l.entities.entities());
    let live: BTreeSet<_> = incoming
        .into_iter()
        .flat_map(|entries| entries.keys().copied())
        .collect();
    let removed: Vec<_> = shared
        .objects
        .keys()
        .filter(|id| !live.contains(id))
        .copied()
        .collect();
    for id in removed {
        let o = shared.objects.remove(&id).unwrap();
        shared.world.remove(o.body);
        commands.entity(o.visual).despawn();
        meshes.remove(&o.mesh);
        materials.remove(&o.material);
    }
    for (&id, state) in incoming.into_iter().flatten() {
        if let Some(object) = shared.objects.get_mut(&id) {
            let old = &object.samples.back().unwrap().1;
            if old.tick >= state.tick {
                continue;
            }
            if old.epoch != state.epoch || old.generation != state.generation {
                object.samples.clear();
            }
            object.samples.push_back((now, state.clone()));
            while object.samples.len() > 8 {
                object.samples.pop_front();
            }
            continue;
        }
        let Ok(body) = shared.world.spawn_replica(&BodyDefinition {
            body: description(&state.definition),
            extras: Vec::new(),
            deformation: None,
        }) else {
            continue;
        };
        shared.world.set_pose(body, state.position, state.rotation);
        let mesh = meshes.add(match state.definition.shape {
            Shape::Box { half_extents } => {
                Mesh::from(Cuboid::from_size(Vec3::from_array(half_extents) * 2.))
            }
            Shape::Sphere { radius } => Mesh::from(Sphere::new(radius)),
            Shape::Capsule {
                half_height,
                radius,
            } => Mesh::from(Capsule3d::new(radius, half_height * 2.)),
        });
        let [r, g, b, a] = state.definition.color;
        let material = materials.add(StandardMaterial {
            base_color: Color::srgba(r, g, b, a),
            perceptual_roughness: 0.65,
            ..Default::default()
        });
        let visual = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(Vec3::from_array(state.position))
                    .with_rotation(Quat::from_array(state.rotation)),
                Name::new(format!("Shared {}:{}", state.resource, state.key)),
            ))
            .id();
        shared.objects.insert(
            id,
            Object {
                body,
                visual,
                mesh,
                material,
                samples: VecDeque::from([(now, state.clone())]),
            },
        );
    }
}
pub(crate) fn prepare(
    net: Res<Multiplayer>,
    mut shared: ResMut<SharedObjects>,
    mut physics: ResMut<crate::physics::GamePhysics>,
    skater: Res<crate::physics::SkaterRuntime>,
) {
    let now = net.started.elapsed().as_secs_f64();
    shared.now = now;
    let samples: Vec<_> = shared
        .objects
        .values()
        .filter_map(|o| sample(&o.samples, now).map(|s| (o.body, s)))
        .collect();
    for (id, (p, q, v, w)) in samples {
        shared.world.set_pose(id, p, q);
        shared.world.set_linvel(id, v);
        shared.world.set_angvel(id, w);
    }
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    for solid in shared.solids() {
        proxies.append_solid(solid, &physics, &skater, false);
    }
    physics.network_proxies = proxies;
}
pub(super) fn render(
    net: Res<Multiplayer>,
    shared: Res<SharedObjects>,
    mut visuals: Query<(&mut Transform, &mut Visibility)>,
) {
    let now = net.started.elapsed().as_secs_f64();
    for o in shared.objects.values() {
        let Ok((mut transform, mut visibility)) = visuals.get_mut(o.visual) else {
            continue;
        };
        if let Some((p, q, _, _)) = sample(&o.samples, now) {
            transform.translation = Vec3::from_array(p);
            transform.rotation = Quat::from_array(q);
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}
type Motion = ([f32; 3], [f32; 4], [f32; 3], [f32; 3]);
fn sample(samples: &VecDeque<(f64, State)>, now: f64) -> Option<Motion> {
    let (latest_at, latest) = samples.back()?;
    if now - *latest_at > 1. {
        return None;
    }
    let target = now - 0.08;
    for index in 1..samples.len() {
        let (at, a) = &samples[index - 1];
        let (bt, b) = &samples[index];
        if *bt >= target && *bt > *at {
            let t = ((target - at) / (bt - at)).clamp(0., 1.) as f32;
            return Some((
                Vec3::from_array(a.position)
                    .lerp(Vec3::from_array(b.position), t)
                    .to_array(),
                Quat::from_array(a.rotation)
                    .slerp(Quat::from_array(b.rotation), t)
                    .normalize()
                    .to_array(),
                Vec3::from_array(a.velocity)
                    .lerp(Vec3::from_array(b.velocity), t)
                    .to_array(),
                b.angular,
            ));
        }
    }
    let dt = (target - latest_at).clamp(0., 0.1) as f32;
    let p = Vec3::from_array(latest.position) + Vec3::from_array(latest.velocity) * dt;
    let q = (Quat::from_scaled_axis(Vec3::from_array(latest.angular) * dt)
        * Quat::from_array(latest.rotation))
    .normalize();
    Some((p.to_array(), q.to_array(), latest.velocity, latest.angular))
}
fn description(def: &Definition) -> BodyDesc {
    let shape = match def.shape {
        Shape::Box { half_extents } => skate_dynamics::Shape::Box { half_extents },
        Shape::Sphere { radius } => skate_dynamics::Shape::Sphere { radius },
        Shape::Capsule {
            half_height,
            radius,
        } => skate_dynamics::Shape::Capsule {
            half_height,
            radius,
        },
    };
    let body_type = match def.body_type {
        BodyType::Dynamic => skate_dynamics::BodyType::Dynamic,
        BodyType::Kinematic => skate_dynamics::BodyType::Kinematic,
        BodyType::Static => skate_dynamics::BodyType::Static,
    };
    BodyDesc {
        shape,
        body_type,
        mass: def.mass,
        friction: def.friction,
        ccd: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(tick: u64, x: f32) -> State {
        State {
            id: 1, resource: "test".into(), generation: 1, key: "crate".into(),
            instance: 0, controller: None,
            definition: Definition { shape: Shape::Box { half_extents: [0.5; 3] },
                body_type: BodyType::Dynamic, mass: 1., friction: 0.7, color: [1.; 4] },
            tick, epoch: 1, position: [x, 0., 0.], rotation: [0., 0., 0., 1.],
            velocity: [10., 0., 0.], angular: [0., 1., 0.],
        }
    }

    #[test]
    fn shared_object_motion_interpolates_received_samples() {
        let samples = VecDeque::from([(1., state(1, 0.)), (1.1, state(2, 1.))]);
        let (position, rotation, velocity, _) = sample(&samples, 1.13).unwrap();
        assert!((position[0] - 0.5).abs() < 0.0001);
        assert_eq!(velocity, [10., 0., 0.]);
        assert!((Quat::from_array(rotation).length() - 1.).abs() < 0.0001);
    }

    #[test]
    fn missing_object_updates_bound_prediction_and_remove_stale_collision_samples() {
        let samples = VecDeque::from([(1., state(1, 0.))]);
        let near = sample(&samples, 1.3).unwrap();
        let late = sample(&samples, 1.9).unwrap();
        assert!((near.0[0] - 1.).abs() < 0.0001, "prediction must stop after100ms");
        assert_eq!(near, late, "packet loss cannot extrapolate indefinitely");
        assert!(sample(&samples, 2.001).is_none(), "expired snapshots supply neither visuals nor collision");
    }
}
