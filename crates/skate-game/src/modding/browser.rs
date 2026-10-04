//! Native resource browser ownership and lifecycle. The browser receives only
//! packaged web assets and JSON; every engine command still comes from its VM.
use super::Mods;
use bevy::prelude::*;
use skate_browser::{Event, Init, Input, Options, process::Page};
use std::collections::BTreeMap;
type Slot = (String, String);
pub(super) struct Owned {
    page: Page,
    ready: bool,
    focused: bool,
}
pub(super) type Pages = BTreeMap<Slot, Owned>;

pub(super) fn focused(mods: &Mods) -> bool {
    mods.browsers.values().any(|p| p.focused)
}
fn restore_input(world: &mut World) {
    if let Some(mut keys) = world.get_resource_mut::<ButtonInput<KeyCode>>() {
        keys.reset_all();
    }
    if let Some(mut buttons) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
        buttons.reset_all();
    }
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.focused = true;
    }
}
pub(super) fn open(
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
    let page = Page::open(
        &executable,
        Init {
            root,
            title: format!("{owner} — {}", slot.1),
            options,
        },
    )?;
    info!("RESOURCE_BROWSER_OPEN resource={owner} key={} pid={}", slot.1, page.id());
    mods.browsers.insert(
        slot,
        Owned {
            page,
            ready: false,
            focused,
        },
    );
    Ok(())
}
pub(super) fn send(
    mods: &mut Mods,
    owner: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let page = mods
        .browsers
        .get(&(owner.to_owned(), key.to_owned()))
        .ok_or("browser page is not open")?;
    if !page.ready {
        return Err("browser page is not ready; await its ready event".into());
    }
    page.page.send(&Input::Message { value })
}
pub(super) fn focus(
    world: &mut World,
    mods: &mut Mods,
    owner: &str,
    key: &str,
    focused: bool,
) -> Result<(), String> {
    let slot = (owner.to_owned(), key.to_owned());
    if focused
        && mods
            .browsers
            .iter()
            .any(|(id, page)| id != &slot && page.focused)
    {
        return Err("another browser page owns input focus".into());
    }
    let page = mods
        .browsers
        .get_mut(&slot)
        .ok_or("browser page is not open")?;
    page.page.send(&Input::Focus { focused })?;
    let was = page.focused;
    page.focused = focused;
    if was && !focused {
        restore_input(world);
    }
    Ok(())
}
pub(super) fn close(world: &mut World, mods: &mut Mods, owner: &str, key: &str) {
    if let Some(page) = mods.browsers.remove(&(owner.to_owned(), key.to_owned())) {
        let focused = page.focused;
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
pub(super) fn poll(world: &mut World, mods: &mut Mods) {
    let mut events = Vec::new();
    let had_focus = focused(mods);
    for ((owner, key), page) in &mut mods.browsers {
        for event in page.page.poll() {
            match &event {
                Event::Ready => {
                    page.ready = true;
                    info!("RESOURCE_BROWSER_READY resource={owner} key={key}");
                },
                Event::Focus { focused } => page.focused = *focused,
                _ => {}
            }
            events.push((owner.clone(), key.clone(), event));
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
