//! Host-owned scene photography. Scripts may open a viewfinder, never press its shutter.
use super::Mods;
use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::{
            Hdr,
            screenshot::{Screenshot, ScreenshotCaptured},
        },
    },
};
use serde_json::{Value, json};
use skate_mods::photo::Operation;
use std::{
    collections::{BTreeMap, VecDeque},
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const MAX_GALLERY: usize = 32;
const MAX_TOTAL_GALLERY: usize = 64;
const SHUTTER_INTERVAL: Duration = Duration::from_secs(1);
const READBACK_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_THUMBNAIL: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Owner {
    id: String,
    generation: u64,
}
impl Owner {
    fn live(&self, mods: &Mods) -> bool {
        mods.manager.resources.as_ref().is_some_and(|host| {
            host.running(&self.id) && host.generation(&self.id) == Some(self.generation)
        })
    }
}

struct Viewfinder {
    owner: Owner,
    transform: Transform,
    fov: f32,
    yaw: f32,
    pitch: f32,
}
struct Photo {
    id: String,
    thumbnail: String,
}
struct Saved {
    photo: Photo,
    path: PathBuf,
}
enum Phase {
    Waiting(u8),
    Readback,
    Captured(Image),
    Saving(Mutex<mpsc::Receiver<Result<Saved, String>>>),
}
struct Job {
    owner: Owner,
    id: String,
    started: Instant,
    cancelled: Arc<AtomicBool>,
    camera: Option<Entity>,
    image: Option<Handle<Image>>,
    screenshot: Option<Entity>,
    phase: Phase,
}

#[derive(Default, Resource)]
struct Photos {
    mode: Option<Viewfinder>,
    pending: Option<Job>,
    gallery: BTreeMap<Owner, VecDeque<Photo>>,
    events: VecDeque<(Owner, Value)>,
    sequence: u64,
    last_shutter: Option<Instant>,
    status: String,
}

#[derive(Component)]
struct ViewfinderHud;
#[derive(Component)]
struct ViewfinderStatus;

pub(super) fn install(app: &mut App) {
    app.init_resource::<Photos>()
        .add_systems(PostStartup, setup_hud)
        .add_systems(Update, draw_hud)
        .add_systems(
            PostUpdate,
            present.before(bevy::transform::TransformSystems::Propagate),
        );
}

pub(super) fn active(world: &World) -> bool {
    world
        .get_resource::<Photos>()
        .is_some_and(|state| state.mode.is_some())
}

pub(super) fn owner(world: &World) -> Option<&str> {
    world
        .get_resource::<Photos>()?
        .mode
        .as_ref()
        .map(|mode| mode.owner.id.as_str())
}

/// Host escape route, independent of the owning page's responsiveness.
pub(super) fn leave(world: &mut World) {
    let Some(mut state) = world.get_resource_mut::<Photos>() else {
        return;
    };
    if let Some(mode) = state.mode.take() {
        event(
            &mut state,
            &mode.owner,
            json!({"kind":"mode","enabled":false}),
        );
    }
}

fn event(state: &mut Photos, owner: &Owner, value: Value) {
    state.status = match value["kind"].as_str() {
        Some("saving") => "Saving photograph…".into(),
        Some("saved") => "Saved to Desktop / Skate Photos".into(),
        Some("error") => value["message"]
            .as_str()
            .unwrap_or("Photograph unavailable")
            .to_owned(),
        Some("mode") if value["enabled"] == true => {
            "Ready · photographs stay on this computer".into()
        }
        _ => state.status.clone(),
    };
    if state.events.len() < 32 {
        state.events.push_back((owner.clone(), value));
    }
}

fn setup_hud(mut commands: Commands) {
    commands.spawn((ViewfinderHud, GlobalZIndex(35), Node {
        display: Display::None, width:percent(100), height:percent(100), position_type:PositionType::Absolute,
        flex_direction:FlexDirection::Column, justify_content:JustifyContent::SpaceBetween, align_items:AlignItems::Center,
        padding:UiRect::all(px(32)), ..default()
    })).with_children(|root| {
        root.spawn((Text::new("CAMERA  /  1920 × 1080"), TextFont{font_size:20.,..default()}, TextColor(Color::WHITE),
            BackgroundColor(Color::srgba(0.01,0.02,0.03,0.8)), Node{padding:UiRect::all(px(12)),..default()}));
        root.spawn((Text::new("+"),TextFont{font_size:36.,..default()},TextColor(Color::srgba(1.,1.,1.,0.7))));
        root.spawn((Node{flex_direction:FlexDirection::Column,row_gap:px(8),align_items:AlignItems::Center,padding:UiRect::all(px(16)),..default()},BackgroundColor(Color::srgba(0.01,0.02,0.03,0.85))))
            .with_children(|panel| {
                panel.spawn((ViewfinderStatus,Text::new(""),TextFont{font_size:18.,..default()},TextColor(Color::WHITE)));
                panel.spawn((Text::new("X / F12  Photograph     Right stick / Arrows  Frame     Triggers / PgUp–PgDn  Zoom     B / Esc  Back"),TextFont{font_size:15.,..default()},TextColor(Color::srgb(0.83,0.88,0.91))));
            });
    });
}

fn draw_hud(
    state: Res<Photos>,
    mut roots: Query<&mut Node, With<ViewfinderHud>>,
    mut labels: Query<&mut Text, With<ViewfinderStatus>>,
) {
    for mut root in &mut roots {
        root.display = if state.mode.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut label in &mut labels {
        **label = state.status.clone();
    }
}

pub(super) fn apply(
    world: &mut World,
    mods: &mut Mods,
    id: &str,
    generation: u64,
    operation: Operation,
) -> Result<(), String> {
    let owner = Owner {
        id: id.into(),
        generation,
    };
    if !owner.live(mods) || !operation.validate() {
        return Err("Photo request has a stale owner or invalid arguments.".into());
    }
    let result = (|| {
        match operation {
            Operation::Mode { enabled: true } => {
                may_enter(id, super::browser::focused_owner(mods))?;
                if world
                    .resource::<Photos>()
                    .mode
                    .as_ref()
                    .is_some_and(|mode| mode.owner != owner)
                {
                    return Err("Another interface owns the viewfinder. Close it first.".into());
                }
                if mods
                    .camera
                    .owner
                    .as_deref()
                    .is_some_and(|other| other != id)
                {
                    return Err("Another resource owns the scene camera.".into());
                }
                // Validate local output before inviting the user to press the shutter.
                skate_platform::photos::PhotoDirectory::resolve()?;
                let (transform, fov) = world
                .query_filtered::<(&Transform, &Projection), With<crate::camera::GameplayCamera>>()
                .iter(world)
                .find_map(|(transform, projection)| {
                    if let Projection::Perspective(p) = projection {
                        Some((*transform, p.fov))
                    } else {
                        None
                    }
                })
                .ok_or("Scene camera is not ready.")?;
                let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
                let mut state = world.resource_mut::<Photos>();
                state.mode = Some(Viewfinder {
                    owner: owner.clone(),
                    transform,
                    fov,
                    yaw,
                    pitch,
                });
                event(
                    &mut state,
                    &owner,
                    json!({"kind":"mode","enabled":true,"width":WIDTH,"height":HEIGHT}),
                );
            }
            Operation::Mode { enabled: false } => {
                let mut state = world.resource_mut::<Photos>();
                if state.mode.as_ref().is_some_and(|mode| mode.owner == owner) {
                    state.mode = None;
                }
                event(&mut state, &owner, json!({"kind":"mode","enabled":false}));
            }
            Operation::Gallery => {
                let mut state = world.resource_mut::<Photos>();
                let entries: Vec<_> = state
                    .gallery
                    .get(&owner)
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|photo| json!({"id":photo.id,"width":WIDTH,"height":HEIGHT,"saved":true}))
                    .collect();
                event(
                    &mut state,
                    &owner,
                    json!({"kind":"gallery","photos":entries,"limit":MAX_GALLERY}),
                );
            }
            Operation::Thumbnail { id } => {
                let mut state = world.resource_mut::<Photos>();
                let thumbnail = state
                    .gallery
                    .get(&owner)
                    .and_then(|photos| photos.iter().find(|photo| photo.id == id))
                    .map(|photo| photo.thumbnail.clone())
                    .ok_or("Photo is not in this interface's local gallery.")?;
                event(
                    &mut state,
                    &owner,
                    json!({"kind":"thumbnail","id":id,"data":thumbnail}),
                );
            }
        }
        Ok::<(), String>(())
    })();
    // Missing Desktop, busy camera and expired thumbnails are ordinary UI errors,
    // not resource failures: preserve the phone and deliver a usable error state.
    if let Err(message) = result {
        event(
            &mut world.resource_mut::<Photos>(),
            &owner,
            json!({"kind":"error","message":message}),
        );
    }
    Ok(())
}

fn may_enter(owner: &str, focused_owner: Option<&str>) -> Result<(), String> {
    if focused_owner != Some(owner) {
        return Err("Open and focus this interface before entering its camera.".into());
    }
    Ok(())
}

/// Called only by the trusted native input owner. No resource command calls this.
/// `look` is radians this frame; `zoom` is a signed FOV delta in radians.
pub(super) fn input(world: &mut World, mods: &Mods, shutter: bool, look: [f32; 2], zoom: f32) {
    let mut request = None;
    {
        let mut state = world.resource_mut::<Photos>();
        let Some(mode) = state.mode.as_mut() else {
            return;
        };
        if !mode.owner.live(mods)
            || super::browser::focused_owner(mods) != Some(mode.owner.id.as_str())
        {
            state.mode = None;
            return;
        }
        if look.iter().all(|value| value.is_finite()) && zoom.is_finite() {
            mode.yaw -= look[0].clamp(-0.2, 0.2);
            mode.pitch = (mode.pitch - look[1].clamp(-0.2, 0.2)).clamp(-1.45, 1.45);
            mode.transform.rotation = Quat::from_euler(EulerRot::YXZ, mode.yaw, mode.pitch, 0.);
            mode.fov =
                (mode.fov + zoom.clamp(-0.1, 0.1)).clamp(20_f32.to_radians(), 90_f32.to_radians());
        }
        if shutter {
            request = Some((mode.owner.clone(), mode.transform, mode.fov));
        }
    }
    if let Some((owner, transform, fov)) = request {
        if let Err(error) = start_capture(world, owner.clone(), transform, fov) {
            event(
                &mut world.resource_mut::<Photos>(),
                &owner,
                json!({"kind":"error","message":error}),
            );
        }
    }
}

fn present(
    state: Res<Photos>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<crate::camera::GameplayCamera>>,
) {
    let Some(mode) = &state.mode else {
        return;
    };
    for (mut transform, mut projection) in &mut cameras {
        *transform = mode.transform;
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = mode.fov;
        }
    }
}

