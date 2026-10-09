//! The retail menu registry (skate_core::menus) as a game resource: the
//! structure comes from setup's `assets/private/menu-tables.json` (extracted
//! from the user's own executable, skate_data::menu_tables); engine features
//! and mods register handlers on it; mods edit it through `sdk.menus`.
//! Drawing the menus is a later milestone: this holds data, hooks and events.
use bevy::prelude::*;
use serde_json::{json, Value};
use skate_core::menus::{
    Cmp, Condition, Edit, EntryKind, EntryRef, Handler, Icon, Interceptor, MenuAction, MenuContext, MenuData, MenuEvent,
    MenuRegistry, Outcome, Owner, Placement, Popup, Rule, SettingValue, ValueHook, Widget,
};
use skate_mods::menus::{EntryOptions, HandleOptions, MenuValue, RuleSpec};
use std::path::Path;

pub(crate) const TABLES: &str = "private/menu-tables.json";

#[derive(Resource)]
pub(crate) struct RetailMenus {
    pub registry: MenuRegistry,
    /// The current mode and facts (set by the systems that own them).
    pub context: MenuContext,
    /// Whether the retail tables were found (setup ran the extraction).
    pub loaded: bool,
    /// Events for mods, sent at the next mod update: (mod id, on_event payload).
    pending: Vec<(String, Value)>,
    /// Entries each mod touched (limit and cleanup bookkeeping).
    touched: std::collections::BTreeMap<String, std::collections::BTreeSet<EntryRef>>,
    /// Last value each engine binding synced (decides which side changed).
    synced: std::collections::BTreeMap<&'static str, SettingValue>,
    /// Mod values applied to the engine at runtime (not saved), per engine-bound row.
    overrides: std::collections::BTreeMap<&'static str, SettingValue>,
}

impl RetailMenus {
    pub(crate) fn new(data: MenuData, loaded: bool) -> Self {
        Self {
            registry: MenuRegistry::new(data),
            context: MenuContext::new("FreePlay"),
            loaded,
            pending: Vec::new(),
            touched: Default::default(),
            synced: Default::default(),
            overrides: Default::default(),
        }
    }

    /// Apply a menu action (from the menu UI, or replayed from another peer) in
    /// the current context; events for mods are queued.
    #[allow(dead_code)] // The menu UI (a later milestone) is the caller.
    pub(crate) fn act(&mut self, action: MenuAction) -> Outcome {
        let context = self.context.clone();
        let outcome = self.registry.dispatch(action, &context);
        if let Outcome::Forward(Owner::Mod(id), event) = &outcome {
            self.pending.push((id.clone(), event_json(event, self.registry.value(event.action.entry()))));
        }
        outcome
    }

    pub(crate) fn drain_events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.pending)
    }
}

/// The JSON a mod's on_event receives (also the serialised form of an action).
pub(crate) fn event_json(event: &MenuEvent, value: Option<SettingValue>) -> Value {
    let entry = event.action.entry();
    let name = match &event.action {
        MenuAction::Select { .. } | MenuAction::Confirm { .. } => "menu_select",
        MenuAction::Highlight { .. } => "menu_highlight",
        MenuAction::SetValue { .. } | MenuAction::Step { .. } => "menu_value",
    };
    let mut out = json!({"name": name, "entry": entry.kind.name(), "id": entry.id, "mode": event.mode});
    if let MenuAction::SetValue { value, .. } = &event.action {
        out["value"] = value_json(value);
    } else if let Some(value) = value {
        out["value"] = value_json(&value);
    }
    out
}

fn value_json(value: &SettingValue) -> Value {
    match value {
        SettingValue::Bool(b) => json!(b),
        SettingValue::Int(i) => json!(i),
        SettingValue::Float(f) => json!(f),
    }
}

fn setting_value(value: &MenuValue) -> SettingValue {
    match value {
        MenuValue::Bool(b) => SettingValue::Bool(*b),
        MenuValue::Int(i) => SettingValue::Int(*i),
        MenuValue::Float(f) => SettingValue::Float(*f),
    }
}

pub(crate) fn rule(spec: &RuleSpec) -> Rule {
    let RuleSpec::Table(t) = spec else {
        return if matches!(spec, RuleSpec::Bool(true)) { Rule::Always } else { Rule::Never };
    };
    if let Some(fact) = &t.fact {
        let (cmp, value) = [(Cmp::Eq, t.eq), (Cmp::Ne, t.ne), (Cmp::Lt, t.lt), (Cmp::Le, t.le), (Cmp::Gt, t.gt), (Cmp::Ge, t.ge)]
            .into_iter()
            .find_map(|(c, v)| v.map(|v| (c, v)))
            .unwrap_or((Cmp::Ne, 0.0));
        return Rule::fact(fact, cmp, value);
    }
    if let Some(modes) = &t.modes {
        return Rule::Mode(modes.clone());
    }
    if let Some(all) = &t.all {
        return Rule::All(all.iter().map(rule).collect());
    }
    if let Some(any) = &t.any {
        return Rule::Any(any.iter().map(rule).collect());
    }
    match &t.not {
        Some(inner) => Rule::Not(Box::new(rule(inner))),
        None => Rule::Never,
    }
}

