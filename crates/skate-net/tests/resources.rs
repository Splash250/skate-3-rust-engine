use skate_net::{
    dedicated::{Config, Server},
    lobby::{Info, Session},
    resources::*,
};
fn server() -> Server {
    Server::new(Config {
        session: 7,
        server_id: 99,
        map: 1,
        max_players: 16,
    })
    .unwrap()
}
fn client(id: u64) -> Session {
    Session::dedicated_client(
        7,
        Info {
            id,
            map: 1,
            rig: 2,
            physics: 3,
            appearance: 4,
        },
        1,
    )
}
fn pump(s: &mut Server, c: &mut Session, now: u64) {
    for p in c.service(now) {
        s.receive(10, &p.data, now);
    }
    for p in s.service(now) {
        if p.peer == 10 {
            c.receive(1, &p.data, now)
        }
    }
}
fn response(c: &Session) -> ServerRecord {
    serde_json::from_slice(&c.actors[&99].application[&server_key(c.local)].value).unwrap()
}
fn configure(s: &mut Server, revision: &str, generation: u64) {
    s.configure_resources(
        revision.into(),
        31030,
        [("challenge".into(), generation)].into(),
    )
    .unwrap();
}
#[test]
fn required_resources_gate_gameplay_and_derive_sender_from_connection() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    for t in (0..500).step_by(10) {
        pump(&mut s, &mut c, t)
    }
    assert!(!s.resource_ready(2));
    let r = response(&c);
    let mut resources = Client::default();
    resources.receive(&r).unwrap();
    resources.set_ready(true);
    resources
        .emit("challenge", 1, "join", serde_json::json!({"sender":999}))
        .unwrap();
    for t in (500..1500).step_by(10) {
        c.publish_application(CLIENT_KEY, resources.encode().unwrap(), t);
        pump(&mut s, &mut c, t);
        resources.receive(&response(&c)).unwrap();
    }
    assert!(s.resource_ready(2));
    let incoming = s.drain_resource_events();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].sender, 2);
    assert_eq!(incoming[0].message.name, "join");
}
#[test]
fn revision_update_and_stale_generation_cannot_affect_new_instance() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    let mut r = Client::default();
    for t in (0..500).step_by(10) {
        pump(&mut s, &mut c, t)
    }
    r.receive(&response(&c)).unwrap();
    r.set_ready(true);
    for t in (500..900).step_by(10) {
        c.publish_application(CLIENT_KEY, r.encode().unwrap(), t);
        pump(&mut s, &mut c, t);
        r.receive(&response(&c)).unwrap();
    }
    configure(&mut s, &"b".repeat(64), 2);
    assert!(!s.resource_ready(2));
    for t in (900..1200).step_by(10) {
        pump(&mut s, &mut c, t)
    }
    assert!(r.receive(&response(&c)).unwrap());
    assert!(!r.ready());
    r.set_ready(true);
    r.emit("challenge", 1, "stale", serde_json::Value::Null)
        .unwrap();
    for t in (1200..1800).step_by(10) {
        c.publish_application(CLIENT_KEY, r.encode().unwrap(), t);
        pump(&mut s, &mut c, t);
        r.receive(&response(&c)).unwrap();
    }
    assert!(s.drain_resource_events().is_empty());
    r.emit("challenge", 2, "new", serde_json::Value::Null)
        .unwrap();
    for t in (1800..2500).step_by(10) {
        c.publish_application(CLIENT_KEY, r.encode().unwrap(), t);
        pump(&mut s, &mut c, t);
        r.receive(&response(&c)).unwrap();
    }
    assert_eq!(s.drain_resource_events()[0].message.name, "new");
}
#[test]
fn reliable_resource_messages_are_bounded_and_delivered_once() {
    let mut channel = Client::default();
    let offer = Offer {
        revision: "a".repeat(64),
        port: 31030,
        epoch: 7,
    };
    channel
        .receive(&ServerRecord {
            offer,
            ack: 0,
            message: None,
        })
        .unwrap();
    channel.set_ready(true);
    assert!(
        channel
            .emit(
                "challenge",
                1,
                "oversize",
                serde_json::json!("x".repeat(400))
            )
            .is_err()
    );
    for _ in 0..MAX_PENDING {
        channel
            .emit("challenge", 1, "join", serde_json::Value::Null)
            .unwrap();
    }
    assert!(
        channel
            .emit("challenge", 1, "join", serde_json::Value::Null)
            .is_err()
    );
}
#[test]
fn application_allowlist_rejects_resource_state_or_spoofed_resource_keys() {
    let mut c = client(2);
    let valid = ClientRecord {
        epoch: 1,
        revision: "a".repeat(64),
        ready: true,
        ack: 0,
        message: Some(Message {
            id: 1,
            resource: "challenge".into(),
            generation: 1,
            kind: Kind::State,
            name: "score".into(),
            value: serde_json::json!(100),
        }),
    };
    assert!(!c.publish_application(CLIENT_KEY, serde_json::to_vec(&valid).unwrap(), 0));
    assert!(!c.publish_application(&server_key(2), b"{}".to_vec(), 0));
}

