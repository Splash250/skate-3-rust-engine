//! Native resource browser ownership and lifecycle. The browser receives only
//! packaged web assets and JSON; every engine command still comes from its VM.
use super::Mods;
use bevy::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use skate_browser::{Event, Init, Input, Options, Surface, SurfaceInput, process::Page};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_FOCUS_TOKEN: AtomicU64 = AtomicU64::new(2);
type Slot = (String, String);
pub(super) struct Owned {
    page: Page,
    ready: bool,
    focused: bool,
    surface: Option<Surface>,
    size: Vec2,
    image: Option<Handle<Image>>,
    entity: Option<Entity>,
    just_opened: bool,
    focus_token: u64,
}
pub(super) type Pages = BTreeMap<Slot, Owned>;

pub(super) fn focused(mods: &Mods) -> bool {
    mods.browsers.values().any(|p| p.focused)
}
pub(super) fn composited_focused(mods: &Mods) -> bool {
    mods.browsers
        .values()
        .any(|p| p.focused && p.surface.is_some())
}
pub(super) fn focus_token(mods: &Mods) -> u64 {
    mods.browsers
        .values()
        .find(|page| page.focused)
        .map_or(0, |page| page.focus_token)
}
pub(super) fn focused_owner(mods: &Mods) -> Option<&str> {
    mods.browsers
        .iter()
        .find(|(_, p)| p.focused && p.surface.is_some())
        .map(|((owner, _), _)| owner.as_str())
}
fn restore_input(world: &mut World) {
    if let Some(mut keys) = world.get_resource_mut::<ButtonInput<KeyCode>>() {
        keys.clear();
    }
    if let Some(mut buttons) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
        buttons.clear();
    }
    if let Some(mut nav) = world.get_resource_mut::<crate::customiser::Navigation>() {
        nav.pressed = 0;
    }
    if let Some(mut controller) = world.get_resource_mut::<crate::input::ControllerInput>() {
        controller.discard_gameplay();
    }
}
pub(super) fn open(
    world: &mut World,
    mods: &mut Mods,
    owner: &str,
    key: String,
    options: Options,
) -> Result<(), String> {
    let slot = (owner.to_owned(), key);
    if mods.browsers.contains_key(&slot) {
        return Err("browser page already open; close it before replacing".into());
    }
    if mods.browsers.len() >= 4 || mods.browsers.keys().filter(|(id, _)| id == owner).count() >= 2 {
        return Err("browser page limit: 2 per resource, 4 total".into());
    }
    if options.focus && focused(mods) {
        return Err("another browser page owns input focus".into());
    }
    let root = mods
        .manager
        .packages
        .get(owner)
        .ok_or("browser owner is not installed")?
        .root
        .clone();
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name(if cfg!(windows) {
            "skate-browser-host.exe"
        } else {
            "skate-browser-host"
        });
    let focused = options.focus;
    let surface = options.surface.clone();
    let size = Vec2::new(options.width as f32, options.height as f32);
    let page = Page::open(
        &executable,
        Init {
            root,
            title: format!("{owner} — {}", slot.1),
            options,
        },
    )?;
    info!(
        "RESOURCE_BROWSER_OPEN resource={owner} key={} pid={}",
        slot.1,
        page.id()
    );
    if focused {
        restore_input(world);
    }
    mods.browsers.insert(
        slot,
        Owned {
            page,
            ready: false,
            focused,
            surface,
            size,
            image: None,
            entity: None,
            just_opened: true,
            focus_token: NEXT_FOCUS_TOKEN.fetch_add(1, Ordering::Relaxed),
        },
    );
    Ok(())
}
pub(super) fn send(
    pages: &Pages,
    owner: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    // Host Escape can close a page before maintenance applies an accepted VM
    // update. Stale presentation work is inert, not a resource lifecycle error.
    let Some(page) = pages.get(&(owner.to_owned(), key.to_owned())) else {
        return Ok(());
    };
    if !page.ready {
        return Err("browser page is not ready; await its ready event".into());
    }
    page.page.send(&Input::Message { value })
}
pub(super) fn focus(
    world: &mut World,
    pages: &mut Pages,
    owner: &str,
    key: &str,
    focused: bool,
) -> Result<(), String> {
    let slot = (owner.to_owned(), key.to_owned());
    // Check retirement before focus conflicts: an absent page cannot compete
    // with a replacement interface, restore input, or recreate itself.
    if !pages.contains_key(&slot) {
        return Ok(());
    }
    if focused && pages.iter().any(|(id, page)| id != &slot && page.focused) {
        return Err("another browser page owns input focus".into());
    }
    let page = pages.get_mut(&slot).ok_or("browser page is not open")?;
    page.page.set_focus(focused)?;
    let was = page.focused;
    page.focused = focused;
    if focused && !was {
        page.focus_token = NEXT_FOCUS_TOKEN.fetch_add(1, Ordering::Relaxed);
    }
    page.just_opened = focused;
    if was && !focused {
        if super::photos::owner(world) == Some(owner) {
            super::photos::leave(world);
        }
        restore_input(world);
    }
    Ok(())
}
pub(super) fn close(world: &mut World, mods: &mut Mods, owner: &str, key: &str) {
    if let Some(page) = mods.browsers.remove(&(owner.to_owned(), key.to_owned())) {
        if super::photos::owner(world) == Some(owner) {
            super::photos::leave(world);
        }
        let focused = page.focused;
        if let Some(entity) = page.entity {
            let _ = world.despawn(entity);
        }
        if let (Some(image), Some(mut images)) = (
            page.image.as_ref(),
            world.get_resource_mut::<Assets<Image>>(),
        ) {
            images.remove(image.id());
        }
        drop(page);
        info!("RESOURCE_BROWSER_CLOSED resource={owner} key={key}");
        if focused {
            restore_input(world);
        }
    }
}
pub(super) fn clear(world: &mut World, mods: &mut Mods, owner: Option<&str>) {
    let keys: Vec<_> = mods
        .browsers
        .keys()
        .filter(|(id, _)| owner.is_none_or(|owner| owner == id))
        .cloned()
        .collect();
    for (id, key) in keys {
        close(world, mods, &id, &key);
    }
}
#[derive(Resource)]
struct FailureNotice {
    entity: Entity,
    expires: std::time::Instant,
}
fn failure_notice(world: &mut World, owner: &str, message: &str) {
    if let Some(old) = world.remove_resource::<FailureNotice>() {
        let _ = world.despawn(old.entity);
    }
    let text = format!(
        "Interface closed · {}\n{}\nRelease held controls to resume. Reopen after the resource restarts.",
        owner.chars().take(64).collect::<String>(),
        message.chars().take(180).collect::<String>()
    );
    let entity = world
        .spawn((
            Name::new("Resource browser failure notice"),
            GlobalZIndex(80),
            Node {
                position_type: PositionType::Absolute,
                left: px(24),
                top: px(24),
                max_width: px(600),
                padding: UiRect::all(px(16)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.16, 0.035, 0.035, 0.96)),
            bevy::ui::FocusPolicy::Pass,
        ))
        .with_child((
            Text::new(text),
            TextFont {
                font_size: 16.,
                ..default()
            },
            TextColor(Color::WHITE),
        ))
        .id();
    world.insert_resource(FailureNotice {
        entity,
        expires: std::time::Instant::now() + std::time::Duration::from_secs(10),
    });
}
pub(super) fn poll(world: &mut World, mods: &mut Mods) {
    if world
        .get_resource::<FailureNotice>()
        .is_some_and(|n| std::time::Instant::now() >= n.expires)
    {
        if let Some(notice) = world.remove_resource::<FailureNotice>() {
            let _ = world.despawn(notice.entity);
        }
    }
    let mut events = Vec::new();
    let had_focus = focused(mods);
    for ((owner, key), page) in &mut mods.browsers {
        for event in page.page.poll() {
            match &event {
                Event::Ready => {
                    page.ready = true;
                    info!("RESOURCE_BROWSER_READY resource={owner} key={key}");
                }
                Event::Focus { focused } => page.focused = *focused,
                _ => {}
            }
            events.push((owner.clone(), key.clone(), event));
        }
        if let Some(frame) = page.page.take_frame() {
            let image = Image::new(
                Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                frame.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            if let Some(handle) = &page.image {
                if let Some(target) = world.resource_mut::<Assets<Image>>().get_mut(handle) {
                    *target = image;
                }
            } else {
                let handle = world.resource_mut::<Assets<Image>>().add(image);
                let entity = world
                    .spawn((
                        Name::new("Resource browser surface"),
                        ImageNode::new(handle.clone()),
                        Node::default(),
                        GlobalZIndex(30),
                        bevy::ui::FocusPolicy::Block,
                    ))
                    .id();
                page.image = Some(handle);
                page.entity = Some(entity);
            }
        }
        if let (Some(surface), Some(entity)) = (&page.surface, page.entity) {
            let (position, size) = layout(world, surface, page.size);
            let hidden = !page.focused
                || world
                    .get_resource::<crate::graphics_menu::Menu>()
                    .is_some_and(|m| m.open)
                || super::photos::active(world);
            if let Some(mut node) = world.get_mut::<Node>(entity) {
                *node = Node {
                    position_type: PositionType::Absolute,
                    left: px(position.x),
                    top: px(position.y),
                    width: px(size.x),
                    height: px(size.y),
                    display: if hidden { Display::None } else { Display::Flex },
                    ..default()
                };
            }
        }
    }
    if had_focus && !focused(mods) {
        restore_input(world);
    }
    for (owner, key, event) in events {
        if !mods.browsers.contains_key(&(owner.clone(), key.clone())) {
            continue;
        }
        if let Event::Error { message } = &event {
            clear(world, mods, Some(&owner));
            failure_notice(world, &owner, message);
            mods.manager
                .fail(&owner, format!("browser {key}: {message}"));
            continue;
        }
        if matches!(event, Event::Closed) {
            close(world, mods, &owner, &key);
        }
        mods.manager.call(
            &owner,
            "on_event",
            serde_json::json!({"type":"browser","key":key,"event":event}),
        );
    }
}

fn layout(world: &mut World, surface: &Surface, size: Vec2) -> (Vec2, Vec2) {
    let viewport = world
        .query::<&Window>()
        .iter(world)
        .next()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(Vec2::new(1280., 720.));
    let ui_scale = world
        .get_resource::<UiScale>()
        .map_or(1., |s| s.0)
        .max(0.01);
    let view = viewport / ui_scale;
    let offset = Vec2::from_array(surface.offset);
    let scale = surface
        .scale
        .min(((view - offset * 2.) / size).min_element())
        .max(0.05);
    let size = size * scale;
    let position = match surface.anchor {
        skate_browser::Anchor::BottomRight => view - size - offset,
        skate_browser::Anchor::Center => (view - size) * 0.5,
    };
    (position, size)
}
#[derive(Resource, Default)]
struct InputState {
    keyboard: bevy::ecs::message::MessageCursor<bevy::input::keyboard::KeyboardInput>,
    wheel: bevy::ecs::message::MessageCursor<bevy::input::mouse::MouseWheel>,
    pointer: Option<Vec2>,
    pad_previous: u16,
    pad_held: f32,
    pad_repeat: f32,
    slot: Option<Slot>,
    focus_token: u64,
    pad_latched: bool,
    keyboard_latched: bool,
    pointer_latched: bool,
}
/// Host-only input routing, scheduled after Navigation and before stock menus.
/// Escape/Start always retires the surface even if its JavaScript is unresponsive.
pub(super) fn input(world: &mut World) {
    let mut state = world.remove_resource::<InputState>().unwrap_or_default();
    let keyboard: Vec<_> = world
        .get_resource::<Messages<bevy::input::keyboard::KeyboardInput>>()
        .map(|messages| state.keyboard.read(messages).cloned().collect())
        .unwrap_or_default();
    let wheel: f32 = world
        .get_resource::<Messages<bevy::input::mouse::MouseWheel>>()
        .map(|messages| {
            state
                .wheel
                .read(messages)
                .map(|e| {
                    e.y * if e.unit == bevy::input::mouse::MouseScrollUnit::Line {
                        36.
                    } else {
                        1.
                    }
                })
                .sum()
        })
        .unwrap_or_default();
    let raw = world
        .get_resource::<crate::input::ControllerFrame>()
        .map_or_else(crate::input::RawInput::default, |c| c.raw_input());
    let dt = world
        .get_resource::<Time<Real>>()
        .map_or(0.016, |t| t.delta_secs().min(0.1));
    let mut nav = controller_edges(&mut state, raw, dt);
    let window_focused = world.query::<&Window>().iter(world).any(|w| w.focused);
    if !window_focused
        || super::photos::active(world)
        || world
            .get_resource::<crate::graphics_menu::Menu>()
            .is_some_and(|m| m.open)
    {
        state.slot = None;
        world.insert_resource(state);
        return;
    }
    let Some(mut mods) = world.remove_resource::<Mods>() else {
        world.insert_resource(state);
        return;
    };
    let slot = mods
        .browsers
        .iter()
        .find(|(_, p)| p.focused && p.surface.is_some())
        .map(|(key, _)| key.clone());
    if let Some(slot) = slot {
        let token = mods.browsers[&slot].focus_token;
        if state.slot.as_ref() != Some(&slot) || state.focus_token != token {
            state.slot = Some(slot.clone());
            state.focus_token = token;
            state.pad_latched = true;
            state.keyboard_latched = true;
            state.pointer_latched = true;
        }
        if state.pad_latched {
            if raw.buttons == 0 && raw.left.iter().all(|v| v.abs() < 0.25) {
                state.pad_latched = false;
            }
            nav = 0;
        }
        let keyboard_allowed = world
            .get_resource::<ButtonInput<KeyCode>>()
            .is_none_or(|keys| keyboard_ready(&mut state, keys));
        let pointer_down = world
            .get_resource::<ButtonInput<MouseButton>>()
            .is_some_and(|buttons| buttons.pressed(MouseButton::Left));
        if !pointer_down {
            state.pointer_latched = false;
        }
        let escape = world
            .get_resource::<ButtonInput<KeyCode>>()
            .is_some_and(|k| k.just_pressed(KeyCode::Escape))
            || nav & 0x10 != 0;
        if escape {
            close(world, &mut mods, &slot.0, &slot.1);
            mods.manager.call(
                &slot.0,
                "on_event",
                serde_json::json!({"type":"browser","key":slot.1,"event":{"kind":"closed"}}),
            );
        } else {
            let page = mods.browsers.get_mut(&slot).unwrap();
            if !page.just_opened && page.ready {
                let mut inputs = Vec::new();
                let shift = world
                    .get_resource::<ButtonInput<KeyCode>>()
                    .is_some_and(|k| {
                        k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight)
                    });
                let control = world
                    .get_resource::<ButtonInput<KeyCode>>()
                    .is_some_and(|k| {
                        k.pressed(KeyCode::ControlLeft) || k.pressed(KeyCode::ControlRight)
                    });
                for event in keyboard.into_iter().filter(|e| {
                    keyboard_allowed
                        && e.state == bevy::input::ButtonState::Pressed
                        && activation_allowed(e.key_code, e.repeat)
                }) {
                    if control && matches!(event.key_code, KeyCode::KeyM | KeyCode::KeyD) {
                        continue;
                    }
                    if control && event.key_code == KeyCode::KeyA {
                        inputs.push(SurfaceInput::Key {
                            key: "SelectAll".into(),
                            shift: false,
                        });
                        continue;
                    }
                    use bevy::input::keyboard::Key;
                    let key = match &event.logical_key {
                        Key::Tab => Some("Tab"),
                        Key::Enter => Some("Enter"),
                        Key::Backspace => Some("Backspace"),
                        Key::Delete => Some("Delete"),
                        Key::ArrowUp => Some("ArrowUp"),
                        Key::ArrowDown => Some("ArrowDown"),
                        Key::ArrowLeft => Some("ArrowLeft"),
                        Key::ArrowRight => Some("ArrowRight"),
                        Key::Home => Some("Home"),
                        Key::End => Some("End"),
                        _ => None,
                    };
                    if let Some(key) = key {
                        inputs.push(SurfaceInput::Key {
                            key: key.into(),
                            shift,
                        });
                    } else if let Some(text) = event.text {
                        let text: String =
                            text.chars().filter(|c| !c.is_control()).take(64).collect();
                        if !text.is_empty() {
                            inputs.push(SurfaceInput::Text { text });
                        }
                    }
                }
                for (mask, direction) in [
                    (1, "up"),
                    (2, "down"),
                    (4, "left"),
                    (8, "right"),
                    (0x1000, "accept"),
                    (0x2000, "back"),
                ] {
                    if nav & mask != 0 {
                        inputs.push(SurfaceInput::Navigate {
                            direction: direction.into(),
                        });
                    }
                }
                let pointer = world
                    .query::<&Window>()
                    .iter(world)
                    .next()
                    .and_then(Window::cursor_position);
                let click = !state.pointer_latched
                    && world
                        .get_resource::<ButtonInput<MouseButton>>()
                        .is_some_and(|m| m.just_pressed(MouseButton::Left));
                if let Some(pointer) = pointer {
                    if state.pointer != Some(pointer) || click {
                        let (position, size) =
                            layout(world, page.surface.as_ref().unwrap(), page.size);
                        let scale = world
                            .get_resource::<UiScale>()
                            .map_or(1., |s| s.0)
                            .max(0.01);
                        let point = (pointer / scale - position) / size * page.size;
                        if point.cmpge(Vec2::ZERO).all() && point.cmplt(page.size).all() {
                            inputs.push(SurfaceInput::Pointer {
                                x: point.x,
                                y: point.y,
                                click,
                            });
                        }
                    }
                }
                state.pointer = pointer;
                if wheel != 0. {
                    inputs.push(SurfaceInput::Wheel {
                        delta: (-wheel).clamp(-2000., 2000.),
                    });
                }
                for input in inputs.into_iter().take(32) {
                    if input.validate() {
                        let _ = page.page.send(&Input::SurfaceInput { input });
                    }
                }
            }
            page.just_opened = false;
            // Keep physical held keys intact for trusted PTT, but consume edges
            // so gameplay and stock menu handlers cannot act on the same input.
            if let Some(mut keys) = world.get_resource_mut::<ButtonInput<KeyCode>>() {
                consume_key_edges(&mut keys);
            }
            if let Some(mut buttons) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
                buttons.clear();
            }
            if let Some(mut nav) = world.get_resource_mut::<crate::customiser::Navigation>() {
                nav.pressed = 0;
            }
        }
    }
    if !composited_focused(&mods) {
        state.slot = None;
    }
    world.insert_resource(mods);
    world.insert_resource(state);
}

