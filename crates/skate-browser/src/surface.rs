//! Platform pixel acquisition. These APIs capture only this packaged webview.
use skate_browser::Event;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
};

pub fn snapshot(
    view: &wry::WebView,
    width: u32,
    height: u32,
    output: SyncSender<Event>,
    pending: Arc<AtomicBool>,
) {
    if pending.swap(true, Ordering::AcqRel) {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        view.webview().snapshot(
            webkit2gtk::SnapshotRegion::Visible,
            webkit2gtk::SnapshotOptions::TRANSPARENT_BACKGROUND,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                let result = (|| -> Result<Vec<u8>, String> {
                    let source = result.map_err(|e| e.to_string())?;
                    let target = gtk::cairo::ImageSurface::create(
                        gtk::cairo::Format::ARgb32,
                        width as i32,
                        height as i32,
                    )
                    .map_err(|e| e.to_string())?;
                    let context = gtk::cairo::Context::new(&target).map_err(|e| e.to_string())?;
                    context
                        .set_source_surface(&source, 0., 0.)
                        .map_err(|e| e.to_string())?;
                    context.paint().map_err(|e| e.to_string())?;
                    target.flush();
                    let mut rgba = vec![0; skate_browser::frame_len(width, height)?];
                    let stride = target.stride() as usize;
                    target
                        .with_data(|data| {
                            for y in 0..height as usize {
                                for x in 0..width as usize {
                                    let i = y * stride + x * 4;
                                    let pixel =
                                        u32::from_ne_bytes(data[i..i + 4].try_into().unwrap());
                                    let a = (pixel >> 24) as u8;
                                    let out = &mut rgba[(y * width as usize + x) * 4..][..4];
                                    for (index, shift) in [16, 8, 0].into_iter().enumerate() {
                                        let c = (pixel >> shift) & 255;
                                        out[index] = if a == 0 {
                                            0
                                        } else {
                                            ((c * 255 + a as u32 / 2) / a as u32).min(255) as u8
                                        };
                                    }
                                    out[3] = a;
                                }
                            }
                        })
                        .map_err(|e| e.to_string())?;
                    Ok(rgba)
                })();
                let event = match result {
                    Ok(rgba) => Event::Frame {
                        width,
                        height,
                        rgba,
                    },
                    Err(message) => Event::Error {
                        message: format!("browser snapshot: {message}"),
                    },
                };
                if output.try_send(event).is_err() {
                    pending.store(false, Ordering::Release);
                }
            },
        );
    }
    #[cfg(windows)]
    {
        use webview2_com::{
            CapturePreviewCompletedHandler,
            Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
        };
        use windows::Win32::System::Com::{
            STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
        };
        use wry::WebViewExtWindows;
        // The stream belongs only to this bounded webview snapshot and is never
        // a resource-selected path or general screenshot handle.
        let stream = match unsafe { CreateStreamOnHGlobal(Default::default(), true) } {
            Ok(stream) => stream,
            Err(e) => {
                pending.store(false, Ordering::Release);
                let _ = output.try_send(Event::Error {
                    message: e.to_string(),
                });
                return;
            }
        };
        let captured = stream.clone();
        let failed = output.clone();
        let reset = pending.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |status| {
            let result = (|| -> Result<Vec<u8>, String> {
                status.map_err(|e| e.to_string())?;
                let mut stat = STATSTG::default();
                unsafe { captured.Stat(&mut stat, STATFLAG_NONAME) }.map_err(|e| e.to_string())?;
                if stat.cbSize > skate_browser::MAX_FRAME as u64 + 65536 {
                    return Err("browser PNG snapshot exceeds bound".into());
                }
                let mut png = vec![0; stat.cbSize as usize];
                let mut read = 0;
                unsafe { captured.Seek(0, STREAM_SEEK_SET, None) }.map_err(|e| e.to_string())?;
                unsafe {
                    captured.Read(png.as_mut_ptr().cast(), png.len() as u32, Some(&mut read))
                }
                .ok()
                .map_err(|e| e.to_string())?;
                if read as usize != png.len() {
                    return Err("incomplete browser PNG snapshot".into());
                }
                let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                if image.width() != width || image.height() != height {
                    return Err("browser snapshot dimensions changed".into());
                }
                Ok(image.into_rgba8().into_raw())
            })();
            let event = match result {
                Ok(rgba) => Event::Frame {
                    width,
                    height,
                    rgba,
                },
                Err(message) => Event::Error {
                    message: format!("browser snapshot: {message}"),
                },
            };
            if output.try_send(event).is_err() {
                pending.store(false, Ordering::Release);
            }
            Ok(())
        }));
        if let Err(e) = unsafe {
            view.webview().CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
        } {
            reset.store(false, Ordering::Release);
            let _ = failed.try_send(Event::Error {
                message: e.to_string(),
            });
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (view, width, height);
        pending.store(false, Ordering::Release);
        let _ = output.try_send(Event::Error {
            message: "composited browser surfaces support Linux and Windows".into(),
        });
    }
}
