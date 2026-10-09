//! Expanded-map navigation in screen coordinates; never changes player transforms.
use super::{input::MapViewState, view::ViewFrame};
use bevy::{
    input::{
        gestures::*,
        mouse::{MouseScrollUnit, MouseWheel},
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    Plus,
    Minus,
    Compass,
    Mode,
    Recenter,
    Tilt,
}
impl Control {
    pub const ALL: [Self; 6] = [
        Self::Plus,
        Self::Minus,
        Self::Compass,
        Self::Mode,
        Self::Recenter,
        Self::Tilt,
    ];
    pub fn label(self, top_down: bool) -> &'static str {
        match self {
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Compass => "↑ N",
            Self::Mode => {
                if top_down {
                    "3D"
                } else {
                    "2D"
                }
            }
            Self::Recenter => "YOU",
            Self::Tilt => "",
        }
    }
}
pub(super) fn control_rect(viewport: Rect, control: Control) -> Rect {
    let index = match control {
        Control::Plus => 0,
        Control::Minus => 1,
        Control::Compass => 2,
        Control::Mode | Control::Tilt => 3,
        Control::Recenter => 4,
    };
    let x = viewport.max.x - 10. - 44. - index as f32 * 48.;
    if control == Control::Tilt {
        Rect::new(
            x + 13.,
            viewport.min.y + 50.,
            x + 31.,
            viewport.min.y + 150.,
        )
    } else {
        Rect::new(x, viewport.min.y + 10., x + 44., viewport.min.y + 42.)
    }
}
fn hit(viewport: Rect, p: Vec2) -> Option<Control> {
    Control::ALL
        .into_iter()
        .find(|c| control_rect(viewport, *c).contains(p))
}
#[derive(Resource, Default)]
pub(super) struct NavigationEvents {
    scroll: Vec2,
    wheel: f32,
    pinch: f32,
    rotation: f32,
    double_tap: bool,
}
pub(super) fn collect(
    mut events: ResMut<NavigationEvents>,
    mut wheels: MessageReader<MouseWheel>,
    mut pinch: MessageReader<PinchGesture>,
    mut rotation: MessageReader<RotationGesture>,
    mut pan: MessageReader<PanGesture>,
    mut tap: MessageReader<DoubleTapGesture>,
) {
    *events = NavigationEvents::default();
    for e in wheels.read() {
        match e.unit {
            MouseScrollUnit::Line => events.wheel += e.y,
            MouseScrollUnit::Pixel => events.scroll += Vec2::new(e.x, e.y),
        }
    }
    events.scroll += pan.read().map(|p| p.0).sum::<Vec2>();
    events.pinch = pinch.read().map(|p| p.0).sum();
    events.rotation = rotation.read().map(|p| p.0).sum::<f32>().to_radians();
    events.double_tap = tap.read().next().is_some();
}
#[derive(Clone, Copy)]
enum Drag {
    Map,
    Compass,
    Tilt,
}
#[derive(Resource, Default)]
pub(super) struct MapNavigation {
    pub viewport: Rect,
    pub frame: Option<ViewFrame>,
    pub generation: Option<u64>,
    drag: Option<Drag>,
    previous: Option<Vec2>,
    pressed: Option<Vec2>,
    last_click: Option<(f64, Vec2)>,
}
pub(super) fn release(world: &mut World) {
    let mut nav = world.resource_mut::<MapNavigation>();
    nav.drag = None;
    nav.previous = None;
    nav.pressed = None;
    nav.last_click = None;
}
pub(super) fn command_down(keys: &ButtonInput<KeyCode>) -> bool {
    if cfg!(target_os = "macos") {
        keys.pressed(KeyCode::SuperLeft) || keys.pressed(KeyCode::SuperRight)
    } else {
        keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)
    }
}
fn toggle_view(state: &mut MapViewState) {
    state.top_down = !state.top_down;
}
fn zoom_by(state: &mut MapViewState, amount: f32) {
    if !amount.is_finite() {
        return;
    }
    let steps = (f32::from(state.zoom_level) + state.zoom_fraction + amount).clamp(0., 12.);
    state.zoom_level = steps.floor() as u8;
    state.zoom_fraction = steps.fract();
}
fn pan_delta(frame: &ViewFrame, from: Vec2, to: Vec2, height: f32) -> Option<Vec3> {
    Some(frame.ground_point(from, height)? - frame.ground_point(to, height)?)
}
fn pan(state: &mut MapViewState, nav: &MapNavigation, delta: Vec2) {
    let Some(center) = state.center else {
        return;
    };
    let Some(frame) = &nav.frame else {
        return;
    };
    let size = nav.viewport.size();
    if size.min_element() < 1. {
        return;
    }
    if let Some(motion) = pan_delta(
        frame,
        Vec2::splat(0.5),
        Vec2::splat(0.5) + delta / size,
        center.y,
    ) {
        let mut next = center + motion;
        if let Some((min, max)) = state.bounds {
            let margin = (max - min) * 0.25;
            next.x = next.x.clamp(min.x - margin.x, max.x + margin.x);
            next.z = next.z.clamp(min.z - margin.z, max.z + margin.z);
        }
        state.center = Some(next);
    }
}
fn drag_pan(state: &mut MapViewState, nav: &MapNavigation, from: Vec2, to: Vec2) {
    let Some(center) = state.center else {
        return;
    };
    let Some(frame) = &nav.frame else {
        return;
    };
    if let Some(delta) = pan_delta(
        frame,
        (from - nav.viewport.min) / nav.viewport.size(),
        (to - nav.viewport.min) / nav.viewport.size(),
        center.y,
    ) {
        state.center = Some(center + delta);
    }
}
fn tilt(state: &mut MapViewState, nav: &MapNavigation, p: Vec2) {
    let r = control_rect(nav.viewport, Control::Tilt);
    let fraction = ((p.y - r.min.y) / r.height()).clamp(0., 1.);
    let angle = 90. - fraction * 65.;
    state.top_down = angle >= 89.9;
    if !state.top_down {
        state.pitch = angle.to_radians();
    }
}
fn player_position(world: &mut World) -> Option<Vec3> {
    let entity = world
        .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
        .iter(world)
        .next()?;
    super::hud::position(world, entity)
}
pub(super) fn update(world: &mut World, state: &mut MapViewState) {
    let mut nav = world.remove_resource::<MapNavigation>().unwrap_or_default();
    if nav.generation != state.generation || nav.frame.is_none() {
        nav.drag = None;
        nav.last_click = None;
        nav.previous = None;
        world.insert_resource(nav);
        return;
    }
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
    let command = command_down(&keys);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let dt = world
        .get_resource::<Time>()
        .map_or(0., |t| t.delta_secs().min(0.1));
    let now = world
        .get_resource::<Time>()
        .map_or(0., |t| t.elapsed_secs_f64());
    if command && keys.just_pressed(KeyCode::KeyD) {
        toggle_view(state);
    }
    if command && keys.just_pressed(KeyCode::KeyL) {
        state.center = player_position(world).or(state.center);
    }
    if command
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
        && keys.just_pressed(KeyCode::ArrowUp)
    {
        state.yaw = 0.;
    }
    if command && (keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd)) {
        zoom_by(state, 1.);
    }
    if command && (keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract))
    {
        zoom_by(state, -1.);
    }
    let horizontal = keys.pressed(KeyCode::ArrowRight) as u8 as f32
        - keys.pressed(KeyCode::ArrowLeft) as u8 as f32;
    let vertical =
        keys.pressed(KeyCode::ArrowDown) as u8 as f32 - keys.pressed(KeyCode::ArrowUp) as u8 as f32;
    if alt {
        state.yaw = (state.yaw + horizontal * dt * 1.2).rem_euclid(std::f32::consts::TAU);
    } else if !command {
        pan(state, &nav, -Vec2::new(horizontal, vertical) * dt * 320.);
    }
    let ui_scale = world
        .get_resource::<UiScale>()
        .map_or(1., |s| s.0)
        .max(0.01);
    let (cursor, window_scale) = world
        .query::<&Window>()
        .iter(world)
        .find(|w| w.focused)
        .map(|w| {
            (
                w.cursor_position().map(|p| p / ui_scale),
                w.scale_factor().max(0.01),
            )
        })
        .unwrap_or((None, 1.));
    let buttons = world.resource::<ButtonInput<MouseButton>>();
    let (pressed, held, released) = (
        buttons.just_pressed(MouseButton::Left),
        buttons.pressed(MouseButton::Left),
        buttons.just_released(MouseButton::Left),
    );
    let events = std::mem::take(&mut *world.resource_mut::<NavigationEvents>());
    if let Some(p) = cursor {
        if nav.viewport.contains(p) {
            if events.scroll.is_finite() {
                pan(state, &nav, events.scroll / (window_scale * ui_scale));
            }
            zoom_by(state, events.wheel + events.pinch * 8.);
            if events.rotation.is_finite() {
                state.yaw = (state.yaw - events.rotation).rem_euclid(std::f32::consts::TAU);
            }
            if events.double_tap {
                zoom_by(state, if alt { -1. } else { 1. });
            }
        }
        if pressed && nav.viewport.contains(p) {
            nav.pressed = Some(p);
            match hit(nav.viewport, p) {
                Some(Control::Plus) => zoom_by(state, 1.),
                Some(Control::Minus) => zoom_by(state, -1.),
                Some(Control::Mode) => toggle_view(state),
                Some(Control::Recenter) => state.center = player_position(world),
                Some(Control::Compass) => nav.drag = Some(Drag::Compass),
                Some(Control::Tilt) => {
                    nav.drag = Some(Drag::Tilt);
                    tilt(state, &nav, p);
                }
                None => {
                    let double = nav
                        .last_click
                        .is_some_and(|(time, point)| now - time <= 0.35 && point.distance(p) < 6.);
                    if double {
                        zoom_by(state, if alt { -1. } else { 1. });
                        nav.last_click = None;
                    } else {
                        nav.last_click = Some((now, p));
                    }
                    nav.drag = Some(Drag::Map);
                }
            }
            nav.previous = Some(p);
        } else if held {
            if let Some(previous) = nav.previous {
                match nav.drag {
                    Some(Drag::Map) => drag_pan(state, &nav, previous, p),
                    Some(Drag::Compass) => {
                        state.yaw = (state.yaw + ((p.x - previous.x) - (p.y - previous.y)) * 0.006)
                            .rem_euclid(std::f32::consts::TAU)
                    }
                    Some(Drag::Tilt) => tilt(state, &nav, p),
                    None => {}
                }
            }
            nav.previous = Some(p);
            if nav.pressed.is_some_and(|start| start.distance(p) > 6.) {
                nav.last_click = None;
            }
        }
        if released
            && matches!(nav.drag, Some(Drag::Compass))
            && nav.pressed.is_some_and(|start| start.distance(p) < 3.)
        {
            state.yaw = 0.;
        }
    }
    if cursor.is_none() {
        nav.previous = None;
    }
    if !held {
        nav.drag = None;
        nav.previous = None;
        nav.pressed = None;
    }
    world.insert_resource(nav);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_view::{geometry::OverviewBounds, input::MapViewState, view};
    #[test]
    fn opening_map_is_visible_to_gameplay_before_fixed_ticks() {
        use bevy::input::{gestures::*, mouse::MouseWheel};
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<MouseWheel>()
            .add_message::<PinchGesture>()
            .add_message::<RotationGesture>()
            .add_message::<PanGesture>()
            .add_message::<DoubleTapGesture>()
            .add_plugins(super::super::MapViewPlugin);
        app.world_mut().spawn(Window {
            focused: true,
            ..default()
        });
        app.insert_resource(crate::map_transition::CurrentMap {
            path: None,
            name: "test".into(),
            spawn: [0.; 3],
            heading: 0.,
            generation: 0,
            audio_tag: None,
        });
        app.world_mut().resource_mut::<MapViewState>().generation = Some(0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.world_mut().run_schedule(PreUpdate);
        let state = app.world().resource::<MapViewState>();
        assert!(
            state.expanded && state.visible,
            "opening must block gameplay in this frame's fixed ticks"
        );
    }
    fn scene() -> (World, MapViewState) {
        let bounds = OverviewBounds {
            min: Vec3::splat(-500.),
            max: Vec3::splat(500.),
        };
        let mut world = World::new();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<Time>();
        world.init_resource::<NavigationEvents>();
        let mut window = Window::default();
        window.focused = true;
        window.set_cursor_position(Some(Vec2::new(300., 300.)));
        world.spawn(window);
        world.insert_resource(MapNavigation {
            viewport: Rect::new(0., 0., 800., 600.),
            frame: Some(view::frame_free(
                bounds,
                Vec3::ZERO,
                4. / 3.,
                4.,
                0.,
                std::f32::consts::FRAC_PI_2,
            )),
            generation: Some(1),
            ..default()
        });
        (
            world,
            MapViewState {
                expanded: true,
                generation: Some(1),
                center: Some(Vec3::ZERO),
                bounds: Some((bounds.min, bounds.max)),
                ..default()
            },
        )
    }
    #[test]
    fn pixel_scroll_pan_matches_logical_drag_at_high_dpi() {
        let (mut a, mut sa) = scene();
        let (mut b, mut sb) = scene();
        let window = b
            .query_filtered::<Entity, With<Window>>()
            .single(&b)
            .unwrap();
        let mut w = b.get_mut::<Window>(window).unwrap();
        w.resolution.set_scale_factor_override(Some(2.));
        w.set_cursor_position(Some(Vec2::new(300., 300.)));
        a.resource_mut::<NavigationEvents>().scroll = Vec2::new(20., 10.);
        b.resource_mut::<NavigationEvents>().scroll = Vec2::new(40., 20.);
        update(&mut a, &mut sa);
        update(&mut b, &mut sb);
        assert!(sa.center.unwrap().distance(sb.center.unwrap()) < 0.001);
    }
    #[test]
    fn positive_trackpad_rotation_turns_map_counterclockwise() {
        let (mut world, mut state) = scene();
        state.top_down = true;
        world.resource_mut::<NavigationEvents>().rotation = 0.2;
        update(&mut world, &mut state);
        let b = OverviewBounds {
            min: Vec3::splat(-500.),
            max: Vec3::splat(500.),
        };
        let f = view::frame_free(
            b,
            Vec3::ZERO,
            4. / 3.,
            4.,
            state.yaw,
            std::f32::consts::FRAC_PI_2,
        );
        assert!(view::project(&f, Vec3::NEG_Z * 10.).unwrap().x < 0.5);
    }
    #[test]
    fn apple_shortcuts_and_pixel_pan_keep_zoom_and_center_independent() {
        let (mut world, mut state) = scene();
        let command = if cfg!(target_os = "macos") {
            KeyCode::SuperLeft
        } else {
            KeyCode::ControlLeft
        };
        world.resource_mut::<ButtonInput<KeyCode>>().press(command);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyD);
        update(&mut world, &mut state);
        assert!(state.top_down);
        assert_eq!(state.center, Some(Vec3::ZERO));
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset(KeyCode::KeyD);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Equal);
        update(&mut world, &mut state);
        assert_eq!(state.zoom_level, 1);
        world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
        world.resource_mut::<NavigationEvents>().scroll = Vec2::new(20., 10.);
        update(&mut world, &mut state);
        assert_ne!(state.center, Some(Vec3::ZERO));
        assert_eq!(state.zoom_level, 1);
        world.spawn((crate::world::PlayerRoot, Transform::from_xyz(30., 40., 50.)));
        world.resource_mut::<ButtonInput<KeyCode>>().press(command);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyL);
        update(&mut world, &mut state);
        assert_eq!(state.center, Some(Vec3::new(30., 40., 50.)));
    }
    #[test]
    fn toolbar_mode_click_and_tilt_use_same_hit_regions_as_display() {
        let (mut world, mut state) = scene();
        let viewport = world.resource::<MapNavigation>().viewport;
        let window = world
            .query_filtered::<Entity, With<Window>>()
            .single(&world)
            .unwrap();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(control_rect(viewport, Control::Mode).center()));
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        update(&mut world, &mut state);
        assert!(state.top_down);
        assert_eq!(state.center, Some(Vec3::ZERO));
        world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
        let rect = control_rect(viewport, Control::Tilt);
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(rect.center()));
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        update(&mut world, &mut state);
        assert!(!state.top_down);
        assert!((state.pitch.to_degrees() - 57.5).abs() < 0.001);
    }
    #[test]
    fn pan_grabs_the_ground_at_every_heading_and_tilt() {
        let bounds = OverviewBounds {
            min: Vec3::splat(-500.),
            max: Vec3::splat(500.),
        };
        for yaw in [0., 1., 3.] {
            for pitch in [30_f32.to_radians(), std::f32::consts::FRAC_PI_2] {
                let frame = view::frame_free(bounds, Vec3::ZERO, 4. / 3., 4., yaw, pitch);
                let from = Vec2::new(0.4, 0.4);
                let to = Vec2::new(0.55, 0.6);
                let delta = pan_delta(&frame, from, to, 0.).unwrap();
                let next = view::frame_free(bounds, delta, 4. / 3., 4., yaw, pitch);
                let anchor = frame.ground_point(from, 0.).unwrap();
                assert!(view::project(&next, anchor).unwrap().distance(to) < 0.001);
            }
        }
    }
    #[test]
    fn view_switch_preserves_free_center_zoom_and_last_tilt() {
        let mut s = MapViewState::default();
        s.center = Some(Vec3::new(10., 20., 30.));
        s.zoom_level = 5;
        let pitch = s.pitch;
        toggle_view(&mut s);
        assert!(s.top_down);
        toggle_view(&mut s);
        assert!(!s.top_down);
        assert_eq!(s.pitch, pitch);
        assert_eq!(s.center, Some(Vec3::new(10., 20., 30.)));
        assert_eq!(s.zoom_level, 5);
    }
    #[test]
    fn toolbar_hit_regions_do_not_start_pan() {
        let r = Rect::new(100., 100., 900., 700.);
        assert_eq!(
            hit(r, control_rect(r, Control::Mode).center()),
            Some(Control::Mode)
        );
        assert_eq!(hit(r, Vec2::new(300., 300.)), None);
    }
}