/// Reads the setup output; absent or invalid = an empty structure (mods can
/// still add their own categories / items then).
pub(crate) fn read(root: &Path) -> (MenuData, bool) {
    let path = root.join(TABLES);
    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| skate_data::menu_tables::MenuTables::from_json(&t)) {
        Ok(tables) => (tables.to_menu_data(), true),
        Err(error) => {
            if path.exists() {
                warn!("RETAIL_MENUS: {}: {error}", path.display());
            }
            (MenuData::default(), false)
        }
    }
}

fn load(mut commands: Commands, config: Res<crate::config::Config>) {
    let (data, loaded) = read(&config.asset_root);
    if loaded {
        info!(
            "RETAIL_MENUS: {} categories, {} items, {} settings rows, {} modes",
            data.categories.len(),
            data.items.len(),
            data.settings.len(),
            data.modes.len()
        );
    }
    commands.insert_resource(RetailMenus::new(data, loaded));
}

pub(crate) struct RetailMenusPlugin;

impl Plugin for RetailMenusPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (load, crate::retail_menu_movies::load))
            .add_systems(Update, sync_engine_values);
    }
}

// ---------------------------------------------------------------- engine bindings

/// A retail settings row bound to an engine setting (Engine-owned, so the row is enabled).
/// Engine-only settings (resolution, FPS limit, Motorized / Custom difficulty, ...) stay in
/// the settings files (user decision 2026-10-08); they are not rows.
struct Binding {
    row: &'static str,
    owner: &'static str,
    read: fn(&World) -> Option<SettingValue>,
    write: fn(&mut World, &SettingValue),
    /// Apply a mod's value at runtime without saving (`Some`), or drop it again and go back to
    /// the user's own setting (`None`; the second argument is that setting's value).
    apply: fn(&mut World, Option<&SettingValue>, &SettingValue),
}

fn number(value: &SettingValue) -> f64 {
    skate_core::menu_values::number(value)
}

/// Retail's option volume behind a row, as the engine reads it.
fn read_volume(w: &World, volume: crate::game_audio::RetailVolume) -> Option<SettingValue> {
    w.get_resource::<crate::game_audio::AudioSettings>().map(|a| SettingValue::Float(f64::from(a.retail_volume(volume))))
}

/// A mod's value for one of retail's option volumes, in force but never saved.
fn apply_volume(w: &mut World, volume: crate::game_audio::RetailVolume, v: Option<&SettingValue>) {
    if let Some(mut audio) = w.get_resource_mut::<crate::game_audio::AudioSettings>() {
        audio.override_retail_volume(volume, v.map(|v| number(v) as f32));
    }
}

/// Set (and save) one of retail's option volumes from its row.
fn write_volume(w: &mut World, volume: crate::game_audio::RetailVolume, v: &SettingValue) {
    if let Some(mut audio) = w.get_resource_mut::<crate::game_audio::AudioSettings>() {
        let status = audio.set_retail_volume(volume, number(v) as f32);
        info!("RETAIL_MENUS: {volume:?} volume {}: {status}", number(v));
    }
}

/// Owner name of the camera hold a mod's Camera Angle value takes (`CameraAngleSettings::force`).
const MENU_CAMERA_OWNER: &str = "menus";