fn may_capture(pending: bool, last: Option<Instant>, now: Instant) -> Result<(), String> {
    if pending {
        return Err("A photo is still saving. Wait for its result.".into());
    }
    if last.is_some_and(|last| now.saturating_duration_since(last) < SHUTTER_INTERVAL) {
        return Err("Wait one second between photographs.".into());
    }
    Ok(())
}

fn start_capture(
    world: &mut World,
    owner: Owner,
    transform: Transform,
    fov: f32,
) -> Result<(), String> {
    let now = Instant::now();
    let state = world.resource::<Photos>();
    may_capture(state.pending.is_some(), state.last_shutter, now)?;
    let env = super::capture::gameplay_environment(world);
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            WIDTH,
            HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    let mut camera = world.spawn((
        Camera3d::default(),
        Camera {
            order: -9,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov,
            aspect_ratio: WIDTH as f32 / HEIGHT as f32,
            ..default()
        }),
        transform,
        RenderTarget::Image(image.clone().into()),
        Msaa::Off,
        env.layers
            .unwrap_or_default()
            .without(super::capture::SCREEN_LAYER),
    ));
    if env.hdr {
        camera.insert((Hdr, bevy::core_pipeline::tonemapping::Tonemapping::None));
        if let Some(tone) = env.tone {
            camera.insert(tone);
        }
    }
    let camera = camera.id();
    let mut state = world.resource_mut::<Photos>();
    state.sequence += 1;
    let id = format!("{}-{}", std::process::id(), state.sequence);
    state.last_shutter = Some(now);
    event(&mut state, &owner, json!({"kind":"saving","id":id}));
    state.pending = Some(Job {
        owner,
        id,
        started: now,
        cancelled: Arc::new(AtomicBool::new(false)),
        camera: Some(camera),
        image: Some(image),
        screenshot: None,
        phase: Phase::Waiting(2),
    });
    Ok(())
}

