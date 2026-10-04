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
        budgets: Budgets::default(),
        revision: "a".repeat(64),
        port: 31030,
        epoch: 7,
    };
    channel
        .receive(&ServerRecord {
            bulk: None,
            bulk_ack: Default::default(),
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
                serde_json::json!("x".repeat(MAX_PAYLOAD + 1))
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
        budgets: Budgets::default(),
        bulk: None,
        bulk_ack: Default::default(),
        epoch: 1,
        revision: "a".repeat(64),
        ready: true,
        ack: 0,
        message: Some(Message {
            scope: Default::default(),
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
                scope: Default::default(),
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
    for seq in 1..=Budgets::default().events_per_second + 1 {
        let record = ClientRecord {
            budgets: Budgets::default(),
            bulk: None,
            bulk_ack: Default::default(),
            epoch: offer.epoch,
            revision: offer.revision.clone(),
            ready: true,
            ack: 0,
            message: Some(Message {
                scope: Default::default(),
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
    assert_eq!(
        s.drain_resource_events().len(),
        Budgets::default().events_per_second as usize
    );
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
                    scope: Default::default(),
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
            scope: Default::default(),
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
                scope: Default::default(),
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

#[test]
fn large_resource_values_use_bounded_lane_and_deliver_once_in_both_directions() {
    let mut s = server();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    let mut wire = Client::default();
    for t in (0..500).step_by(10) {
        pump(&mut s, &mut c, t);
    }
    wire.receive(&response(&c)).unwrap();
    wire.set_ready(true);
    let value = serde_json::json!({"text":"λ".repeat(3000)});
    wire.emit("challenge", 1, "large", value.clone()).unwrap();
    let mut sent = false;
    let mut delivered = Vec::new();
    for t in (500..20_000).step_by(10) {
        let bytes = wire.encode().unwrap();
        assert!(bytes.len() <= skate_net::lobby::MAX_APP_VALUE);
        assert!(c.publish_application(CLIENT_KEY, bytes, t));
        pump(&mut s, &mut c, t);
        wire.receive(&response(&c)).unwrap();
        if s.resource_ready(2) && !sent {
            s.send_resource(
                None,
                Message {
                    scope: Default::default(),
                    id: 1,
                    resource: "challenge".into(),
                    generation: 1,
                    kind: Kind::State,
                    name: "inventory".into(),
                    value: value.clone(),
                },
            )
            .unwrap();
            sent = true;
        }
        delivered.extend(wire.take_incoming());
    }
    let incoming = s.drain_resource_events();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].sender, 2);
    assert_eq!(incoming[0].message.value, value);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].value, value);
    assert_eq!(s.player_count(), 1);
}

#[test]
fn negotiated_budgets_reject_oversize_before_queueing_and_state_remains_ordered() {
    let mut s = server();
    s.set_resource_budgets(Budgets {
        value_bytes: 1024,
        ..Default::default()
    })
    .unwrap();
    configure(&mut s, &"a".repeat(64), 1);
    let mut c = client(2);
    let mut wire = Client::default();
    for t in (0..500).step_by(10) {
        pump(&mut s, &mut c, t);
    }
    wire.receive(&response(&c)).unwrap();
    assert_eq!(wire.budgets().value_bytes, 1024);
    wire.set_ready(true);
    assert!(
        wire.emit("challenge", 1, "bad", serde_json::json!("x".repeat(1024)))
            .is_err()
    );
    let mut sent = 0;
    let mut values = Vec::new();
    for t in (500..6000).step_by(10) {
        c.publish_application(CLIENT_KEY, wire.encode().unwrap(), t);
        pump(&mut s, &mut c, t);
        wire.receive(&response(&c)).unwrap();
        values.extend(wire.take_incoming().into_iter().map(|m| m.value));
        if s.resource_ready(2) && sent < 2 {
            let value = if sent == 0 {
                serde_json::json!("x".repeat(900))
            } else {
                serde_json::json!("new")
            };
            s.send_resource(
                None,
                Message {
                    scope: Default::default(),
                    id: 1,
                    resource: "challenge".into(),
                    generation: 1,
                    kind: Kind::State,
                    name: "state".into(),
                    value,
                },
            )
            .unwrap();
            sent += 1;
        }
    }
    assert_eq!(values.last(), Some(&serde_json::json!("new")));
}

#[test]
fn incoming_small_and_bulk_messages_share_byte_budget_before_acknowledgement() {
    for include_bulk in [false, true] {
        let budgets = Budgets {
            value_bytes: 1024,
            queue_bytes: 2048,
            ..Default::default()
        };
        let mut client = Client::with_budgets(budgets).unwrap();
        let mut record = ServerRecord {
            offer: Offer {
                budgets,
                revision: "b".repeat(64),
                port: 31030,
                epoch: 1,
            },
            ack: 0,
            message: None,
            bulk: None,
            bulk_ack: Default::default(),
        };
        client.receive(&record).unwrap();
        client.set_ready(true);
        let message = |id, size| Message {
            scope: Default::default(),
            id,
            resource: "challenge".into(),
            generation: 1,
            kind: Kind::Event,
            name: "incoming".into(),
            value: serde_json::json!("x".repeat(size)),
        };
        if include_bulk {
            let mut sender = skate_net::bulk::Channel::default();
            sender
                .send(serde_json::to_vec(&message(1, 800)).unwrap())
                .unwrap();
            while let Some(frame) = sender.frame() {
                record.bulk = Some(frame);
                client.receive(&record).unwrap();
                let response: ClientRecord =
                    serde_json::from_slice(&client.encode().unwrap()).unwrap();
                sender.acknowledge(response.bulk_ack).unwrap();
            }
            record.bulk = None;
        }
        let mut accepted = 0;
        loop {
            record.message = Some(message(accepted + 1, 380));
            if client.receive(&record).is_err() {
                break;
            }
            accepted += 1;
            assert!(accepted < 10, "small messages bypassed the byte budget");
            // An already admitted retransmission is harmless even when full.
            client.receive(&record).unwrap();
        }
        let response: ClientRecord = serde_json::from_slice(&client.encode().unwrap()).unwrap();
        assert_eq!(response.ack, accepted, "rejected message was acknowledged");
        let incoming = client.take_incoming();
        assert_eq!(
            incoming.len(),
            accepted as usize + usize::from(include_bulk)
        );
        assert!(
            incoming
                .iter()
                .map(|m| serde_json::to_vec(m).unwrap().len())
                .sum::<usize>()
                <= budgets.queue_bytes
        );
        // Backpressure is retryable after the consumer drains admitted messages.
        client.receive(&record).unwrap();
        record.message = Some(message(accepted + 2, 500));
        assert!(
            client.receive(&record).is_err(),
            "large values bypassed the chunked lane"
        );
    }
}

#[test]
fn instance_transition_rotates_resource_activation_and_rejects_inflight_private_data() {
    let mut server = server();
    configure(&mut server, &"a".repeat(64), 1);
    let mut guest = client(2);
    let mut channel = Client::default();
    for now in (0..1000).step_by(10) {
        pump(&mut server, &mut guest, now);
        if guest
            .actors
            .get(&99)
            .is_some_and(|a| a.application.contains_key(&server_key(2)))
        {
            channel.receive(&response(&guest)).unwrap();
            channel.set_ready(true);
            guest.publish_application(CLIENT_KEY, channel.encode().unwrap(), now);
        }
    }
    assert!(server.resource_ready(2));
    for (scope, value) in [
        (Scope::Instance { id: 0 }, "old-room"),
        (Scope::Instance { id: 7 }, "new-room"),
    ] {
        server
            .send_resource(
                None,
                Message {
                    id: 1,
                    resource: "challenge".into(),
                    generation: 1,
                    scope,
                    kind: Kind::State,
                    name: "room".into(),
                    value: serde_json::json!(value),
                },
            )
            .unwrap();
    }
    server
        .send_resource(
            Some(2),
            Message {
                id: 1,
                resource: "challenge".into(),
                generation: 1,
                scope: Scope::Instance { id: 0 },
                kind: Kind::Event,
                name: "old-event".into(),
                value: serde_json::json!("x".repeat(8000)),
            },
        )
        .unwrap();
    for now in (1000..1100).step_by(10) {
        pump(&mut server, &mut guest, now);
    }
    let inflight = response(&guest);
    let old_epoch = inflight.offer.epoch;
    let epoch = server
        .teleport(
            2,
            skate_net::dedicated::TeleportDestination {
                position: [0., 1., 0.],
                heading: 0.,
                velocity: [0.; 3],
                instance: 7,
            },
            1100,
        )
        .unwrap();
    assert!(epoch > old_epoch);
    assert!(!server.resource_ready(2));
    let mut received = Vec::new();
    let mut reset_seen = false;
    for now in (1110..2200).step_by(10) {
        pump(&mut server, &mut guest, now);
        if guest.resource_epoch_floor() == epoch && !reset_seen {
            reset_seen = true;
            channel = Client::default(); // ClientResources retires the old VM/channel here.
            assert!(inflight.offer.epoch < guest.resource_epoch_floor());
        }
        let record = response(&guest);
        if record.offer.epoch < guest.resource_epoch_floor() {
            continue;
        }
        channel.receive(&record).unwrap();
        channel.set_ready(true);
        received.extend(channel.take_incoming());
        guest.publish_application(CLIENT_KEY, channel.encode().unwrap(), now);
    }
    assert!(reset_seen && server.resource_ready(2));
    assert_eq!(channel.offer().unwrap().epoch, epoch);
    assert_eq!(
        received.len(),
        1,
        "old state/events must not enter the new activation"
    );
    assert_eq!(received[0].scope, Scope::Instance { id: 7 });
    assert_eq!(received[0].value, "new-room");
    assert!(!channel.receive(&inflight).unwrap());
    assert!(channel.take_incoming().is_empty());
}

#[test]
fn hot_reload_never_pairs_new_incarnation_with_old_channel_acknowledgements() {
    let mut server=server();configure(&mut server,&"a".repeat(64),1);
    let mut guest=client(2);let mut channel=Client::default();let mut emitted=false;
    for now in (0..1000).step_by(10) {
        pump(&mut server,&mut guest,now);
        if guest.actors.get(&99).is_some_and(|a|a.application.contains_key(&server_key(2))) {
            channel.receive(&response(&guest)).unwrap();channel.set_ready(true);
            if !emitted {channel.emit("challenge",1,"before_reload",serde_json::json!({})).unwrap();emitted=true;}
            guest.publish_application(CLIENT_KEY,channel.encode().unwrap(),now);
        }
    }
    assert_eq!(response(&guest).ack,1);
    let mut incarnation=guest.resource_scope_epoch();
    configure(&mut server,&"b".repeat(64),2);
    let packets=server.service(1200);
    assert!(!packets.is_empty());
    for packet in packets {
        if packet.peer!=10 {continue;}
        guest.receive(1,&packet.data,1200);
        if incarnation!=guest.resource_scope_epoch() {
            incarnation=guest.resource_scope_epoch();channel=Client::default();
        }
        if let Some(record)=guest.actors.get(&99).and_then(|a|a.application.get(&server_key(2))) {
            let record:ServerRecord=serde_json::from_slice(&record.value).unwrap();
            channel.receive(&record).expect("new incarnation must never observe previous channel ack");
            assert_eq!(record.offer.revision,"b".repeat(64));
            assert_eq!(record.ack,0);
        }
    }
}
