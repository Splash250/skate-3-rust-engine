//! Owned collections drive real grind acquisition and physical lifecycle on the
//! redistributable community park. No network, renderer or fake query host.
use super::*;
use crate::physics::{grind::Family, grind_host::LiveHost, player_input::grind::{PreContext,PostContext}};
use skate_core::{math::Vector3,physics::{drive_frames::RetailAffineTransform,grind_contact::truck_contacts}};

#[test]
#[ignore = "requires locally owned stock animation banks and collections via SKATE3_ASSET_ROOT"]
fn community_park_acquires_paired_trucks_enters_native_grind_and_retires() {
    let root=std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root=std::path::Path::new(&root);
    let assets=skate_data::GameAssets::load(root).unwrap();
    let graphs=crate::graph_runtime::StockGraphs::load(root,&assets).unwrap();
    let map=skate_data::skate_map::SkateMap::parse(include_bytes!("../../../../../resources/community-park/park.skate")).unwrap();
    let mode=crate::difficulty::Difficulty::Easy;
    let mut physics=GamePhysics::load_with_difficulty(root,Some(&map),mode).unwrap();
    // Position an upright board over the original authored rail, with actual
    // board rates along its tangent. Contacts and candidate are derived below.
    physics.board.set_transform(RetailAffineTransform {translation:Vector3::new(5.,0.85,0.),..RetailAffineTransform::IDENTITY});
    for body in physics.board.bodies_mut() {body.rates.linear_velocity=Vector3::new(0.,0.,4.);}
    let mut skater=SkaterRuntime::load(root,&graphs,&physics,mode.profile_key()).unwrap();
    initialize(&mut physics,&mut skater).unwrap();
    let board=crate::physics::solve::deck_frame(&physics.board);
    let provider=physics.grind_provider();
    let collections=crate::custom_difficulty::load_collections(root).unwrap();
    let trucks=truck_contacts(board,0,
        collections.float("physics_grinds","default","TruckToWheel").unwrap(),
        collections.float("physics_grinds","default","DeckCenterToTruck").unwrap(),provider.primitives());
    assert!(trucks.iter().all(Option::is_some),"actual authored rail must intersect both native truck rectangles: {trucks:?}, board={board:?}");
    let p=&mut skater.player_input.processed;
    p.state_2508=PhysicalStateId::PhysicsGround as u32;p.category_2512=100;
    p.scalar_2652=4.;p.timestep_2604=1./120.;
    p.vectors_400_416[0]=[0.,0.,4.,0.].map(f32::to_bits);
    p.vectors_464_480_496_512_528[0]=board[0].map(f32::to_bits);
    p.vectors_544_560_592_608[0]=board[1].map(f32::to_bits);
    let mut host=LiveHost {board:&mut physics.board,settings:&mut physics.settings,materials:&physics.grind_materials};
    let pending=skater.player_input.grind.pre_update(p,&provider,&physics.world,PreContext {
        board,air_counter:20,tip_state:0,air_targeting_grind_9653:false,balance_2720:0.,translation_2796:0.,stability_nudge_2800:0.,up_down_2804:0.,grab_min_height_2808:0.,
    },&mut host).unwrap();
    let result=skater.player_input.grind.post_update(p,&physics.world,pending,PostContext {
        board,balance_2720:0.,translation_2796:0.,stability_nudge_2800:0.,up_down_2804:0.,grab_min_height_2808:0.,
    },&mut host).unwrap();
    assert!(p.grind.valid_1488,"native manager rejected authored rail: {:?}",skater.player_input.grind.investigation);
    assert_eq!(p.grind.family_1248,Family::FiftyFifty as u32);
    assert!(result.wipeout_reasons.is_empty());
    skater.grind.observe(result.observation);
    let snapshot=skater.player_input.processed_snapshot(physics.ticks);
    selection::advance(&mut physics,&mut skater,snapshot).unwrap();
    assert_eq!(skater.player_state.current(),PhysicalStateId::GrindFiftyFifty);
    assert_eq!(skater.grind.active_name(),Some("50-50"));
    crate::physics::grind::fill(&physics,&mut skater).unwrap();
    // Removing/reloading resource geometry takes the real native Exit path.
    physics.replace_grind_provider(&mut skater,std::sync::Arc::new(crate::grind_world::StaticProvider::authored(&[]).unwrap())).unwrap();
    assert_eq!(skater.player_state.current(),PhysicalStateId::PhysicsAir);
    assert!(skater.grind.active_name().is_none());
}