fn dispose_targets(world: &mut World, job: &mut Job) {
    if let Some(entity) = job.camera.take() {
        world.despawn(entity);
    }
    if let Some(entity) = job.screenshot.take() {
        let _ = world.despawn(entity);
    }
    if let Some(image) = job.image.take() {
        world.resource_mut::<Assets<Image>>().remove(image.id());
    }
}

fn encode_photo(image: Image) -> Result<(Vec<u8>, String), String> {
    if image.width() != WIDTH || image.height() != HEIGHT {
        return Err("Photo renderer returned unexpected dimensions.".into());
    }
    let rgb = image
        .try_into_dynamic()
        .map_err(|_| "Photo renderer returned an unsupported pixel format.")?
        .to_rgb8();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb.clone())
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Cannot encode this photograph.")?;
    let thumbnail = image::imageops::thumbnail(&rgb, 160, 90);
    let mut jpeg = Vec::new();
    for quality in [65, 45, 25] {
        jpeg.clear();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode_image(&thumbnail)
            .map_err(|_| "Cannot create the gallery thumbnail.")?;
        if jpeg.len() <= MAX_THUMBNAIL {
            break;
        }
    }
    if jpeg.len() > MAX_THUMBNAIL {
        return Err("Thumbnail exceeds its bounded cache limit.".into());
    }
    Ok((
        png.into_inner(),
        format!("data:image/jpeg;base64,{}", base64(&jpeg)),
    ))
}

fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i > chunk.len() {
                '='
            } else {
                DIGITS[((value >> (18 - i * 6)) & 63) as usize] as char
            });
        }
    }
    out
}

fn save_worker(image: Image, id: String, cancelled: Arc<AtomicBool>) -> Result<Saved, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Photo cancelled.".into());
    }
    let (png, thumbnail) = encode_photo(image)?;
    let directory = skate_platform::photos::PhotoDirectory::resolve()?;
    let path = directory.write_png(&png, &cancelled)?;
    Ok(Saved {
        photo: Photo { id, thumbnail },
        path,
    })
}

/// Service bounded GPU/readback/worker completions and deliver current-generation events.
pub(super) fn poll(world: &mut World, mods: &mut Mods) {
    world.resource_scope(|world, mut state: Mut<Photos>| {
        if state.mode.as_ref().is_some_and(|mode| !mode.owner.live(mods)) { state.mode = None; }
        state.gallery.retain(|owner, _| owner.live(mods));
        if let Some(mut job) = state.pending.take() {
            let live = job.owner.live(mods) && !job.cancelled.load(Ordering::Acquire);
            if !live { job.cancelled.store(true, Ordering::Release); dispose_targets(world, &mut job); }
            let mut keep = live || matches!(job.phase, Phase::Saving(_));
            match &mut job.phase {
                Phase::Waiting(frames) if live => {
                    *frames = frames.saturating_sub(1);
                    if *frames == 0 {
                        let id = job.id.clone();
                        let owner = job.owner.clone();
                        let entity = world.spawn(Screenshot::image(job.image.as_ref().unwrap().clone()))
                            .observe(move |capture: On<ScreenshotCaptured>, mut state: ResMut<Photos>| {
                                if let Some(job) = state.pending.as_mut().filter(|job| job.id == id && job.owner == owner && matches!(job.phase, Phase::Readback) && !job.cancelled.load(Ordering::Acquire)) {
                                    job.phase = Phase::Captured(capture.image.clone());
                                    job.screenshot = None;
                                }
                            }).id();
                        job.screenshot = Some(entity);
                        job.phase = Phase::Readback;
                    }
                }
                Phase::Captured(_) if live => {
                    dispose_targets(world, &mut job);
                    let Phase::Captured(image) = std::mem::replace(&mut job.phase, Phase::Readback) else { unreachable!() };
                    let (send, receive) = mpsc::sync_channel(1);
                    let id = job.id.clone();
                    let cancelled = job.cancelled.clone();
                    let spawn = std::thread::Builder::new().name("skate-photo-save".into()).spawn(move || {
                        let result = save_worker(image, id, cancelled.clone());
                        if cancelled.load(Ordering::Acquire) {
                            if let Ok(saved) = &result { let _ = std::fs::remove_file(&saved.path); }
                        }
                        if let Err(error) = send.send(result) {
                            if let Ok(saved) = error.0 { let _ = std::fs::remove_file(saved.path); }
                        }
                    });
                    if spawn.is_ok() { job.phase = Phase::Saving(Mutex::new(receive)); }
                    else { keep = false; event(&mut state, &job.owner, json!({"kind":"error","message":"Cannot start the photo writer."})); }
                }
                Phase::Saving(receive) => {
                    match receive.lock().unwrap().try_recv() {
                        Ok(result) => {
                            keep = false;
                            match result {
                                Ok(saved) if live => {
                                    info!("PHOTO_SAVED {} {}x{}", saved.path.display(), WIDTH, HEIGHT);
                                    let id = saved.photo.id.clone();
                                    insert_photo(&mut state.gallery, job.owner.clone(), saved.photo);
                                    event(&mut state, &job.owner, json!({"kind":"saved","id":id,"width":WIDTH,"height":HEIGHT,"destination":"Desktop / Skate Photos"}));
                                }
                                Ok(saved) => { let _ = std::fs::remove_file(saved.path); }
                                Err(error) if live => event(&mut state, &job.owner, json!({"kind":"error","message":error})),
                                Err(_) => (),
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => (),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            keep = false;
                            if live { event(&mut state, &job.owner, json!({"kind":"error","message":"Photo writer stopped unexpectedly."})); }
                        }
                    }
                }
                _ => (),
            }
            if !matches!(job.phase, Phase::Saving(_)) && job.started.elapsed() > READBACK_TIMEOUT {
                keep = false;
                if live { event(&mut state, &job.owner, json!({"kind":"error","message":"Photo renderer timed out. Close and reopen the camera."})); }
            }
            if keep { state.pending = Some(job); } else { job.cancelled.store(true, Ordering::Release); dispose_targets(world, &mut job); }
        }
        for (owner, value) in state.events.drain(..) {
            if owner.live(mods) {
                if let Some(host) = mods.manager.resources.as_mut() { let _ = host.host_event(&owner.id, owner.generation, "photo", value); }
            }
        }
    });
}

fn insert_photo(gallery: &mut BTreeMap<Owner, VecDeque<Photo>>, owner: Owner, photo: Photo) {
    let photos = gallery.entry(owner.clone()).or_default();
    if photos.len() >= MAX_GALLERY {
        photos.pop_front();
    }
    photos.push_back(photo);
    while gallery.values().map(VecDeque::len).sum::<usize>() > MAX_TOTAL_GALLERY {
        let key = gallery
            .keys()
            .find(|key| **key != owner)
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if let Some(photos) = gallery.get_mut(&key) {
            photos.pop_front();
            if photos.is_empty() {
                gallery.remove(&key);
            }
        }
    }
}

pub(super) fn clear_owner(world: &mut World, id: &str) {
    if !world.contains_resource::<Photos>() {
        return;
    }
    world.resource_scope(|world, mut state: Mut<Photos>| {
        if state.mode.as_ref().is_some_and(|mode| mode.owner.id == id) {
            state.mode = None;
        }
        state.gallery.retain(|owner, _| owner.id != id);
        state.events.retain(|(owner, _)| owner.id != id);
        if state.pending.as_ref().is_some_and(|job| job.owner.id == id) {
            let mut job = state.pending.take().unwrap();
            job.cancelled.store(true, Ordering::Release);
            dispose_targets(world, &mut job);
            // Retain the global slot until the bounded writer actually exits.
            if matches!(job.phase, Phase::Saving(_)) {
                state.pending = Some(job);
            }
        }
    });
}

pub(super) fn clear(world: &mut World) {
    let Some(state) = world.get_resource::<Photos>() else {
        return;
    };
    let mut owners: Vec<_> = state.gallery.keys().map(|owner| owner.id.clone()).collect();
    owners.extend(state.mode.as_ref().map(|mode| mode.owner.id.clone()));
    owners.extend(state.pending.as_ref().map(|job| job.owner.id.clone()));
    for owner in owners {
        clear_owner(world, &owner);
    }
    world.resource_mut::<Photos>().events.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_or_other_resource_cannot_arm_camera() {
        assert!(may_enter("camera", None).is_err());
        assert!(may_enter("camera", Some("other")).is_err());
        assert!(may_enter("camera", Some("camera")).is_ok());
    }
    #[test]
    fn shutter_limits_queue_and_rate_independently() {
        let now = Instant::now();
        assert!(may_capture(true, None, now).is_err());
        assert!(may_capture(false, Some(now), now).is_err());
        assert!(may_capture(false, Some(now - SHUTTER_INTERVAL), now).is_ok());
        assert!(may_capture(false, None, now).is_ok());
    }
    #[test]
    fn gallery_is_owner_scoped_and_bounded() {
        let mut gallery = BTreeMap::new();
        for generation in 1..=4 {
            let owner = Owner {
                id: "sample".into(),
                generation,
            };
            for i in 0..50 {
                insert_photo(
                    &mut gallery,
                    owner.clone(),
                    Photo {
                        id: format!("{generation}-{i}"),
                        thumbnail: "small".into(),
                    },
                );
            }
        }
        assert!(gallery.values().all(|photos| photos.len() <= MAX_GALLERY));
        assert_eq!(
            gallery.values().map(VecDeque::len).sum::<usize>(),
            MAX_TOTAL_GALLERY
        );
        assert!(
            gallery[&Owner {
                id: "sample".into(),
                generation: 4
            }]
                .iter()
                .all(|photo| photo.id.starts_with("4-"))
        );
    }
    #[test]
    fn thumbnails_encode_correctly_and_fit_bridge() {
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(&[]), "");
        assert!(base64(&[0; MAX_THUMBNAIL]).len() + 256 < skate_browser::MAX_MESSAGE);
    }
    #[test]
    fn actual_png_dimensions_pixels_and_thumbnail_are_valid() {
        let rgb = image::RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 91])
        });
        let input =
            Image::from_dynamic(image::DynamicImage::ImageRgb8(rgb.clone()), true, default());
        let (png, thumbnail) = encode_photo(input).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgb8();
        assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
        assert_eq!(decoded.get_pixel(171, 83), rgb.get_pixel(171, 83));
        assert!(thumbnail.starts_with("data:image/jpeg;base64,"));
        assert!(thumbnail.len() < 11 * 1024);
    }
    #[test]
    fn retirement_cancels_pending_target_and_drops_gallery() {
        let mut world = World::new();
        world.init_resource::<Photos>();
        world.init_resource::<Assets<Image>>();
        let owner = Owner {
            id: "sample".into(),
            generation: 2,
        };
        let camera = world.spawn_empty().id();
        let handle = world.resource_mut::<Assets<Image>>().add(Image::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut state = world.resource_mut::<Photos>();
        state.pending = Some(Job {
            owner: owner.clone(),
            id: "1".into(),
            started: Instant::now(),
            cancelled: cancelled.clone(),
            camera: Some(camera),
            image: Some(handle.clone()),
            screenshot: None,
            phase: Phase::Waiting(2),
        });
        state.gallery.insert(
            owner,
            VecDeque::from([Photo {
                id: "1".into(),
                thumbnail: "local".into(),
            }]),
        );
        clear_owner(&mut world, "sample");
        assert!(cancelled.load(Ordering::Acquire));
        assert!(world.resource::<Photos>().pending.is_none());
        assert!(world.resource::<Photos>().gallery.is_empty());
        assert!(world.get_entity(camera).is_err());
        assert!(!world.resource::<Assets<Image>>().contains(handle.id()));
    }

    #[test]
    fn retirement_keeps_busy_slot_until_writer_finishes() {
        let mut world = World::new();
        world.init_resource::<Photos>();
        world.init_resource::<Assets<Image>>();
        let (_, receive) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        world.resource_mut::<Photos>().pending = Some(Job {
            owner: Owner {
                id: "sample".into(),
                generation: 1,
            },
            id: "1".into(),
            started: Instant::now(),
            cancelled: cancelled.clone(),
            camera: None,
            image: None,
            screenshot: None,
            phase: Phase::Saving(Mutex::new(receive)),
        });
        clear_owner(&mut world, "sample");
        assert!(cancelled.load(Ordering::Acquire));
        assert!(world.resource::<Photos>().pending.is_some());
        assert!(
            may_capture(
                world.resource::<Photos>().pending.is_some(),
                None,
                Instant::now()
            )
            .is_err()
        );
    }

    #[test]
    fn wrong_readback_dimensions_cannot_be_exported() {
        assert!(encode_photo(Image::default()).is_err());
    }
}
