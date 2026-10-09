//! Diagnostic: grind bails at the PCU Library rails (University), see
//! docs/hails-additions/30-grind-random-bails.md. Lists the grind splines near
//! SKATE3_GRIND_NEAR (default the library stairs), then drops the skater onto
//! each one (or SKATE3_GRIND_RAIL=<index>) moving along it and traces the
//! physical state until the first WipeoutGround.
//! SKATE3_ASSET_ROOT=<assets> SKATE3_GRIND_MAP=<University.skate>
//! [SKATE3_GRIND_SLIDE=1 boardslide approach] [SKATE3_GRIND_REVERSE=1]
//! [SKATE3_GRIND_LEAD=<m from the start>] [SKATE3_GRIND_SPEED=8]
//! [SKATE3_GRIND_TICKS=260] [SKATE3_GRIND_TRIS=x,y,z,r collision dump]
//! cargo test --release --bin skate3rust -- --ignored --nocapture grind_bail
use super::*;
use skate_core::player::state::PhysicalStateId;

fn rails_near(provider: &crate::grind_world::StaticProvider, c: [f32; 3], r: f32) -> Vec<(u64, Vec<[f32; 3]>)> {
    let hits = provider.query([c[0] - r, c[1] - 8., c[2] - r], [c[0] + r, c[1] + 8., c[2] + r]).unwrap();
    let mut out: Vec<(u64, Vec<[f32; 3]>)> = Vec::new();
    let mut hits = hits;
    hits.sort();
    for i in hits {
        let p = provider.primitives()[i];
        let (s, e) = ([p.start[0], p.start[1], p.start[2]], [p.end[0], p.end[1], p.end[2]]);
        match out.iter_mut().find(|(o, _)| *o == p.owner) {
            Some((_, pts)) => {
                if pts.last() != Some(&s) { pts.push(s); }
                pts.push(e);
            }
            None => out.push((p.owner, vec![s, e])),
        }
    }
    out
}

fn run(root: &std::path::Path, map: &skate_data::skate_map::SkateMap, start: [f32; 3], dir: [f32; 3], perpendicular: bool, speed: f32) -> (u32, Vec<String>) {
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_world(root, ground::Terrain::Course, Some(map)).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut log = Vec::new();
    let mut grind_ticks = 0;
    let mut bailed = 0;
    for tick in 0..std::env::var("SKATE3_GRIND_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(260) {
        if tick == 30 {
            // Forward axis (row 2) along the rail, or across it for a slide.
            let f = if perpendicular { [-dir[2], 0., dir[0]] } else { dir };
            let up = [0., 1., 0.];
            let right = [up[1] * f[2] - up[2] * f[1], up[2] * f[0] - up[0] * f[2], up[0] * f[1] - up[1] * f[0]];
            let transform = [
                [right[0], right[1], right[2], 0.],
                [0., 1., 0., 0.],
                [f[0], f[1], f[2], 0.],
                [start[0], start[1], start[2], 1.],
            ];
            skater.travel(transform, Some(dir.map(|x| x * speed))).unwrap();
        }
        if (31..=32).contains(&tick) {
            let v = skate_core::math::Vector3::new(dir[0] * speed, dir[1] * speed, dir[2] * speed);
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity = v;
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity = v;
            }
        }
        input.sample_raw_for_test(skate_core::input::xbox::XboxState { buttons: 0, triggers: [0; 2], left: [0; 2], right: [0; 2] });
        let mut actions = input.player_actions();
        controls.update(&mut actions, physics.settings.step.simulation.time_step, physics.settings.input_magnitude_threshold, skater.player_input.physical.scoring.capabilities_204);
        if std::env::var_os("SKATE_DEBUG_GRIND").is_some() {
            eprintln!("TICK {tick}");
        }
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera).unwrap_or_else(|e| panic!("tick {tick}: {e}"));
        let state = skater.player_state.current();
        let deck = physics.board.bodies()[skate_core::physics::board::BodyId::Deck.index()].rates;
        if (400..=405).contains(&(state as u32)) {
            grind_ticks += 1;
        }
        if matches!(state, PhysicalStateId::WipeoutGround) && bailed == 0 {
            bailed = tick;
        }
        if tick >= 30 {
            let v = deck.linear_velocity;
            log.push(format!(
                "t{tick:3} {:<16} deck ({:.2}, {:.2}, {:.2}) v ({:.2}, {:.2}, {:.2}) grind {:?}",
                format!("{state:?}"), deck.position.x, deck.position.y, deck.position.z, v.x, v.y, v.z, skater.grind.active_name()
            ));
        }
    }
    eprintln!("grind ticks {grind_ticks}, first wipeout tick {bailed}");
    (bailed, log)
}

