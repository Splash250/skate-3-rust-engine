//! Actual loopback sockets, server simulation and two independent client
//! replicas. This is asset-free protocol/physics evidence, not graphical play.
use skate_net::{dedicated::{Config,Server},entities::{BodyType,Command,Definition,Shape,Spawn},lobby::{Info,Session},packed::{self,BodyState,Packed},Body,Pose};
use skate_server::entities::ServerWorld;
use std::{collections::BTreeMap,net::{Ipv4Addr,SocketAddr,SocketAddrV4,UdpSocket}};
fn endpoint(address:SocketAddr)->u64 {let SocketAddr::V4(a)=address else {panic!()};(u64::from(u32::from(*a.ip()))<<16)|u64::from(a.port())}
fn address(peer:u64)->SocketAddrV4 {SocketAddrV4::new(Ipv4Addr::from((peer>>16) as u32),peer as u16)}
fn socket()->UdpSocket {let s=UdpSocket::bind("127.0.0.1:0").unwrap();skate_net::socket::configure(&s).unwrap();s}
struct Client {socket:UdpSocket,session:Session}
impl Client {
    fn new(id:u64,host:u64)->Self {let socket=socket();let mut session=Session::dedicated_client(7,Info{id,map:1,rig:2,physics:3,appearance:4},host);session.set_loopback(true);Self{socket,session}}
    fn body(&mut self,x:f32,v:f32,now:u64) {
        let root=Pose{p:[x,0.,0.],q:[0.,0.,0.,1.]};
        let body=Body{pose:root,velocity:[v,0.,0.],angular:[0.;3]};
        self.session.publish(packed::BODY,Packed::body(&BodyState{root,enabled:(1<<33)-1,bodies:vec![body;33]}).unwrap(),now);
    }
}
fn pump(socket:&UdpSocket,server:&mut Server,world:&mut ServerWorld,clients:&mut [Client],now:u64) {
    pump_cadence(socket,server,world,clients,now,true,Some(0.01));
}
fn pump_cadence(socket:&UdpSocket,server:&mut Server,world:&mut ServerWorld,clients:&mut [Client],now:u64,client_tick:bool,server_dt:Option<f32>) {
    if client_tick {for client in clients.iter_mut() {for p in client.session.service(now) {client.socket.send_to(&p.data,address(p.peer)).unwrap();}}}
    let mut bytes=[0;65536];
    if let Some(dt)=server_dt {
        while let Ok((n,peer))=socket.recv_from(&mut bytes) {server.receive(endpoint(peer),&bytes[..n],now);}
        world.step(dt,server);
        for p in server.service(now) {assert!(p.data.len()<=packed::MTU);socket.send_to(&p.data,address(p.peer)).unwrap();}
    }
    let host=endpoint(socket.local_addr().unwrap());
    if client_tick {for client in clients {while let Ok((n,_))=client.socket.recv_from(&mut bytes) {client.session.receive(host,&bytes[..n],now);}}}
}
fn object(key:&str,position:[f32;3],kind:BodyType,half:[f32;3],controller:Option<u64>)->Command {
    Command::Spawn(Spawn{key:key.into(),instance:0,controller,definition:Definition{shape:Shape::Box{half_extents:half},body_type:kind,mass:5.,friction:0.7,color:[0.3,0.6,0.9,1.]},position,rotation:[0.,0.,0.,1.],velocity:[0.;3]})
}
#[test]
fn two_udp_clients_share_a_pushed_object_late_join_and_recover_a_controller_disconnect() {
    let udp=socket();let host=endpoint(udp.local_addr().unwrap());
    let mut server=Server::new(Config{session:7,server_id:99,map:1,max_players:16}).unwrap();
    let mut clients=vec![Client::new(2,host),Client::new(3,host)];let mut world=ServerWorld::default();
    world.sync_resources(BTreeMap::from([("objects".into(),1)]));
    for now in (0..200).step_by(10) {clients[0].body(0.,0.,now);clients[1].body(-10.,0.,now);pump(&udp,&mut server,&mut world,&mut clients,now);}
    assert_eq!(server.player_count(),2);
    let floor=world.command("objects",1,object("floor",[0.,-0.5,0.],BodyType::Static,[20.,0.5,20.],None),&mut server).unwrap();
    let id=world.command("objects",1,object("box",[1.5,0.5,0.],BodyType::Dynamic,[0.5;3],Some(2)),&mut server).unwrap();
    for now in (200..2200).step_by(10) {
        clients[0].body((now-200) as f32*0.002,2.,now);clients[1].body(-10.,0.,now);
        pump(&udp,&mut server,&mut world,&mut clients,now);
    }
    let first=&clients[0].session.entities.entities()[&id];let second=&clients[1].session.entities.entities()[&id];
    assert!(first.position[0]>2.5,"player contact must move server object: {:?}",first.position);
    assert!((first.position[0]-second.position[0]).abs()<0.3);
    let contacts=world.contact_metrics();
    assert!(contacts.pairs.iter().any(|pair|pair.entity==id && pair.actor==2 && pair.contact_steps>0),"physical push must expose real player/object solver contacts");
    assert!(!contacts.pairs.iter().any(|pair|pair.entity==id && pair.actor==3),"distant player must not inherit another player's contact");
    clients.push(Client::new(4,host));
    for now in (2200..3200).step_by(10) {clients[0].body(4.,0.,now);clients[1].body(-10.,0.,now);pump(&udp,&mut server,&mut world,&mut clients,now);}
    assert!(clients[2].session.entities.entities()[&id].position[0]>2.5);
    assert!(clients[2].session.entities.entities().contains_key(&floor),"late join must include static colliders");
    assert!(server.kick(2,3200));
    for now in (3210..3900).step_by(10) {pump(&udp,&mut server,&mut world,&mut clients,now);}
    assert_eq!(clients[1].session.entities.entities()[&id].controller,None);
    assert!(clients[1].session.entities.entities().contains_key(&floor),"disconnect must preserve shared static objects");
    assert!(clients[2].session.entities.entities().contains_key(&floor),"late join static objects survive another player leaving");
    world.sync_resources(BTreeMap::from([("objects".into(),2)]));
    for now in (3900..4500).step_by(10) {pump(&udp,&mut server,&mut world,&mut clients,now);}
    assert!(clients[1].session.entities.entities().is_empty());
    assert!(clients[2].session.entities.entities().is_empty());
    assert!(world.contact_metrics().pairs.is_empty(),"retired entity generations must release contact diagnostics");
}