const BINDINGS: [Binding; 5] = [
    // The option volumes (rows 7 / 8 / 9, settings +128 / +132 / +136) feed the MixMap's Master
    // controller as retail's `sub_824D5160` does: SFX -> Master.in2, Dialog -> in3, Music -> in1
    // (`game_audio::RetailVolumes`).
    Binding {
        row: "ID_GAMESETTINGS_SFXVOLUME",
        owner: "audio",
        read: |w| read_volume(w, crate::game_audio::RetailVolume::Sfx),
        write: |w, v| write_volume(w, crate::game_audio::RetailVolume::Sfx, v),
        apply: |w, v, _| apply_volume(w, crate::game_audio::RetailVolume::Sfx, v),
    },
    Binding {
        row: "ID_GAMESETTINGS_DIALOGVOLUME",
        owner: "audio",
        read: |w| read_volume(w, crate::game_audio::RetailVolume::Dialog),
        write: |w, v| write_volume(w, crate::game_audio::RetailVolume::Dialog, v),
        apply: |w, v, _| apply_volume(w, crate::game_audio::RetailVolume::Dialog, v),
    },
    Binding {
        row: "ID_GAMESETTINGS_MUSICVOLUME",
        owner: "audio",
        read: |w| read_volume(w, crate::game_audio::RetailVolume::Music),
        write: |w, v| write_volume(w, crate::game_audio::RetailVolume::Music, v),
        apply: |w, v, _| apply_volume(w, crate::game_audio::RetailVolume::Music, v),
    },
    // Camera Angle (row 11, settings +188): 0 = LOW, 1 = HIGH, the camera graph type.
    Binding {
        row: "ID_GAMESETTINGS_CAMERA_ANGLE_TOGGLE",
        owner: "camera",
        read: |w| {
            w.get_resource::<crate::camera::CameraAngleSettings>().map(|c| SettingValue::Int(i64::from(c.selected.graph_type())))
        },
        write: |w, v| {
            if let Some(mut camera) = w.get_resource_mut::<crate::camera::CameraAngleSettings>() {
                camera.selected = if number(v) != 0.0 { crate::camera::CameraAngle::High } else { crate::camera::CameraAngle::Low };
                if let Err(e) = camera.save() {
                    warn!("RETAIL_MENUS: camera angle applied, not saved: {e}");
                }
            }
        },
        // A mod's value holds the camera's forced angle (owner `menus`); the player's choice is untouched.
        apply: |w, v, _| {
            let Some(mut camera) = w.get_resource_mut::<crate::camera::CameraAngleSettings>() else { return };
            if v.is_none() && camera.forced_by() != Some(MENU_CAMERA_OWNER) {
                return;
            }
            let angle = v.map(|v| if number(v) != 0.0 { crate::camera::CameraAngle::High } else { crate::camera::CameraAngle::Low });
            if let Err(e) = camera.force(MENU_CAMERA_OWNER, angle) {
                warn!("RETAIL_MENUS: camera angle not applied: {e}");
            }
        },
    },
    // Play Mode (row 28, settings +180): Easy 0 / Normal 1 / Hardcore 2. Our Motorized (3) and
    // Custom (4) show as their index; the retail rule steps them back into 0..2.
    Binding {
        row: "ID_GAMESETTINGS_DIFFICULTY_OPTIONS",
        owner: "difficulty",
        read: |w| w.get_resource::<crate::config::Config>().map(|c| SettingValue::Int(c.difficulty as i64)),
        write: |w, v| {
            let Some(difficulty) = crate::difficulty::Difficulty::ALL.get(number(v) as usize).copied() else { return };
            if let Some(mut physics) = w.get_resource_mut::<crate::physics::GamePhysics>() {
                physics.set_difficulty(difficulty);
            }
            let Some(mut config) = w.get_resource_mut::<crate::config::Config>() else { return };
            config.difficulty = difficulty;
            if let Err(e) = difficulty.save(&config.asset_root) {
                warn!("RETAIL_MENUS: difficulty applied, not saved: {e}");
            }
        },
        // A mod's value is applied without saving; dropping it applies the user's setting again.
        apply: |w, v, user| {
            let v = v.unwrap_or(user);
            let Some(difficulty) = crate::difficulty::Difficulty::ALL.get(number(v) as usize).copied() else { return };
            if let Some(mut physics) = w.get_resource_mut::<crate::physics::GamePhysics>() {
                physics.set_difficulty(difficulty);
            }
            if let Some(mut config) = w.get_resource_mut::<crate::config::Config>() {
                config.difficulty = difficulty;
            }
        },
    },
];