#[test]
#[ignore = "requires private assets and a converted University map; diagnostic only"]
fn grind_bail_trace() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let map_path = std::env::var_os("SKATE3_GRIND_MAP").expect("set SKATE3_GRIND_MAP");
    let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&map_path)).unwrap();
    let centre: [f32; 3] = std::env::var("SKATE3_GRIND_NEAR")
        .ok()
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (p.len() == 3).then(|| [p[0], p[1], p[2]])
        })
        .unwrap_or([266., 74.5, -429.]);
    let listing = GamePhysics::load_with_world(root, ground::Terrain::Course, Some(&map)).unwrap();
    let rails = rails_near(&listing.grind_world, centre, 12.);
    if let Some(q) = std::env::var("SKATE3_GRIND_TRIS").ok().map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>()) {
        for t in listing.world.triangles() {
            let [a, b, c] = t.triangle.vertices;
            let m = [(a.x + b.x + c.x) / 3., (a.y + b.y + c.y) / 3., (a.z + b.z + c.z) / 3.];
            let near = [a, b, c].iter().any(|v| ((v.x - q[0]).powi(2) + (v.y - q[1]).powi(2) + (v.z - q[2]).powi(2)).sqrt() < q[3]);
            if near {
                let (u, w) = ([b.x - a.x, b.y - a.y, b.z - a.z], [c.x - a.x, c.y - a.y, c.z - a.z]);
                let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let n = skate_core::math::Vector3::new(n[0], n[1], n[2]);
                eprintln!("tri tag {:#x} centre ({:.3},{:.3},{:.3}) n ({:.2},{:.2},{:.2}) feature {:?} verts ({:.3},{:.3},{:.3}) ({:.3},{:.3},{:.3}) ({:.3},{:.3},{:.3})", t.tag, m[0], m[1], m[2], n.x / l, n.y / l, n.z / l, t.triangle.feature, a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z);
            }
        }
    }
    drop(listing);
    for (i, (owner, points)) in rails.iter().enumerate() {
        eprintln!("rail {i} owner {owner:#x} points {}: {:?}", points.len(), points);
    }
    let only: Option<usize> = std::env::var("SKATE3_GRIND_RAIL").ok().and_then(|v| v.parse().ok());
    let perpendicular = std::env::var("SKATE3_GRIND_SLIDE").is_ok_and(|v| v == "1");
    let speed: f32 = std::env::var("SKATE3_GRIND_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(8.);
    let reverse = std::env::var("SKATE3_GRIND_REVERSE").is_ok_and(|v| v == "1");
    for (i, (_, points)) in rails.iter().enumerate() {
        if only.is_some_and(|o| o != i) || points.len() < 2 {
            continue;
        }
        let (a, b) = if reverse { (points[points.len() - 1], points[0]) } else { (points[0], points[points.len() - 1]) };
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = d.map(|x| x / len);
        let lead: f32 = std::env::var("SKATE3_GRIND_LEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);
        let start = [a[0] + dir[0] * lead, a[1] + dir[1] * lead + 0.35, a[2] + dir[2] * lead];
        eprintln!("=== rail {i} start {start:?} dir {dir:?} slide {perpendicular} speed {speed}");
        let (bailed, log) = run(root, &map, start, dir, perpendicular, speed);
        for line in &log {
            eprintln!("{line}");
        }
        eprintln!("=== rail {i}: first wipeout tick {bailed}");
    }
}

/// Regression for doc 30: with the retail per-volume query box (82777E70,
/// tested per triangle in 8277BC58), the deck no longer gets a predictive
/// contact with the post below the bottom bend of the library handrail
/// (owner 0x688), so boardslide and 50-50 grind off the end without a wipeout.
/// The uphill boardslide on the same rail stays clean.
#[test]
#[ignore = "requires private assets and a converted University map"]
fn library_handrail_bend_grinds_through() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let map_path = std::env::var_os("SKATE3_GRIND_MAP").expect("set SKATE3_GRIND_MAP");
    let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&map_path)).unwrap();
    let listing = GamePhysics::load_with_world(root, ground::Terrain::Course, Some(&map)).unwrap();
    let rails = rails_near(&listing.grind_world, [266., 74.5, -429.], 12.);
    drop(listing);
    let points = &rails.iter().find(|(owner, _)| *owner == 0x688).expect("rail 0x688").1;
    for (reverse, slide, lead, grind) in [
        (true, true, 3., "GrindBoardslide"),
        (true, false, 3., "GrindFiftyFifty"),
        (false, true, 1.5, "GrindBoardslide"),
    ] {
        let (a, b) = if reverse { (points[points.len() - 1], points[0]) } else { (points[0], points[points.len() - 1]) };
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = d.map(|x| x / len);
        let start = [a[0] + dir[0] * lead, a[1] + dir[1] * lead + 0.35, a[2] + dir[2] * lead];
        let (bailed, log) = run(root, &map, start, dir, slide, 8.);
        let grind_ticks = log.iter().filter(|l| l.contains(grind)).count();
        assert!(grind_ticks >= 50, "reverse {reverse} slide {slide}: only {grind_ticks} {grind} ticks");
        assert_eq!(bailed, 0, "reverse {reverse} slide {slide}: wipeout at tick {bailed}");
    }
}
