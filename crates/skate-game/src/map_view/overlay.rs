//! Project script-owned vector layers into the same viewport as the 3D map.
use super::{layers::MapLayerRegistry, view};
use bevy::prelude::*;
use skate_mods::map::MapItem;
use std::collections::{BTreeMap, BTreeSet};
type Key = (bool, String, String, String, usize);
#[derive(Resource, Default)]
struct LayerHud {
    nodes: BTreeMap<Key, Entity>,
}
struct Part {
    key: Key,
    position: Vec2,
    size: Vec2,
    angle: f32,
    color: Color,
    text: Option<String>,
}
fn clip_segment(a: Vec2, b: Vec2) -> Option<(Vec2, Vec2)> {
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let d = b - a;
    let mut lo = 0f32;
    let mut hi = 1f32;
    for (p, q) in [(-d.x, a.x), (d.x, 1. - a.x), (-d.y, a.y), (d.y, 1. - a.y)] {
        if p.abs() < 1e-8 {
            if q < 0. {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0. {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
            if lo > hi {
                return None;
            }
        }
    }
    Some((a + d * lo, a + d * hi))
}
pub(super) fn draw(world: &mut World, parent: Entity, frame: &view::ViewFrame, size: Vec2) {
    let mut parts = Vec::new();
    if let Some(registry) = world.get_resource::<MapLayerRegistry>() {
        for ((server, owner), entry) in &registry.entries {
            for layer in entry.snapshot.layers.iter().filter(|l| l.visible) {
                for item in &layer.items {
                    let key = (
                        *server,
                        owner.clone(),
                        layer.key.clone(),
                        item.key().to_owned(),
                        0,
                    );
                    let style = item.style();
                    let c = style.color;
                    let color = Color::srgba(c[0], c[1], c[2], c[3]);
                    match item {
                        MapItem::Marker {
                            position, label, ..
                        }
                        | MapItem::Label {
                            position,
                            text: label,
                            ..
                        } => {
                            if let Some(p) =
                                view::project(frame, Vec3::from_array(*position)).map(|p| p * size)
                            {
                                if matches!(item, MapItem::Marker { .. }) && owner=="engine-locations" && layer.key=="locations" {
                                    for (index,(a,b)) in [([-6.,0.],[0.,-6.]),([0.,-6.],[6.,0.]),([-4.,-1.],[-4.,6.]),([-4.,6.],[4.,6.]),([4.,6.],[4.,-1.])].into_iter().enumerate(){
                                        let a=p+Vec2::from_array(a);let b=p+Vec2::from_array(b);let delta=b-a;let mut line_key=key.clone();line_key.4=index+2;
                                        parts.push(Part{key:line_key,position:(a+b)*0.5-Vec2::new(delta.length(),2.)*0.5,size:Vec2::new(delta.length(),2.),angle:delta.y.atan2(delta.x),color,text:None});
                                    }
                                } else if matches!(item, MapItem::Marker { .. }) {
                                    parts.push(Part {
                                        key: key.clone(),
                                        position: p - Vec2::splat(style.size * 0.5),
                                        size: Vec2::splat(style.size),
                                        angle: 0.,
                                        color,
                                        text: None,
                                    });
                                }
                                if !label.is_empty() {
                                    let mut k = key;
                                    k.4 = 1;
                                    parts.push(Part {
                                        key: k,
                                        position: p + Vec2::new(8., -16.),
                                        size: Vec2::new(200., 20.),
                                        angle: 0.,
                                        color,
                                        text: Some(label.clone()),
                                    });
                                }
                            }
                        }
                        MapItem::Path { points, .. } | MapItem::Region { points, .. } => {
                            let edges = points.len() - 1
                                + usize::from(matches!(item, MapItem::Region { .. }));
                            for i in 0..edges {
                                let a = view::project_unclipped(frame, Vec3::from_array(points[i]));
                                let b = view::project_unclipped(
                                    frame,
                                    Vec3::from_array(points[(i + 1) % points.len()]),
                                );
                                if let Some((a, b)) = a.zip(b).and_then(|(a, b)| clip_segment(a, b))
                                {
                                    let a = a * size;
                                    let b = b * size;
                                    let delta = b - a;
                                    let length = delta.length();
                                    if length <= 0.01 {
                                        continue;
                                    }
                                    let mut k = key.clone();
                                    k.4 = i;
                                    let extent = Vec2::new(length, style.size);
                                    parts.push(Part {
                                        key: k,
                                        position: (a + b) * 0.5 - extent * 0.5,
                                        size: extent,
                                        angle: delta.y.atan2(delta.x),
                                        color,
                                        text: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Global UI ceiling complements per-owner schema limits; native player
    // markers have their own HUD and are never displaced by a script layer.
    parts.truncate(4096);
    let mut hud = world.remove_resource::<LayerHud>().unwrap_or_default();
    let desired: BTreeSet<_> = parts.iter().map(|p| p.key.clone()).collect();
    hud.nodes.retain(|key, e| {
        if desired.contains(key) && world.get_entity(*e).is_ok() {
            true
        } else {
            if world.get_entity(*e).is_ok() {
                world.despawn(*e);
            }
            false
        }
    });
    for part in parts {
        let e = *hud.nodes.entry(part.key).or_insert_with(|| {
            world
                .spawn((
                    Node::default(),
                    ChildOf(parent),
                    Pickable::IGNORE,
                    ZIndex(0),
                ))
                .id()
        });
        world.entity_mut(e).insert((
            Node {
                position_type: PositionType::Absolute,
                left: px(part.position.x),
                top: px(part.position.y),
                width: px(part.size.x),
                height: px(part.size.y),
                border_radius: if part.text.is_none() && part.size.x == part.size.y {
                    BorderRadius::MAX
                } else {
                    BorderRadius::ZERO
                },
                ..default()
            },
            UiTransform::from_rotation(Rot2::radians(part.angle)),
        ));
        if let Some(text) = part.text {
            world.entity_mut(e).insert((
                Text::new(text),
                TextFont {
                    font_size: 14.,
                    ..default()
                },
                TextColor(part.color),
                BackgroundColor(Color::srgba(0., 0., 0., 0.65)),
            ));
        } else {
            world.entity_mut(e).remove::<(Text, TextFont, TextColor)>();
            world.entity_mut(e).insert(BackgroundColor(part.color));
        }
    }
    world.insert_resource(hud);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_crossing_viewport_clip_without_missing_endpoints() {
        let (a, b) = clip_segment(Vec2::new(-1., 0.5), Vec2::new(2., 0.5)).unwrap();
        assert_eq!(a, Vec2::new(0., 0.5));
        assert_eq!(b, Vec2::new(1., 0.5));
        assert!(clip_segment(Vec2::new(-2., -1.), Vec2::new(-1., -1.)).is_none());
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn changing_item_kind_and_clearing_layers_removes_old_ui() {
        let mut world = World::new();
        world.init_resource::<MapLayerRegistry>();
        let parent = world.spawn(Node::default()).id();
        let frame = view::frame(
            super::super::geometry::OverviewBounds {
                min: Vec3::splat(-10.),
                max: Vec3::splat(10.),
            },
            Vec3::ZERO,
            false,
            4. / 3.,
            1.,
        );
        let label=skate_mods::map::MapSnapshot::parse(serde_json::json!({"layers":[{"key":"a","items":[{"kind":"label","key":"x","position":[0,0,0],"text":"Old label"}]}]})).unwrap();
        world
            .resource_mut::<MapLayerRegistry>()
            .set("owner", false, 0, 0, label)
            .unwrap();
        draw(&mut world, parent, &frame, Vec2::new(240., 180.));
        let key = (false, "owner".into(), "a".into(), "x".into(), 1);
        let e = world.resource::<LayerHud>().nodes[&key];
        assert!(world.get::<Text>(e).is_some());
        let path=skate_mods::map::MapSnapshot::parse(serde_json::json!({"layers":[{"key":"a","items":[{"kind":"path","key":"x","points":[[0,0,0],[1,0,0],[2,0,0]]}]}]})).unwrap();
        world
            .resource_mut::<MapLayerRegistry>()
            .set("owner", false, 0, 0, path)
            .unwrap();
        draw(&mut world, parent, &frame, Vec2::new(240., 180.));
        assert!(world.get::<Text>(e).is_none());
        world
            .resource_mut::<MapLayerRegistry>()
            .remove("owner", None);
        draw(&mut world, parent, &frame, Vec2::new(240., 180.));
        assert!(world.resource::<LayerHud>().nodes.is_empty());
        assert!(world.get_entity(e).is_err());
    }
}