/// Two-way sync between the Engine-owned bindings and the engine settings: a menu change
/// (or a mod's `set_value` that reached the engine layer) is applied and saved; a change
/// from elsewhere (old menu, settings file) updates the row without an event.
fn sync_engine_values(world: &mut World) {
    let Some(menus) = world.get_resource::<RetailMenus>() else { return };
    if !menus.loaded {
        return;
    }
    for binding in &BINDINGS {
        let entry = EntryRef::setting(binding.row);
        let owner = Owner::Engine(binding.owner.into());
        let Some(engine) = (binding.read)(world) else { continue };
        let mut menus = world.resource_mut::<RetailMenus>();
        if menus.registry.data().setting(binding.row).is_none() {
            continue;
        }
        let current = menus.registry.owner_value(&entry, &owner);
        // A mod's value layer on top: applied at runtime, never saved; the engine layer keeps the
        // user's setting. When the layer goes (disable, reload, failure) that setting is applied again.
        if let Some(user) = current.clone() {
            if matches!(menus.registry.value_owner(&entry), Some(Owner::Mod(_))) {
                let Some(value) = menus.registry.value(&entry) else { continue };
                if menus.overrides.get(binding.row) != Some(&value) {
                    menus.overrides.insert(binding.row, value.clone());
                    drop(menus);
                    (binding.apply)(world, Some(&value), &user);
                }
                continue;
            }
            if menus.overrides.remove(binding.row).is_some() {
                drop(menus);
                (binding.apply)(world, None, &user);
                continue;
            }
        }
        match current {
            None => {
                menus.registry.bind_value(entry, owner, ValueHook::Stored(engine.clone()));
                menus.synced.insert(binding.row, engine);
            }
            Some(row) if menus.synced.get(binding.row) != Some(&row) => {
                menus.synced.insert(binding.row, row.clone());
                drop(menus);
                (binding.write)(world, &row);
            }
            Some(row) if row != engine => {
                menus.registry.set_owner_value(&entry, &owner, engine.clone());
                menus.synced.insert(binding.row, engine);
            }
            Some(_) => {}
        }
    }
}

// ---------------------------------------------------------------- mods

fn kind(name: &str) -> Result<EntryKind, String> {
    EntryKind::parse(name).ok_or_else(|| format!("unknown menu entry kind {name}"))
}

fn icon(options: &EntryOptions) -> Option<Icon> {
    options.image.clone().map(Icon::Image).or_else(|| options.icon.clone().map(Icon::Retail))
}

fn placement(options: &EntryOptions, with_category: bool) -> Option<Placement> {
    let any = !options.modes.is_empty() || options.position.is_some() || (with_category && options.category.is_some());
    any.then(|| Placement {
        modes: options.modes.clone(),
        category: options.category.clone().filter(|_| with_category),
        position: options.position,
    })
}

/// Apply one `sdk.menus` command for mod `id`.
pub(crate) fn apply_command(menus: &mut RetailMenus, id: &str, command: skate_mods::Command) -> Result<(), String> {
    use skate_mods::Command as C;
    let owner = Owner::Mod(id.to_owned());
    let touch = |menus: &mut RetailMenus, entry: EntryRef, new: bool| -> Result<(), String> {
        if new && !entry.id.starts_with(&format!("{id}.")) {
            return Err(format!("new menu entry {} must be named {id}.<name>", entry.id));
        }
        let set = menus.touched.entry(id.to_owned()).or_default();
        if !set.contains(&entry) && set.len() >= skate_mods::menus::MAX_ENTRIES_PER_MOD {
            return Err(format!("{} menu entries per mod maximum", skate_mods::menus::MAX_ENTRIES_PER_MOD));
        }
        set.insert(entry);
        Ok(())
    };
    match command {
        C::MenuCategory { id: entry, options } => {
            let new = menus.registry.data().category(&entry).is_none();
            touch(menus, EntryRef::category(&entry), new)?;
            let edit = Edit::Category { id: entry, label: options.label.clone(), icon: icon(&options), place: placement(&options, false) };
            menus.registry.edit(owner, edit)
        }
        C::MenuItem { id: entry, options } => {
            let new = menus.registry.data().item(&entry).is_none();
            touch(menus, EntryRef::item(&entry), new)?;
            let edit = Edit::Item {
                id: entry,
                label: options.label.clone(),
                icon: icon(&options),
                help: options.help.clone(),
                place: placement(&options, true),
            };
            menus.registry.edit(owner, edit)
        }
        C::MenuSetting { id: entry, options } => {
            let new = menus.registry.data().setting(&entry).is_none();
            touch(menus, EntryRef::setting(&entry), new)?;
            let widget = options.widget.as_deref().and_then(Widget::parse);
            let edit = Edit::Setting {
                id: entry,
                label: options.label.clone(),
                widget,
                link: None,
                place: options.screen.map(|s| (s, options.position)),
            };
            menus.registry.edit(owner, edit)
        }
        C::MenuHide { entry, id: name, modes } => {
            let entry = EntryRef { kind: kind(&entry)?, id: name };
            if !menus.registry.data().contains(&entry) {
                return Err(format!("no menu entry {}", entry.id));
            }
            touch(menus, entry.clone(), false)?;
            menus.registry.edit(owner, Edit::Hide { entry, modes })
        }
        C::MenuHandle { entry, id: name, options } => {
            let entry = EntryRef { kind: kind(&entry)?, id: name };
            if !menus.registry.data().contains(&entry) {
                return Err(format!("no menu entry {}", entry.id));
            }
            touch(menus, entry.clone(), false)?;
            handle(&mut menus.registry, entry, owner, &options);
            Ok(())
        }
        C::MenuUnhandle { entry, id: name } => {
            let entry = EntryRef { kind: kind(&entry)?, id: name };
            menus.registry.remove_owner_from(&entry, &owner);
            Ok(())
        }
        C::MenuSetValue { id: name, value } => {
            let entry = EntryRef::setting(&name);
            if menus.registry.value(&entry).is_none() {
                return Err(format!("menu setting {name} has no bound value"));
            }
            if !menus.registry.enabled(&entry, &menus.context) {
                return Err(format!("menu setting {name} is not enabled"));
            }
            // The mod's own top layer takes the value like a menu change (`menu_value` event).
            if menus.registry.value_owner(&entry) == Some(&owner) {
                return match menus.act(MenuAction::SetValue { entry, value: setting_value(&value) }) {
                    Outcome::Ignored => Err(format!("menu setting {name} is not enabled")),
                    _ => Ok(()),
                };
            }
            // Anything else (an engine binding = the user's real setting, another mod's value): the
            // value goes into this mod's own layer on top (last writer wins), never below it, so
            // removing the mod restores exactly what was there and no settings file is written.
            touch(menus, entry.clone(), false)?;
            menus.registry.bind_value(entry, owner, ValueHook::Stored(setting_value(&value)));
            Ok(())
        }
        _ => Err("not a menu command".into()),
    }
}

