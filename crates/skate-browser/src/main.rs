//! Standalone native webview. Launched only by the local engine, using private
//! stdin/stdout pipes. HTTP content is served by a closed custom scheme.
mod surface;
use skate_browser::{
    Assets, CSP, Event, Init, Input, MAX_INIT, MAX_MESSAGE, QUEUE, encode, read_record,
};
use std::{
    borrow::Cow,
    io::{BufReader, Write},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

fn emit(tx: &SyncSender<Event>, event: Event) {
    if tx.try_send(event).is_err() {
        std::process::exit(70);
    }
}
struct App {
    init: Init,
    assets: Arc<Assets>,
    input: Receiver<Input>,
    output: SyncSender<Event>,
    window: Option<Window>,
    view: Option<wry::WebView>,
    ping: Instant,
    pending: Arc<std::sync::atomic::AtomicBool>,
    last_frame: Instant,
    #[cfg(target_os = "linux")]
    offscreen: Option<gtk::OffscreenWindow>,
}
impl App {
    fn fail(&self, event_loop: &ActiveEventLoop, message: String) {
        emit(&self.output, Event::Error { message });
        event_loop.exit();
    }
    fn drain(&mut self, event_loop: &ActiveEventLoop) {
        for _ in 0..QUEUE {
            let input = match self.input.try_recv() {
                Ok(input) => input,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => {
                    event_loop.exit();
                    break;
                }
            };
            match input {
                Input::Close => {
                    emit(&self.output, Event::Closed);
                    event_loop.exit();
                    break;
                }
                Input::Frame => {
                    if self.init.options.surface.is_some()
                        && self.last_frame.elapsed() >= Duration::from_millis(30)
                    {
                        self.last_frame = Instant::now();
                        if let Some(view) = &self.view {
                            surface::snapshot(
                                view,
                                self.init.options.width,
                                self.init.options.height,
                                self.output.clone(),
                                self.pending.clone(),
                            );
                        }
                    }
                }
                Input::SurfaceInput { input } => {
                    if self.init.options.surface.is_some() && input.validate() {
                        if let Some(view) = &self.view {
                            let _ = view.evaluate_script(&format!(
                                "window.__skateInput({});",
                                serde_json::to_string(&input).unwrap()
                            ));
                        }
                    }
                }
                Input::Focus { focused } => {
                    if self.init.options.surface.is_some() {
                        emit(&self.output, Event::Focus { focused });
                        continue;
                    }
                    if let Some(window) = &self.window {
                        window.set_visible(focused);
                        if focused {
                            window.focus_window();
                            if let Some(view) = &self.view {
                                let _ = view.focus();
                            }
                        }
                        emit(&self.output, Event::Focus { focused });
                    }
                }
                Input::Message { value } => {
                    if let Some(view) = &self.view {
                        let data = serde_json::to_string(&value).unwrap();
                        if let Err(error) = view.evaluate_script(&format!(
                            "window.dispatchEvent(new MessageEvent('message',{{data:{data}}}));"
                        )) {
                            self.fail(event_loop, error.to_string());
                        }
                    }
                }
            }
        }
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title(self.init.title.clone())
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.init.options.width,
                    self.init.options.height,
                ))
                .with_visible(self.init.options.surface.is_none())
                .with_active(self.init.options.focus && self.init.options.surface.is_none()),
        ) {
            Ok(window) => window,
            Err(e) => {
                self.fail(event_loop, e.to_string());
                return;
            }
        };
        let assets = self.assets.clone();
        let navigation = self.assets.clone();
        let ipc = self.output.clone();
        let loaded = self.output.clone();
        let builder=wry::WebViewBuilder::new()
            .with_incognito(true).with_clipboard(false).with_devtools(false).with_autoplay(false)
            .with_transparent(self.init.options.surface.is_some())
            .with_focused(self.init.options.focus && self.init.options.surface.is_none()).with_general_autofill_enabled(false)
            .with_back_forward_navigation_gestures(false)
            .with_permission_handler(|_|wry::PermissionResponse::Deny)
            .with_new_window_req_handler(|_,_|wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_,_|false)
            .with_drag_drop_handler(|_|true)
            .with_navigation_handler(move |uri|navigation.get(&uri).is_some_and(|(_,mime)|mime.starts_with("text/html")))
            .with_custom_protocol("skate".into(),move|_,request|{
                let (status,mime,body)=if request.method()=="GET" {match assets.get(&request.uri().to_string()) {Some((bytes,mime))=>(200,mime,bytes.to_vec()),None=>(404,"text/plain",b"Resource asset unavailable".to_vec())}}
                    else{(405,"text/plain",b"Read-only resource assets".to_vec())};
                wry::http::Response::builder().status(status).header("Content-Type",mime).header("Content-Security-Policy",CSP)
                    .header("X-Content-Type-Options","nosniff").header("Cache-Control","no-store")
                    .header("Permissions-Policy","camera=(), microphone=(), geolocation=(), display-capture=(), clipboard-read=(), clipboard-write=()")
                    .body(Cow::Owned(body)).unwrap()
            })
            .with_initialization_script(include_str!("input.js"))
            .with_initialization_script("Object.defineProperty(window,'skate',{value:Object.freeze({postMessage:value=>window.ipc.postMessage(JSON.stringify(value))})});")
            .with_ipc_handler(move |request| {
                if request.body().len()+64>MAX_MESSAGE {emit(&ipc,Event::Error{message:"browser message exceeds byte budget".into()});return;}
                match serde_json::from_str(request.body()){Ok(value)=>emit(&ipc,Event::Message{value}),Err(_)=>emit(&ipc,Event::Error{message:"browser message must be bounded JSON".into()})}
            })
            .with_on_page_load_handler(move|event,_|{if matches!(event,wry::PageLoadEvent::Finished){emit(&loaded,Event::Ready);}})
            .with_url(format!("skate://ui/{}",self.init.options.entry));
        #[cfg(target_os = "linux")]
        let built = if self.init.options.surface.is_some() {
            use gtk::prelude::*;
            use wry::WebViewBuilderExtUnix;
            let offscreen = gtk::OffscreenWindow::new();
            offscreen.set_default_size(
                self.init.options.width as i32,
                self.init.options.height as i32,
            );
            let result = builder.build_gtk(&offscreen);
            offscreen.show_all();
            self.offscreen = Some(offscreen);
            result
        } else {
            builder.build(&window)
        };
        #[cfg(not(target_os = "linux"))]
        let built = builder.build(&window);
        match built {
            Ok(view) => {
                self.view = Some(view);
                self.window = Some(window);
            }
            Err(e) => self.fail(event_loop, e.to_string()),
        }
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
        self.drain(event_loop);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                emit(&self.output, Event::Closed);
                event_loop.exit();
            }
            WindowEvent::Focused(focused) if self.init.options.surface.is_none() => {
                emit(&self.output, Event::Focus { focused })
            }
            WindowEvent::Resized(size) if self.init.options.surface.is_none() => {
                if let Some(view) = &self.view {
                    let _ = view.set_bounds(wry::Rect {
                        position: wry::dpi::LogicalPosition::new(0., 0.).into(),
                        size: wry::dpi::PhysicalSize::new(size.width, size.height).into(),
                    });
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "linux")]
        {
            for _ in 0..256 {
                if !gtk::events_pending() {
                    break;
                }
                gtk::main_iteration_do(false);
            }
        }
        self.drain(event_loop);
        if self.ping.elapsed() >= Duration::from_secs(1) {
            self.ping = Instant::now();
            if let Some(view) = &self.view {
                let output = self.output.clone();
                let _ = view
                    .evaluate_script_with_callback("1", move |_| emit(&output, Event::Heartbeat));
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(10),
        ));
    }
}
fn run() -> Result<(), String> {
    let mut stdin = BufReader::new(std::io::stdin());
    let init: Init = serde_json::from_slice(
        &read_record(&mut stdin, MAX_INIT)?.ok_or("browser initialization missing")?,
    )
    .map_err(|e| e.to_string())?;
    let assets = Arc::new(Assets::load(&init)?);
    #[cfg(target_os = "linux")]
    gtk::init().map_err(|e| e.to_string())?;
    let mut builder = EventLoop::<()>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = builder.build().map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let (tx, input) = mpsc::sync_channel(QUEUE);
    std::thread::spawn(move || {
        loop {
            let message = match read_record(&mut stdin, MAX_MESSAGE) {
                Ok(Some(bytes)) => serde_json::from_slice::<Input>(&bytes).unwrap_or(Input::Close),
                _ => Input::Close,
            };
            let close = matches!(message, Input::Close);
            if tx.send(message).is_err() {
                break;
            }
            let _ = proxy.send_event(());
            if close {
                break;
            }
        }
    });
    let (output, rx) = mpsc::sync_channel::<Event>(QUEUE);
    let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let written = pending.clone();
    std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        while let Ok(event) = rx.recv() {
            let bytes = match encode(&event, MAX_MESSAGE) {
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            if stdout
                .write_all(&bytes)
                .and_then(|_| {
                    if let Event::Frame { rgba, .. } = &event {
                        stdout.write_all(rgba)
                    } else {
                        Ok(())
                    }
                })
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
            if matches!(event, Event::Frame { .. }) {
                written.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        std::process::exit(70);
    });
    event_loop
        .run_app(&mut App {
            init,
            assets,
            input,
            output,
            window: None,
            view: None,
            ping: Instant::now(),
            pending,
            last_frame: Instant::now(),
            #[cfg(target_os = "linux")]
            offscreen: None,
        })
        .map_err(|e| e.to_string())
}
fn main() {
    if let Err(message) = run() {
        let _ =
            std::io::stdout().write_all(&encode(&Event::Error { message }, MAX_MESSAGE).unwrap());
        std::process::exit(1);
    }
}
