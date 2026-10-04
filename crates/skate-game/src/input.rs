//! Platform input adapter; no animation or physics state mutation here.
use crate::app::SimulationSet;
use bevy::prelude::*;

mod controllers;
pub(crate) mod controller_kind;
#[cfg(test)]
#[path = "input/tests/action_docs.rs"]
mod action_docs;
pub(crate) mod gesture_catalog;
mod gesture_mapping_data;
pub(crate) mod gesture_mapping;
pub(crate) mod gesture_input;
pub(crate) mod platform;
pub(crate) use controllers::{ControllerInput, ControllerStatus, RawInput};
use skate_core::input::tick::TickInput;

#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct PublishedTickInput(pub TickInput);

impl Default for PublishedTickInput {
    fn default() -> Self {
        Self(TickInput::new(
            0,
            skate_core::input::gameplay_map::GameplayActions::from_values([0.0; 18]),
            false,
        ))
    }
}

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .add_systems(Startup, start_controllers)
            .add_systems(PreUpdate, poll_controllers.run_if(crate::graphics_menu::gameplay_active))
            .add_systems(FixedUpdate, publish_actions.in_set(SimulationSet::Input));
    }
}

/// settings/controller.json, e.g. {"paddles": {"right1": "a", "left1": "x"}}.
/// Paddle names are right1, left1, right2, left2 (SDL paddle order); values
/// are names from `platform::BUTTON_NAMES`.
/// `models` names pads the built-in table lacks (or renames them):
/// [{"vendor": "2dc8", "product": "3106", "name": "8BitDo", "family": "xbox_one", "paddles": 2}].
#[derive(serde::Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ControllerSettings {
    paddles: std::collections::BTreeMap<String, String>,
    models: Vec<ModelSetting>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelSetting {
    vendor: serde_json::Value,
    product: serde_json::Value,
    name: String,
    #[serde(default)]
    family: Option<controller_kind::Family>,
    #[serde(default)]
    paddles: u8,
}

fn user_models(settings: &ControllerSettings, path: &std::path::Path) -> Vec<controller_kind::Model> {
    settings.models.iter().filter_map(|m| {
        match (controller_kind::parse_id(&m.vendor), controller_kind::parse_id(&m.product)) {
            (Some(vendor), Some(product)) => Some(controller_kind::Model {
                vendor, product, name: m.name.clone(), family: m.family, paddles: m.paddles,
            }),
            _ => {
                warn!("{}: ignoring controller model {:?} (vendor/product must be hex ids)", path.display(), m.name);
                None
            }
        }
    }).collect()
}

fn start_controllers(config: Res<crate::config::Config>) {
    let root = &config.asset_root;
    let path = root.parent().unwrap_or(root).join("settings/controller.json");
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<ControllerSettings>(&bytes) {
            Ok(settings) => {
                let mut masks = [0u16; 4];
                for (paddle, button) in &settings.paddles {
                    let slot = ["right1", "left1", "right2", "left2"].iter().position(|p| p == paddle);
                    match (slot, platform::button_mask(button)) {
                        (Some(slot), Some(mask)) => masks[slot] = mask,
                        _ => warn!("{}: ignoring paddle mapping {paddle:?} -> {button:?}", path.display()),
                    }
                }
                platform::set_paddles(masks);
                controller_kind::set_user_models(user_models(&settings, &path));
            }
            Err(error) => warn!("Invalid controller settings {}: {error}", path.display()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!("Cannot read controller settings {}: {error}", path.display()),
    }
    // Start the device backend now rather than stalling the first gameplay frame.
    let _ = platform::poll(0);
}

pub(crate) fn poll_controllers(mut input: ResMut<ControllerInput>,config:Res<crate::config::Config>,net:Option<Res<crate::multiplayer::Multiplayer>>,windows:Query<&Window>,mut capabilities:Local<[platform::CapabilityCache;4]>) {
    let previous = input.status;
    let previous_kinds = input.kinds.clone();
    let focused=windows.iter().any(|w|w.focused);
    let active=net.is_some_and(|n|n.active());
    input.collect(std::array::from_fn(|slot| {
        if active && ((!focused && config.multiplayer.controller.is_none()) || config.multiplayer.controller.is_some_and(|selected|selected as usize!=slot)) {
            capabilities[slot].invalidate();
            Err(platform::DeviceError::Disconnected)
        } else {platform::poll_cached(slot, &mut capabilities[slot])}
    }));
    for (index, (&before, &after)) in previous.iter().zip(&input.status).enumerate() {
        if before != after {
            match after {
                ControllerStatus::Ready => info!("Controller {index}: ready"),
                ControllerStatus::Unavailable(platform::DeviceError::Disconnected) => {
                    info!("Controller {index}: disconnected");
                }
                _ => warn!("Controller {index}: {after:?}"),
            }
        }
    }
    for (index, (before, after)) in previous_kinds.iter().zip(&input.kinds).enumerate() {
        if let Some(kind) = after.as_deref().filter(|&kind| before.as_deref() != Some(kind)) {
            info!("Controller {index}: identified as {}", kind.summary());
        }
    }
}

pub(crate) fn publish_actions(
    mut input: ResMut<ControllerInput>,
    mut published: ResMut<PublishedTickInput>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    debug: Res<crate::debug_cam::DebugCam>,
    camera: Res<crate::camera::CameraRuntime>,
    mods: Option<Res<crate::modding::Mods>>,
) {
    let blocked = !crate::graphics_menu::gameplay_active(menu) || debug.suppress_gameplay(&camera) || crate::modding::browser_focused(mods.as_deref());
    if blocked {
        input.discard_gameplay();
    }
    input.publish_actions();
    let tick = input.tick_input();
    let mut values=*tick.actions().values();
    if !blocked { crate::modding::override_actions(mods.as_deref(), &mut values); }
    let tick=TickInput::new(tick.tick(),skate_core::input::gameplay_map::GameplayActions::from_values(values),tick.controller_available());
    published.0 = if debug.suppress_gameplay(&camera) {
        TickInput::new(
            tick.tick(),
            skate_core::input::gameplay_map::GameplayActions::from_values([0.0; 18]),
            tick.controller_available(),
        )
    } else {
        tick
    };
}