#[test]
fn state_snapshot_pages_all_keys_without_overflow_and_recovers_duplicate_records() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    let mut channel = Client::default();
    for i in 0..64 {
        s.send_resource(
            None,
            Message {
                id: 1,
                resource: "challenge".into(),
                generation: 1,
                kind: Kind::State,
                name: format!("key{i}"),
                value: serde_json::json!(i),
            },
        )
        .unwrap();
    }
    let mut states = std::collections::BTreeMap::new();
    for now in (0..15000).step_by(10) {
        if channel.offer().is_some() {
            channel.set_ready(true);
            c.publish_application(CLIENT_KEY, channel.encode().unwrap(), now);
        }
        for (i, p) in c.service(now).into_iter().enumerate() {
            if (now / 10 + i as u64) % 5 != 0 {
                s.receive(10, &p.data, now);
            }
        }
        for (i, p) in s.service(now).into_iter().enumerate() {
            if p.peer == 10 && (now / 10 + i as u64) % 7 != 0 {
                c.receive(1, &p.data, now);
                c.receive(1, &p.data, now);
            }
        }
        if c.actors
            .get(&99)
            .is_some_and(|a| a.application.contains_key(&server_key(2)))
        {
            let record = response(&c);
            channel.receive(&record).unwrap();
            channel.receive(&record).unwrap();
        }
        for m in channel.take_incoming() {
            assert!(
                states.insert(m.name, m.value).is_none(),
                "duplicate runtime delivery"
            );
        }
    }
    assert_eq!(states.len(), 64);
    assert_eq!(s.player_count(), 1);
}
#[test]
fn event_rate_limit_disconnects_without_allowing_payload_sender_spoof() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    for now in (0..500).step_by(10) {
        pump(&mut s, &mut c, now)
    }
    let offer = response(&c).offer;
    for seq in 1..=21 {
        let record = ClientRecord {
            epoch: offer.epoch,
            revision: offer.revision.clone(),
            ready: true,
            ack: 0,
            message: Some(Message {
                id: seq as u64,
                resource: "challenge".into(),
                generation: 1,
                kind: Kind::Event,
                name: "event".into(),
                value: serde_json::Value::Null,
            }),
        };
        let mut data = skate_net::packed::header(7, 2, skate_net::lobby::APPLICATION, seq);
        data.push(CLIENT_KEY.len() as u8);
        data.extend(CLIENT_KEY.as_bytes());
        data.extend(serde_json::to_vec(&record).unwrap());
        s.receive(10, &data, 500);
        s.service(500);
    }
    assert_eq!(s.player_count(), 0);
    assert_eq!(s.drain_resource_events().len(), 20);
}

#[test]
fn deleted_state_keys_release_quota_and_hot_keys_do_not_starve_others() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    for n in 0..70 {
        for value in [serde_json::json!(n), serde_json::Value::Null] {
            s.send_resource(
                None,
                Message {
                    id: 1,
                    resource: "challenge".into(),
                    generation: 1,
                    kind: Kind::State,
                    name: format!("old{n}"),
                    value,
                },
            )
            .unwrap();
        }
    }
    let mut c = client(2);
    let mut channel = Client::default();
    let mut found = false;
    s.send_resource(
        None,
        Message {
            id: 1,
            resource: "challenge".into(),
            generation: 1,
            kind: Kind::State,
            name: "z_cold".into(),
            value: serde_json::json!(1),
        },
    )
    .unwrap();
    for now in (0..3000).step_by(10) {
        s.send_resource(
            None,
            Message {
                id: 1,
                resource: "challenge".into(),
                generation: 1,
                kind: Kind::State,
                name: "a_hot".into(),
                value: serde_json::json!(now),
            },
        )
        .unwrap();
        if channel.offer().is_some() {
            channel.set_ready(true);
            c.publish_application(CLIENT_KEY, channel.encode().unwrap(), now);
        }
        pump(&mut s, &mut c, now);
        if c.actors
            .get(&99)
            .is_some_and(|a| a.application.contains_key(&server_key(2)))
        {
            channel.receive(&response(&c)).unwrap();
        }
        for message in channel.take_incoming() {
            if message.name == "z_cold" {
                found = true;
            }
        }
    }
    assert!(
        found,
        "a frequently updated key starved later replicated state"
    );
}
