#![cfg(feature = "host")]
use skate_browser::{Event, Init, Input, Options, process::Page};
use std::{
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(html: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-browser-real-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("index.html"), html).unwrap();
        Self(path)
    }
    fn page(&self) -> Page {
        Page::open(
            std::path::Path::new(env!("CARGO_BIN_EXE_skate-browser-host")),
            Init {
                root: self.0.clone(),
                title: "Resource browser verification".into(),
                options: Options {
                    entry: "index.html".into(),
                    files: vec!["index.html".into()],
                    width: 640,
                    height: 480,
                    focus: false,
                    surface: None,
                },
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn until(page: &mut Page, deadline: Duration, mut test: impl FnMut(&Event) -> bool) -> Event {
    let start = Instant::now();
    loop {
        let mut matched = None;
        for event in page.poll() {
            if test(&event) {
                matched = Some(event);
            } else if let Event::Error { message } = event {
                panic!("browser failed: {message}");
            }
        }
        if let Some(event) = matched {
            return event;
        }
        assert!(start.elapsed() < deadline, "browser event deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "requires a real Linux X11/Windows WebView2 desktop; run --features host --test host -- --ignored --test-threads=1"]
fn real_webview_exchanges_json_blocks_network_and_cleans_up() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let fixture = Fixture::new(&format!(
        r#"<!doctype html><html><body><h1>Inventory</h1><button id="buy">Buy board</button><script>
        window.addEventListener('message',event=>skate.postMessage({{echo:event.data}}));
        fetch('http://{}/private').then(()=>skate.postMessage({{network:'escaped'}})).catch(()=>skate.postMessage({{network:'blocked'}}));
        </script></body></html>"#,
        listener.local_addr().unwrap()
    ));
    let mut page = fixture.page();
    let mut blocked = false;
    until(&mut page, Duration::from_secs(20), |event| {
        if let Event::Message { value } = event {
            blocked |= value["network"] == "blocked";
        }
        matches!(event, Event::Ready)
    });
    page.send(&Input::Message {
        value: serde_json::json!({"coins":42,"items":["board"]}),
    })
    .unwrap();
    until(&mut page, Duration::from_secs(5), |event| {
        if let Event::Message { value } = event {
            blocked |= value["network"] == "blocked";
            return value["echo"]["coins"] == 42;
        }
        false
    });
    if !blocked {
        until(&mut page, Duration::from_secs(3), |event| {
            if let Event::Message { value } = event {
                blocked |= value["network"] == "blocked";
            }
            blocked
        });
    }
    assert!(
        listener.accept().is_err(),
        "web page contacted an ungranted network destination"
    );
    page.send(&Input::Focus { focused: true }).unwrap();
    until(&mut page, Duration::from_secs(3), |event| {
        matches!(event, Event::Focus { focused: true })
    });
    page.send(&Input::Focus { focused: false }).unwrap();
    until(&mut page, Duration::from_secs(3), |event| {
        matches!(event, Event::Focus { focused: false })
    });
    let pid = page.id();
    drop(page);
    #[cfg(target_os = "linux")]
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "browser host remained after teardown"
    );
}
#[test]
#[ignore = "requires a real desktop and intentional runaway renderer"]
fn renderer_runaway_times_out_without_stalling_parent() {
    let fixture = Fixture::new(
        "<!doctype html><h1>Runaway containment</h1><script>window.addEventListener('message',()=>{while(true){}});</script>",
    );
    let mut page = fixture.page();
    until(&mut page, Duration::from_secs(20), |e| {
        matches!(e, Event::Ready)
    });
    page.send(&Input::Message {
        value: serde_json::Value::Null,
    })
    .unwrap();
    let start = Instant::now();
    until(
        &mut page,
        Duration::from_secs(10),
        |e| matches!(e,Event::Error{message} if message.contains("stopped responding")),
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    drop(page);
}

#[test]
#[ignore = "requires a real desktop and delegated cgroup v2/WebView2 JobObject; intentionally exhausts the bounded browser budget"]
fn renderer_allocations_cannot_escape_the_process_tree_memory_budget() {
    let fixture = Fixture::new(
        r#"<!doctype html><html><body>Bounded memory fixture<script>
      const retained=[];
      setTimeout(()=>{setInterval(()=>{const value=new Uint8Array(64*1024*1024);value.fill(73);retained.push(value);},10)},1500);
    </script></body></html>"#,
    );
    let mut page = fixture.page();
    until(&mut page, Duration::from_secs(20), |event| {
        matches!(event, Event::Ready)
    });
    until(&mut page, Duration::from_secs(15), |event| {
        matches!(event, Event::Error { .. })
    });
    drop(page);
}

#[test]
#[ignore = "requires actual WebKitGTK/WebView2 graphical session and bounded process tree"]
fn composited_webview_renders_pixels_and_receives_host_input() {
    let fixture = Fixture::new(
        r#"<!doctype html><style>html,body{margin:0;background:transparent}button{position:absolute;left:10px;top:10px;width:160px;height:80px;background:rgb(240,40,20);border:0}input{position:absolute;left:10px;top:110px;width:160px}</style><button onclick="skate.postMessage({clicked:true})"><svg viewBox="0 0 40 40" style="position:absolute;left:0;top:0;width:40px;height:40px"><rect width="40" height="40" fill="rgb(240,40,20)"/></svg>Accept</button><input oninput="skate.postMessage({text:this.value})"><input type="number" value="12" style="top:170px" oninput="skate.postMessage({number:this.value})"><select style="position:absolute;left:10px;top:230px" onchange="skate.postMessage({choice:this.value})"><option value="first">First</option><option value="second">Second</option></select>"#,
    );
    let mut page = Page::open(
        std::path::Path::new(env!("CARGO_BIN_EXE_skate-browser-host")),
        Init {
            root: fixture.0.clone(),
            title: "Offscreen surface acceptance".into(),
            options: Options {
                entry: "index.html".into(),
                files: vec!["index.html".into()],
                width: 640,
                height: 480,
                focus: true,
                surface: Some(Default::default()),
            },
        },
    )
    .unwrap();
    until(&mut page, Duration::from_secs(20), |e| {
        matches!(e, Event::Ready)
    });
    let start = Instant::now();
    let frame = loop {
        for e in page.poll() {
            if let Event::Error { message } = e {
                panic!("surface renderer: {message}");
            }
        }
        if let Some(frame) = page.take_frame() {
            let pixel = &frame.rgba[(20 * 640 + 20) * 4..][..4];
            if pixel[0] > 220 && pixel[1] < 60 && pixel[2] < 40 {
                break frame;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no actual painted button pixels"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        (frame.width, frame.height, frame.rgba.len()),
        (640, 480, 640 * 480 * 4)
    );
    assert_eq!(
        frame.rgba[(400 * 640 + 500) * 4 + 3],
        0,
        "transparent surface background"
    );
    if let Some(path) = std::env::var_os("SKATE_BROWSER_TEST_CAPTURE") {
        image::save_buffer_with_format(
            path,
            &frame.rgba,
            640,
            480,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
    }
    page.send(&Input::SurfaceInput {
        input: skate_browser::SurfaceInput::Pointer {
            x: 30.,
            y: 30.,
            click: true,
        },
    })
    .unwrap();
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["clicked"]==true),
    );
    page.send(&Input::SurfaceInput {
        input: skate_browser::SurfaceInput::Navigate {
            direction: "down".into(),
        },
    })
    .unwrap();
    page.send(&Input::SurfaceInput {
        input: skate_browser::SurfaceInput::Text {
            text: "hello".into(),
        },
    })
    .unwrap();
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["text"]=="hello"),
    );
    for input in [
        skate_browser::SurfaceInput::Pointer {
            x: 30.,
            y: 180.,
            click: true,
        },
        skate_browser::SurfaceInput::Key {
            key: "SelectAll".into(),
            shift: false,
        },
        skate_browser::SurfaceInput::Text { text: "27".into() },
    ] {
        page.send(&Input::SurfaceInput { input }).unwrap();
    }
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["number"]=="27"),
    );
    page.send(&Input::SurfaceInput {
        input: skate_browser::SurfaceInput::Key {
            key: "Backspace".into(),
            shift: false,
        },
    })
    .unwrap();
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["number"]=="2"),
    );
    page.send(&Input::SurfaceInput {
        input: skate_browser::SurfaceInput::Navigate {
            direction: "right".into(),
        },
    })
    .unwrap();
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["number"]=="3"),
    );
    for input in [
        skate_browser::SurfaceInput::Pointer {
            x: 30.,
            y: 240.,
            click: true,
        },
        skate_browser::SurfaceInput::Navigate {
            direction: "right".into(),
        },
    ] {
        page.send(&Input::SurfaceInput { input }).unwrap();
    }
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["choice"]=="second"),
    );
    for _ in 0..8 {
        page.set_focus(false).unwrap();
        page.set_focus(true).unwrap();
    }
    let pid = page.id();
    drop(page);
    #[cfg(target_os = "linux")]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