#[test]
fn same_instance_teleports_republish_static_and_dynamic_objects_to_both_udp_clients() {
    for server_interval in [10,17,33] {
    let udp=socket();let host=endpoint(udp.local_addr().unwrap());
    let mut server=Server::new(Config{session:7,server_id:99,map:1,max_players:16}).unwrap();
    let mut clients=vec![Client::new(2,host),Client::new(3,host)];let mut world=ServerWorld::default();
    world.sync_resources(BTreeMap::from([("shared-objects".into(),1)]));
    for now in (0..200).step_by(10) {clients[0].body(-8.,0.,now);clients[1].body(-10.,0.,now);pump(&udp,&mut server,&mut world,&mut clients,now);}
    let floor=world.command("shared-objects",1,object("floor_0",[0.,-0.5,0.],BodyType::Static,[12.,0.5,12.],None),&mut server).unwrap();
    let crate_id=world.command("shared-objects",1,object("crate_0",[2.,0.65,0.],BodyType::Dynamic,[0.6;3],None),&mut server).unwrap();
    let mut portal=object("portal_0",[8.,1.,0.],BodyType::Static,[0.3;3],None);
    if let Command::Spawn(spawn)=&mut portal {spawn.definition.shape=Shape::Sphere{radius:0.3};}
    let portal=world.command("shared-objects",1,portal,&mut server).unwrap();
    let mut partial_samples=0;
    let mut partial_memberships=std::collections::BTreeSet::new();
    for now in 200..6000 {
        if now%500==0 {
            for (id,x) in [(2,-8.),(3,-10.)] {
                server.teleport(id,skate_net::dedicated::TeleportDestination{position:[x,0.,0.],heading:0.,velocity:[0.;3],instance:0},now).unwrap();
            }
        }
        if now%5==0 {for (client,x) in clients.iter_mut().zip([-8.,-10.]) {
            if let Some(reset)=client.session.pending_movement_reset() {client.session.complete_movement_reset(reset.epoch);}
            client.body(x,0.,now);
        }}
        pump_cadence(&udp,&mut server,&mut world,&mut clients,now,now%5==0,(now%server_interval==0).then_some(server_interval as f32/1000.));
        if now>=500 && now%5==0 && now%500<150 {
            partial_samples+=clients.iter().filter(|c|c.session.entities.entities().len()!=3).count();
            for client in &clients {
                let ids=client.session.entities.entities().keys().copied().collect::<Vec<_>>();
                if ids.len()!=3 {partial_memberships.insert(ids);}
            }
        }
        if now%5==0 && now%500>=150 && now>=350 {
            for client in &clients {
                assert_eq!(client.session.entities.entities().keys().copied().collect::<Vec<_>>(),[floor,crate_id,portal],"all objects must recover after teleport at {now} for {} with {server_interval}ms host/5ms client ticks",client.session.local);
            }
        }
    }
    eprintln!("{server_interval}ms host/5ms client: {partial_samples} transient incomplete samples inside150ms recovery windows {partial_memberships:?}; complete outside");
    }
}