fn value_rule(spec: &skate_mods::menus::ValueRuleSpec) -> skate_core::menus::ValueRule {
    use skate_core::menus::ValueRule;
    use skate_mods::menus::ValueRuleSpec as S;
    match spec {
        S::Slider { min, max, step, snap, bars } => {
            ValueRule::Slider { min: *min as f32, max: *max as f32, step: *step as f32, snap: *snap as f32, bars: *bars as f32 }
        }
        S::Toggle { labels } => ValueRule::Toggle { labels: labels.clone() },
        S::Cycle { count, labels } => ValueRule::Cycle { count: *count, labels: labels.clone() },
    }
}

fn handle(registry: &mut MenuRegistry, entry: EntryRef, owner: Owner, options: &HandleOptions) {
    registry.remove_owner_from(&entry, &owner);
    if options.select || options.highlight {
        registry.register_handler(entry.clone(), Handler { owner: owner.clone(), select: options.select, highlight: options.highlight, callback: None });
    }
    if let Some(value) = &options.value {
        registry.bind_value(entry.clone(), owner.clone(), ValueHook::Stored(setting_value(value)));
    }
    if let Some(spec) = &options.rule {
        registry.set_rule(entry.clone(), owner.clone(), value_rule(spec));
    }
    if let Some(spec) = &options.enabled {
        registry.set_enabled(entry.clone(), owner.clone(), Condition::Rule(rule(spec)));
    }
    if let Some(spec) = &options.visible {
        registry.set_visible(entry.clone(), owner.clone(), Condition::Rule(rule(spec)));
    }
    if let Some(confirm) = &options.confirm {
        registry.add_interceptor(
            entry,
            Interceptor {
                owner,
                when: confirm.when.as_ref().map(rule).unwrap_or(Rule::Always),
                popup: Popup {
                    title: confirm.title.clone(),
                    description: confirm.description.clone(),
                    yes: confirm.yes.clone().unwrap_or_else(|| "ID_COMMON_YES".into()),
                    no: confirm.no.clone().unwrap_or_else(|| "ID_COMMON_NO".into()),
                },
            },
        );
    }
}

/// Everything mod `id` did is removed (disable, reload, failure).
pub(crate) fn clear_owner(world: &mut World, id: &str) {
    if let Some(mut movies) = world.get_resource_mut::<crate::retail_menu_movies::RetailMenuMovies>() {
        movies.clear_owner(id);
    }
    if let Some(mut menus) = world.get_resource_mut::<RetailMenus>() {
        menus.registry.remove_owner(&Owner::Mod(id.to_owned()));
        menus.touched.remove(id);
        menus.pending.retain(|(owner, _)| owner != id);
    }
}

pub(crate) fn clear_mods(world: &mut World) {
    if let Some(mut movies) = world.get_resource_mut::<crate::retail_menu_movies::RetailMenuMovies>() {
        movies.clear_mods();
    }
    if let Some(mut menus) = world.get_resource_mut::<RetailMenus>() {
        let ids: Vec<String> = menus.touched.keys().cloned().collect();
        for id in ids {
            menus.registry.remove_owner(&Owner::Mod(id));
        }
        menus.touched.clear();
        menus.pending.clear();
    }
}