#[ignore = "requires real graphical web engine; exercises the shipped phone package"]
fn shipped_phone_themes_render_and_controller_navigation_emits_real_app_routes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/phone");
    for theme in ["twilight", "paper"] {
        let mut page = Page::open(
            std::path::Path::new(env!("CARGO_BIN_EXE_skate-browser-host")),
            Init {
                root: root.clone(),
                title: "Packaged phone acceptance".into(),
                options: Options {
                    entry: "index.html".into(),
                    files: vec![
                        "index.html".into(),
                        "phone.css".into(),
                        "phone.js".into(),
                        "phone-sans.ttf".into(),
                    ],
                    width: 390,
                    height: 760,
                    focus: true,
                    surface: Some(Default::default()),
                },
            },
        )
        .unwrap();
        until(&mut page, Duration::from_secs(20), |e| {
            matches!(e, Event::Ready)
        });
        page.send(&Input::Message{value:serde_json::json!({"kind":"boot","preferences":{"theme":theme,"scale":1},"route":"home","calls":{"call":{"state":"idle"},"contacts":[],"voice":{"enabled":false}}})}).unwrap();
        until(
            &mut page,
            Duration::from_secs(5),
            |e| matches!(e,Event::Message{value} if value["action"]=="route" && value["route"]=="home"),
        );
        let start = Instant::now();
        let mut latest = None;
        while start.elapsed() < Duration::from_millis(700) {
            for event in page.poll() {
                if let Event::Error { message } = event {
                    panic!("{message}");
                }
            }
            if let Some(frame) = page.take_frame() {
                latest = Some(frame);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let frame = latest.expect("shipped phone must produce composited pixels");
        assert!(
            frame.rgba.chunks_exact(4).filter(|p| p[3] > 0).count() > 200_000,
            "phone content must occupy its actual frame"
        );
        if let Some(directory) = std::env::var_os("SKATE_BROWSER_PHONE_CAPTURES") {
            let path = PathBuf::from(directory).join(format!("phone-{theme}.png"));
            image::save_buffer_with_format(
                path,
                &frame.rgba,
                390,
                760,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .unwrap();
        }
        page.send(&Input::SurfaceInput {
            input: skate_browser::SurfaceInput::Navigate {
                direction: "accept".into(),
            },
        })
        .unwrap();
        until(
            &mut page,
            Duration::from_secs(5),
            |e| matches!(e,Event::Message{value} if value["action"]=="route" && value["route"]=="contacts"),
        );
        page.send(&Input::SurfaceInput {
            input: skate_browser::SurfaceInput::Navigate {
                direction: "back".into(),
            },
        })
        .unwrap();
        until(
            &mut page,
            Duration::from_secs(5),
            |e| matches!(e,Event::Message{value} if value["route"]=="home"),
        );
        std::thread::sleep(Duration::from_millis(50));
        for direction in ["right", "accept"] {
            page.send(&Input::SurfaceInput {
                input: skate_browser::SurfaceInput::Navigate {
                    direction: direction.into(),
                },
            })
            .unwrap();
        }
        until(
            &mut page,
            Duration::from_secs(5),
            |e| matches!(e,Event::Message{value} if value["action"]=="route" && value["route"]=="camera"),
        );
    }
}

#[test]
#[ignore = "requires real graphical web engine; verifies shipped dashboard async revocation"]
fn shipped_dashboard_discards_private_settings_reply_after_permission_revocation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/admin-dashboard");
    let html = std::fs::read_to_string(root.join("index.html")).unwrap();
    let probe = r#"<script>window.addEventListener('message',event=>{const v=event.data;if(v.verify==='select')document.querySelector('#resources button')?.click();if(v.verify==='probe')skate.postMessage({probe:document.getElementById('settings').textContent});});</script>"#;
    let fixture = Fixture::new(&html.replace("</body>", &format!("{probe}</body>")));
    for file in ["admin.css", "admin.js"] {
        std::fs::copy(root.join(file), fixture.0.join(file)).unwrap();
    }
    let mut page = Page::open(
        std::path::Path::new(env!("CARGO_BIN_EXE_skate-browser-host")),
        Init {
            root: fixture.0.clone(),
            title: "Dashboard privacy acceptance".into(),
            options: Options {
                entry: "index.html".into(),
                files: vec!["index.html".into(), "admin.css".into(), "admin.js".into()],
                width: 1120,
                height: 760,
                focus: true,
                surface: Some(Default::default()),
            },
        },
    )
    .unwrap();
    until(&mut page, Duration::from_secs(20), |e| {
        matches!(e, Event::Ready)
    });
    for value in [
        serde_json::json!({"kind":"result","key":"permissions","ok":true,"value":{"permissions":["status.read","settings.read"]}}),
        serde_json::json!({"kind":"result","key":"status","ok":true,"value":{"resources":[{"id":"private-plugin","running":true,"generation":1}]}}),
        serde_json::json!({"verify":"select"}),
    ] {
        page.send(&Input::Message { value }).unwrap();
    }
    let request = until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["request"]["kind"]=="settings_read"),
    );
    let key = match request {
        Event::Message { value } => value["key"].clone(),
        _ => unreachable!(),
    };
    for value in [
        serde_json::json!({"kind":"result","key":"permissions","ok":true,"value":{"permissions":["status.read"]}}),
        serde_json::json!({"kind":"result","key":key,"ok":true,"value":{"PRIVATE_SECRET":{"definition":{"type":"string","visibility":"private","change":"live"},"value":"sensitive"}}}),
        serde_json::json!({"verify":"probe"}),
    ] {
        page.send(&Input::Message { value }).unwrap();
    }
    until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["probe"]==""),
    );
}