// Activation is one physical press even when a route changes inside the page.
// Text edits and navigation may repeat, but held Enter cannot accept a new dialog.
fn activation_allowed(key: KeyCode, repeat: bool) -> bool {
    !repeat || !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter)
}
fn keyboard_ready(state: &mut InputState, keys: &ButtonInput<KeyCode>) -> bool {
    if state.keyboard_latched && keys.get_pressed().next().is_none() {
        state.keyboard_latched = false;
    }
    !state.keyboard_latched
}
fn controller_edges(state: &mut InputState, raw: crate::input::RawInput, dt: f32) -> u16 {
    let current = raw.buttons
        | if raw.left[1] > 0.5 {
            1
        } else if raw.left[1] < -0.5 {
            2
        } else {
            0
        }
        | if raw.left[0] > 0.5 {
            8
        } else if raw.left[0] < -0.5 {
            4
        } else {
            0
        };
    let mut pressed = current & !state.pad_previous;
    if current & 15 != 0 && current & 15 == state.pad_previous & 15 {
        state.pad_held += dt;
        if state.pad_held >= state.pad_repeat {
            pressed |= current & 15;
            state.pad_repeat += 0.09;
        }
    } else {
        state.pad_held = 0.;
        state.pad_repeat = 0.35;
    }
    state.pad_previous = current;
    pressed
}
pub(super) fn consume_key_edges(keys: &mut ButtonInput<KeyCode>) {
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let pressed: Vec<_> = keys.get_just_pressed().copied().collect();
    for key in pressed {
        if !control || !matches!(key, KeyCode::KeyM | KeyCode::KeyD) {
            keys.clear_just_pressed(key);
        }
    }
    let released: Vec<_> = keys.get_just_released().copied().collect();
    for key in released {
        keys.clear_just_released(key);
    }
}
#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn queued_presentation_after_host_close_is_inert() {
        // Escape retires the page before maintenance applies commands queued by
        // the last VM callback. They must not retire the still-running resource.
        let mut pages = Pages::new();
        let mut world = World::new();
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyV);
        world.insert_resource(keys);
        for _ in 0..3 {
            assert!(send(&pages, "plugin", "page", serde_json::json!({"late":true})).is_ok());
            assert!(focus(&mut world, &mut pages, "plugin", "page", true).is_ok());
            assert!(focus(&mut world, &mut pages, "plugin", "page", false).is_ok());
        }
        assert!(pages.is_empty());
        assert_eq!(world.entities().len(), 0);
        let keys = world.resource::<ButtonInput<KeyCode>>();
        assert!(keys.pressed(KeyCode::KeyV));
        assert!(keys.just_pressed(KeyCode::KeyV)); // stale focus must not restore input
    }
    #[test]
    fn held_activation_does_not_click_a_new_route_but_text_and_navigation_repeat() {
        for key in [KeyCode::Enter, KeyCode::NumpadEnter] {
            assert!(activation_allowed(key, false));
            assert!(!activation_allowed(key, true));
        }
        for key in [KeyCode::KeyA, KeyCode::ArrowDown, KeyCode::Backspace] {
            assert!(activation_allowed(key, true));
        }
    }
    #[test]
    fn held_enter_and_repeat_cannot_activate_a_new_owner_before_physical_release() {
        let mut state = InputState {
            keyboard_latched: true,
            ..default()
        };
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Enter);
        assert!(!keyboard_ready(&mut state, &keys));
        keys.clear();
        keys.press(KeyCode::Enter); // OS autorepeat does not release the held key.
        assert!(!keyboard_ready(&mut state, &keys));
        keys.release(KeyCode::Enter);
        assert!(keyboard_ready(&mut state, &keys));
        keys.press(KeyCode::Enter);
        assert!(keyboard_ready(&mut state, &keys));
        assert!(keys.just_pressed(KeyCode::Enter));
    }
    #[test]
    fn closing_surface_preserves_held_physical_ptt_until_actual_release() {
        let mut world = World::new();
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyV);
        world.insert_resource(keys);
        restore_input(&mut world);
        let keys = world.resource::<ButtonInput<KeyCode>>();
        assert!(keys.pressed(KeyCode::KeyV));
        assert!(!keys.just_pressed(KeyCode::KeyV));
    }
    #[test]
    fn controller_navigation_uses_selected_raw_pad_once_with_bounded_repeat() {
        let mut state = InputState::default();
        let mut raw = crate::input::RawInput::default();
        raw.buttons = 0x1000;
        assert_eq!(controller_edges(&mut state, raw, 0.016), 0x1000);
        assert_eq!(controller_edges(&mut state, raw, 0.1), 0);
        raw.buttons = 0;
        raw.left = [0., 1.];
        assert_eq!(controller_edges(&mut state, raw, 0.016), 1);
        for _ in 0..3 {
            assert_eq!(controller_edges(&mut state, raw, 0.1), 0);
        }
        assert_eq!(controller_edges(&mut state, raw, 0.1), 1);
    }
    #[test]
    fn ui_consumes_menu_edges_but_preserves_trusted_local_voice_shortcuts() {
        let mut keys = ButtonInput::default();
        for key in [
            KeyCode::ControlLeft,
            KeyCode::KeyM,
            KeyCode::KeyD,
            KeyCode::Tab,
            KeyCode::KeyV,
        ] {
            keys.press(key);
        }
        consume_key_edges(&mut keys);
        assert!(keys.just_pressed(KeyCode::KeyM));
        assert!(keys.just_pressed(KeyCode::KeyD));
        assert!(!keys.just_pressed(KeyCode::Tab));
        assert!(!keys.just_pressed(KeyCode::KeyV));
        assert!(keys.pressed(KeyCode::KeyV));
        assert!(keys.pressed(KeyCode::ControlLeft));
        keys.release(KeyCode::ControlLeft);
        consume_key_edges(&mut keys);
        assert!(!keys.just_pressed(KeyCode::KeyM));
        assert!(!keys.just_pressed(KeyCode::KeyD));
    }
}
