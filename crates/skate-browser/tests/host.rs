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