#[test]
#[ignore = "requires real graphical web engine; preserves drafts and gallery across voice telemetry"]
fn shipped_phone_preserves_settings_drafts_and_gallery_nodes_across_voice_stats() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/phone");
    let html = std::fs::read_to_string(root.join("index.html")).unwrap();
    let probe = r#"<script>window.addEventListener('message',e=>{if(e.data.verify==='draft'){document.getElementById('bind-key').value='F7';document.getElementById('bind-hold').value='1000';}if(e.data.verify==='mark')window.savedGalleryNode=document.querySelector('.photo-grid');if(e.data.verify==='probe')skate.postMessage({probe:true,key:document.getElementById('bind-key')?.value,hold:document.getElementById('bind-hold')?.value,sameGallery:!!window.savedGalleryNode&&window.savedGalleryNode===document.querySelector('.photo-grid')});});</script>"#;
    let fixture = Fixture::new(&html.replace("</body>", &format!("{probe}</body>")));
    for name in ["phone.css", "phone.js", "phone-sans.ttf"] {
        std::fs::copy(root.join(name), fixture.0.join(name)).unwrap();
    }
    let mut page = Page::open(
        std::path::Path::new(env!("CARGO_BIN_EXE_skate-browser-host")),
        Init {
            root: fixture.0.clone(),
            title: "Phone telemetry regression".into(),
            options: Options {
                entry: "index.html".into(),
                files: vec![
                    "index.html".into(),
                    "phone.css".into(),
                    "phone.js".into(),
                    "phone-sans.ttf".into(),
                ],
                width: 390,
                height: 760,
                focus: true,
                surface: Some(Default::default()),
            },
        },
    )
    .unwrap();
    until(&mut page, Duration::from_secs(20), |e| {
        matches!(e, Event::Ready)
    });
    for value in [
        serde_json::json!({"kind":"apps","entries":[{"id":"phone/open","label":"Phone","binding":{"key":"KeyP","hold_ms":0}}]}),
        serde_json::json!({"kind":"boot","preferences":{"theme":"twilight","scale":1},"route":"settings","calls":{"call":{"state":"idle"},"voice":{"enabled":true,"stats":{"captured_frames":0}}}}),
        serde_json::json!({"verify":"draft"}),
        serde_json::json!({"kind":"calls","value":{"call":{"state":"idle"},"voice":{"enabled":true,"stats":{"captured_frames":10}}}}),
        serde_json::json!({"verify":"probe"}),
    ] {
        page.send(&Input::Message { value }).unwrap();
    }
    let reply = until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["probe"]==true),
    );
    assert!(matches!(reply,Event::Message{value} if value["key"]=="F7"&&value["hold"]=="1000"));
    for value in [
        serde_json::json!({"kind":"photo","value":{"kind":"gallery","photos":[{"id":"local-photo","width":1920,"height":1080}]}}),
        serde_json::json!({"kind":"route","route":"gallery"}),
        serde_json::json!({"verify":"mark"}),
        serde_json::json!({"kind":"calls","value":{"call":{"state":"idle"},"voice":{"enabled":true,"stats":{"captured_frames":20}}}}),
        serde_json::json!({"verify":"probe"}),
    ] {
        page.send(&Input::Message { value }).unwrap();
    }
    let reply = until(
        &mut page,
        Duration::from_secs(5),
        |e| matches!(e,Event::Message{value} if value["probe"]==true),
    );
    assert!(matches!(reply,Event::Message{value} if value["sameGallery"]==true));
}

#[test]
#[cfg(unix)]
#[ignore = "requires an isolated Linux graphical browser host"]
fn unexpected_host_exit_is_error_not_graceful_closed() {
    let fixture = Fixture::new("<!doctype html><html><body>Exit verification</body></html>");
    let mut page = fixture.page();
    until(&mut page, Duration::from_secs(20), |event| {
        matches!(event, Event::Ready)
    });
    // Killing the host closes stdout before or alongside child reaping. Neither
    // ordering may report graceful Closed and hide the renderer failure notice.
    assert_eq!(unsafe { libc::kill(page.id() as i32, libc::SIGKILL) }, 0);
    let start = Instant::now();
    loop {
        let events = page.poll();
        assert!(
            !events.iter().any(|event| matches!(event, Event::Closed)),
            "unexpected EOF was classified as graceful close"
        );
        if events
            .iter()
            .any(|event| matches!(event, Event::Error { .. }))
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "host exit was not classified"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(page);
    let mut page = fixture.page();
    until(&mut page, Duration::from_secs(20), |event| {
        matches!(event, Event::Ready)
    });
    page.send(&Input::Close).unwrap();
    until(&mut page, Duration::from_secs(5), |event| {
        matches!(event, Event::Closed)
    });
}