/// `sdk.snapshot.menus`: the mode, whether retail tables loaded, and every bound value.
pub(crate) fn snapshot(world: &World) -> Value {
    let Some(menus) = world.get_resource::<RetailMenus>() else { return json!({"loaded": false}) };
    let data = menus.registry.data();
    let values: serde_json::Map<String, Value> = data
        .settings
        .iter()
        .filter_map(|s| menus.registry.value(&EntryRef::setting(&s.id)).map(|v| (s.id.clone(), value_json(&v))))
        .collect();
    let movies = world
        .get_resource::<crate::retail_menu_movies::RetailMenuMovies>()
        .map_or(Value::Null, |m| m.snapshot());
    json!({"mode": menus.context.mode, "loaded": menus.loaded, "values": values, "movies": movies})
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::menus::{Category, Mode, Tab};

    fn data() -> MenuData {
        MenuData {
            categories: vec![Category { id: "SinglePlayer".into(), label: "L".into(), icon: Icon::Retail("singleplayer".into()) }],
            items: vec![skate_core::menus::Item { id: "ChallengeMap".into(), label: "L".into(), icon: Icon::Retail("map".into()), help: String::new(), sub_option_kind: -1 }],
            settings: Vec::new(),
            screens: vec![Vec::new(); 13],
            modes: vec![Mode { key: "FreePlay".into(), title: "T".into(), slots: 8, tabs: vec![Tab { category: "SinglePlayer".into(), items: vec!["ChallengeMap".into()] }] }],
        }
    }

    fn command(value: Value) -> skate_mods::Command {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn mod_commands_edit_handle_and_revert() {
        let mut menus = RetailMenus::new(data(), true);
        let retail = menus.registry.data().clone();
        apply_command(&mut menus, "m", command(json!({"kind":"menu_item","id":"m.Races","options":{"label":"Races","icon":"map","category":"SinglePlayer","position":0}}))).unwrap();
        assert!(apply_command(&mut menus, "m", command(json!({"kind":"menu_item","id":"Races","options":{"category":"SinglePlayer"}}))).is_err(), "new ids carry the mod id");
        apply_command(&mut menus, "m", command(json!({"kind":"menu_setting","id":"m.Bass","options":{"widget":"slider","screen":3}}))).unwrap();
        apply_command(&mut menus, "m", command(json!({"kind":"menu_handle","entry":"setting","id":"m.Bass","options":{"value":0.5}}))).unwrap();
        apply_command(&mut menus, "m", command(json!({"kind":"menu_handle","entry":"item","id":"m.Races"}))).unwrap();
        let ctx = menus.context.clone();
        let rows = menus.registry.tab_rows("SinglePlayer", &ctx);
        assert_eq!((rows[0].id.as_str(), rows[0].enabled, rows[1].enabled), ("m.Races", true, false));

        apply_command(&mut menus, "m", command(json!({"kind":"menu_set_value","id":"m.Bass","value":0.8}))).unwrap();
        assert!(matches!(menus.act(MenuAction::Select { entry: EntryRef::item("m.Races") }), Outcome::Forward(..)));
        let events = menus.drain_events();
        assert_eq!(events[0].1, json!({"name":"menu_value","entry":"setting","id":"m.Bass","mode":"FreePlay","value":0.8}));
        assert_eq!(events[1].1, json!({"name":"menu_select","entry":"item","id":"m.Races","mode":"FreePlay"}));

        menus.registry.remove_owner(&Owner::Mod("m".into()));
        assert_eq!(*menus.registry.data(), retail);
    }

    #[test]
    fn rules_from_lua_tables() {
        let spec: RuleSpec = serde_json::from_value(json!({"all":[{"fact":"sfx_packs","gt":1},{"not":{"modes":["CareerPark"]}}]})).unwrap();
        let r = rule(&spec);
        assert!(r.eval(&MenuContext::new("Career").with("sfx_packs", 2.0)));
        assert!(!r.eval(&MenuContext::new("CareerPark").with("sfx_packs", 2.0)));
        assert_eq!(rule(&RuleSpec::Bool(false)), Rule::Never);
    }

    #[test]
    fn camera_angle_row_round_trips_with_the_engine_setting() {
        use crate::camera::{CameraAngle, CameraAngleSettings};
        use skate_core::menus::{Direction, Setting};
        let mut d = data();
        let id = "ID_GAMESETTINGS_CAMERA_ANGLE_TOGGLE";
        d.settings.push(Setting { id: id.into(), label: id.into(), widget: Widget::Selector, link: None });
        d.screens[4].push(id.into());
        let dir = std::env::temp_dir().join(format!("skate3rust-menus-m2-{}", std::process::id()));
        let mut world = World::new();
        world.insert_resource(RetailMenus::new(d, true));
        world.insert_resource(CameraAngleSettings::new(CameraAngle::High, dir.join("camera.json")));
        sync_engine_values(&mut world);
        let entry = EntryRef::setting(id);
        assert_eq!(world.resource::<RetailMenus>().registry.value(&entry), Some(SettingValue::Int(1)), "bound and enabled from the engine");
        // Left on the row (retail toggles on either direction) reaches the camera and is saved.
        world.resource_mut::<RetailMenus>().act(MenuAction::Step { entry: entry.clone(), direction: Direction::Left });
        sync_engine_values(&mut world);
        assert_eq!(world.resource::<CameraAngleSettings>().selected, CameraAngle::Low);
        assert!(dir.join("camera.json").exists());
        // A change from elsewhere (old menu, settings file) shows in the row.
        world.resource_mut::<CameraAngleSettings>().selected = CameraAngle::High;
        sync_engine_values(&mut world);
        assert_eq!(world.resource::<RetailMenus>().registry.value(&entry), Some(SettingValue::Int(1)));
        // A mod override sits on top and is dropped on disable; the engine value shows again.
        let mut menus = world.resource_mut::<RetailMenus>();
        apply_command(&mut menus, "m", command(json!({"kind":"menu_handle","entry":"setting","id":id,"options":{"select":false,"value":0,"rule":{"kind":"cycle","count":2}}}))).unwrap();
        assert_eq!(menus.registry.value(&entry), Some(SettingValue::Int(0)));
        menus.registry.remove_owner(&Owner::Mod("m".into()));
        assert_eq!(menus.registry.value(&entry), Some(SettingValue::Int(1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn volume_rows_round_trip_to_the_master_inputs() {
        use crate::game_audio::{AudioSettings, RetailVolume};
        use skate_core::menus::{Direction, Setting};
        let rows = [
            ("ID_GAMESETTINGS_SFXVOLUME", RetailVolume::Sfx, 2),
            ("ID_GAMESETTINGS_DIALOGVOLUME", RetailVolume::Dialog, 3),
            ("ID_GAMESETTINGS_MUSICVOLUME", RetailVolume::Music, 1),
        ];
        let mut d = data();
        for (id, _, _) in rows {
            d.settings.push(Setting { id: id.into(), label: id.into(), widget: Widget::Slider, link: None });
            d.screens[3].push(id.into());
        }
        let dir = std::env::temp_dir().join(format!("skate3rust-menus-volumes-{}", std::process::id()));
        let mut world = World::new();
        world.insert_resource(RetailMenus::new(d, true));
        world.insert_resource(AudioSettings::for_test(dir.join("audio.json")));
        sync_engine_values(&mut world);
        for (step, (id, volume, input)) in rows.into_iter().enumerate() {
            let entry = EntryRef::setting(id);
            assert_eq!(world.resource::<RetailMenus>().registry.value(&entry), Some(SettingValue::Float(1.0)), "{id}: bound and enabled");
            // Left steps 0.1 down (retail's slider rule); the row's volume reaches its Master input only.
            for _ in 0..=step {
                world.resource_mut::<RetailMenus>().act(MenuAction::Step { entry: entry.clone(), direction: Direction::Left });
                sync_engine_values(&mut world);
            }
            let audio = world.resource::<AudioSettings>();
            let expected = 1.0 - 0.1 * (step + 1) as f64;
            assert!((f64::from(audio.retail_volume(volume)) - expected).abs() < 1e-6, "{id}: {}", audio.retail_volume(volume));
            let inputs = audio.master_inputs();
            let value = inputs.iter().find(|(i, _)| *i == input).unwrap().1;
            assert_eq!(value, crate::game_audio::RetailVolumes::master_input(audio.retail_volume(volume)), "{id} -> Master.in{input}");
            assert!((f64::from(value) - expected * 32767.0).abs() < 2.0, "{id} -> Master.in{input} = {value}");
            assert!(inputs.iter().filter(|(i, _)| *i != input).all(|(i, v)| *v == 32767 || rows[..step].iter().any(|r| r.2 == *i)), "{id}: other rows untouched");
            // A change from elsewhere shows in the row.
            world.resource_mut::<AudioSettings>().set_retail_volume(volume, 0.5);
            sync_engine_values(&mut world);
            assert_eq!(world.resource::<RetailMenus>().registry.value(&entry), Some(SettingValue::Float(0.5)));
            world.resource_mut::<AudioSettings>().set_retail_volume(volume, expected as f32);
            sync_engine_values(&mut world);
        }
        assert!(dir.join("audio.json").exists(), "saved");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `sdk.menus.set_value` on Engine-bound rows: the mod's value is shown and applied at runtime
    /// from its own layer, never saved; each mod's removal reverts only its own layer.
    #[test]
    fn mod_set_value_layers_over_engine_rows_and_reverts() {
        use crate::camera::{CameraAngle, CameraAngleSettings};
        use crate::game_audio::{AudioSettings, RetailVolume, RetailVolumes};
        use skate_core::menus::Setting;
        let sfx = "ID_GAMESETTINGS_SFXVOLUME";
        let cam = "ID_GAMESETTINGS_CAMERA_ANGLE_TOGGLE";
        let mut d = data();
        d.settings.push(Setting { id: sfx.into(), label: sfx.into(), widget: Widget::Slider, link: None });
        d.settings.push(Setting { id: cam.into(), label: cam.into(), widget: Widget::Selector, link: None });
        d.screens[3].push(sfx.into());
        d.screens[4].push(cam.into());
        let dir = std::env::temp_dir().join(format!("skate3rust-menus-setvalue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut world = World::new();
        world.insert_resource(RetailMenus::new(d, true));
        let mut audio = AudioSettings::for_test(dir.join("audio.json"));
        let _ = audio.set_retail_volume(RetailVolume::Sfx, 0.7);
        world.insert_resource(audio);
        world.insert_resource(CameraAngleSettings::new(CameraAngle::High, dir.join("camera.json")));
        let saved_audio = std::fs::read(dir.join("audio.json")).unwrap();
        sync_engine_values(&mut world);
        let (sfx_entry, cam_entry) = (EntryRef::setting(sfx), EntryRef::setting(cam));
        let set = |world: &mut World, owner: &str, id: &str, value: Value| {
            apply_command(&mut world.resource_mut::<RetailMenus>(), owner, command(json!({"kind":"menu_set_value","id":id,"value":value}))).unwrap();
            sync_engine_values(world);
        };
        let shown = |world: &World, entry: &EntryRef| world.resource::<RetailMenus>().registry.value(entry);
        let sfx_input = |world: &World| world.resource::<AudioSettings>().master_inputs()[1].1;

        // Mod a: shown and applied (Master.in2, the forced camera angle); the user's settings stay.
        set(&mut world, "a", sfx, json!(0.3));
        set(&mut world, "a", cam, json!(0));
        assert_eq!(shown(&world, &sfx_entry), Some(SettingValue::Float(0.3)));
        assert_eq!(sfx_input(&world), RetailVolumes::master_input(0.3));
        assert_eq!(world.resource::<AudioSettings>().retail_volume(RetailVolume::Sfx), 0.7, "user's volume untouched");
        let camera = world.resource::<CameraAngleSettings>();
        assert_eq!((camera.active(), camera.selected), (CameraAngle::Low, CameraAngle::High));
        assert!(world.resource::<RetailMenus>().pending.is_empty(), "no event for the mod's own write");

        // Mod b on top: last writer wins; mod a writing again moves its layer back on top.
        set(&mut world, "b", sfx, json!(0.6));
        assert_eq!((shown(&world, &sfx_entry), sfx_input(&world)), (Some(SettingValue::Float(0.6)), RetailVolumes::master_input(0.6)));
        set(&mut world, "a", sfx, json!(0.2));
        assert_eq!((shown(&world, &sfx_entry), sfx_input(&world)), (Some(SettingValue::Float(0.2)), RetailVolumes::master_input(0.2)));

        // Removing a reverts only a's layers: b's value shows again, the camera is the user's.
        clear_owner(&mut world, "a");
        sync_engine_values(&mut world);
        assert_eq!((shown(&world, &sfx_entry), sfx_input(&world)), (Some(SettingValue::Float(0.6)), RetailVolumes::master_input(0.6)));
        assert_eq!(shown(&world, &cam_entry), Some(SettingValue::Int(1)));
        let camera = world.resource::<CameraAngleSettings>();
        assert_eq!((camera.active(), camera.forced_by()), (CameraAngle::High, None));

        // Removing b: the engine value exactly as before; no settings file was written.
        clear_owner(&mut world, "b");
        sync_engine_values(&mut world);
        assert_eq!((shown(&world, &sfx_entry), sfx_input(&world)), (Some(SettingValue::Float(f64::from(0.7f32))), RetailVolumes::master_input(0.7)));
        assert_eq!(std::fs::read(dir.join("audio.json")).unwrap(), saved_audio, "audio.json unchanged");
        assert!(!dir.join("camera.json").exists(), "camera.json never written");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_tables_give_an_empty_structure() {
        let (data, loaded) = read(Path::new("does-not-exist"));
        assert!(!loaded && data.items.is_empty());
    }
}
