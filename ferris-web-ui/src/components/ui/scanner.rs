//! Camera barcode/QR scanner.
//!
//! The scanner is the browser-side counterpart of Frappe's `frappe.ui.Scanner`:
//! a dialog with the camera feed, a callback per decoded value, and a `multiple`
//! switch that keeps scanning instead of closing after the first hit.
//!
//! Decoding goes through the platform's `BarcodeDetector` where it exists. There
//! is no bundled decoder: shipping a second WebAssembly image decoder for one
//! control would dominate the Desk bundle, and `BarcodeDetector` is the browser
//! API that already owns the camera pipeline. When it is absent the component
//! reports a typed message instead of pretending the camera found nothing.
//!
//! The camera is released on close, on scan completion and when the component
//! unmounts. A scanner that leaves the light on is a privacy bug, not a
//! cosmetic one.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

use super::button::{Button, ButtonSize, ButtonVariant};
use super::dialog::{Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle};

/// Seconds a decoded value is ignored after it was reported.
///
/// `BarcodeDetector` returns the same code on every frame while it is in view;
/// without a debounce a `multiple` scanner would emit it dozens of times per
/// second.
#[cfg(target_arch = "wasm32")]
const RESCAN_DELAY_MS: i32 = 700;

/// A camera scanner: a trigger button and a dialog with the live feed.
#[component]
pub fn Scanner(
    /// Called once per decoded value.
    on_scan: Callback<String>,
    /// Keep the dialog open and keep decoding after a hit.
    #[prop(default = false)]
    multiple: bool,
    /// Trigger label.
    #[prop(default = "Escanear".to_string(), into)]
    label: String,
    /// Accessible description shown under the title.
    #[prop(default = "Apunta la cámara al código.".to_string())]
    description: String,
) -> impl IntoView {
    // `on_scan`/`multiple` are only read by the client task; SSR still builds
    // the shell and must not report them as unused.
    let _ = (&on_scan, multiple);
    let open = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let video_ref = NodeRef::<leptos::html::Video>::new();

    // Start/stop with the dialog. The effect runs on the client only: SSR has
    // no camera and no `BarcodeDetector`, so the initial HTML is the dialog
    // shell and the first hydration pass does exactly nothing.
    #[cfg(target_arch = "wasm32")]
    {
        let on_scan = on_scan;
        Effect::new(move |_| {
            if !open.get() {
                return;
            }
            error.set(None);
            let Some(video) = video_ref.get() else {
                error.set(Some("El visor de la cámara no está montado".to_string()));
                return;
            };
            let video_for_task = video.clone();
            spawn_local(async move {
                // The element is attached in the same render pass; one frame
                // gives the browser time to create the media element.
                sleep(50).await;
                if let Err(message) = start_camera(&video_for_task).await {
                    error.set(Some(message));
                    return;
                }
                let detector = match detector() {
                    Ok(detector) => detector,
                    Err(message) => {
                        error.set(Some(message));
                        stop_camera(&video_for_task);
                        return;
                    }
                };
                loop {
                    if !open.get_untracked() {
                        break;
                    }
                    match detect_once(&detector, &video_for_task).await {
                        Ok(Some(code)) => {
                            on_scan.run(code);
                            if !multiple {
                                open.set(false);
                                break;
                            }
                            sleep(RESCAN_DELAY_MS).await;
                        }
                        Ok(None) => sleep(250).await,
                        Err(message) => {
                            error.set(Some(message));
                            break;
                        }
                    }
                }
                stop_camera(&video_for_task);
            });
        });
    }

    // Release the camera when the dialog closes for any reason (button, ESC,
    // backdrop): the effect above only observes `open`, so this one owns the
    // teardown that must happen even if the scan task never ran.
    #[cfg(target_arch = "wasm32")]
    {
        on_cleanup(move || {
            if let Some(video) = video_ref.get() {
                stop_camera(&video);
            }
        });
        Effect::new(move |_| {
            if open.get() {
                return;
            }
            if let Some(video) = video_ref.get() {
                stop_camera(&video);
            }
        });
    }

    view! {
        <Button
            variant=ButtonVariant::Outline
            size=ButtonSize::Default
            on:click=move |_| open.set(true)
            attr:data-name="__ScannerTrigger"
        >
            <icons::ScanBarcode class="size-4" />
            {label}
        </Button>

        <Dialog>
            <DialogContent open=open class="max-w-lg">
                <DialogHeader>
                    <DialogTitle>"Escanear código"</DialogTitle>
                    <DialogDescription>{description}</DialogDescription>
                </DialogHeader>
                <div class="relative overflow-hidden rounded-lg border bg-muted">
                    <video
                        node_ref=video_ref
                        class="aspect-video w-full object-cover"
                        autoplay
                        playsinline
                        muted
                    ></video>
                </div>
                <Show when=move || error.get().is_some()>
                    <p role="alert" class="text-sm text-destructive">
                        {move || error.get().unwrap_or_default()}
                    </p>
                </Show>
            </DialogContent>
        </Dialog>
    }
}

/// Resolve the platform decoder once per scan session.
#[cfg(target_arch = "wasm32")]
fn detector() -> Result<js_sys::Object, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;

    let window = web_sys::window().ok_or_else(|| "No hay ventana del navegador".to_string())?;
    let ctor = js_sys::Reflect::get(&window, &JsValue::from_str("BarcodeDetector"))
        .map_err(|_| "El navegador no expone BarcodeDetector".to_string())?;
    if !ctor.is_function() {
        return Err(
            "Este navegador no soporta la lectura de códigos con BarcodeDetector".to_string(),
        );
    }
    let ctor: js_sys::Function = ctor
        .dyn_into()
        .map_err(|_| "BarcodeDetector no es una función".to_string())?;

    // Prefer the common formats; an unsupported name makes `BarcodeDetector`
    // reject the whole construction, so build the list defensively.
    let formats = js_sys::Array::new();
    for name in [
        "qr_code", "code_128", "code_39", "ean_13", "ean_8", "upc_a", "upc_e",
    ] {
        formats.push(&JsValue::from_str(name));
    }
    let init = js_sys::Object::new();
    js_sys::Reflect::set(&init, &JsValue::from_str("formats"), &formats)
        .map_err(|_| "No se pudo configurar BarcodeDetector".to_string())?;
    let args = js_sys::Array::new();
    args.push(&init.into());
    let detector = js_sys::Reflect::construct(&ctor, &args)
        .map_err(|_| "BarcodeDetector rechazó los formatos".to_string())?;
    detector
        .dyn_into()
        .map_err(|_| "BarcodeDetector no devolvió un objeto".to_string())
}

/// Decode one frame, returning the first raw value found.
#[cfg(target_arch = "wasm32")]
async fn detect_once(
    detector: &js_sys::Object,
    video: &web_sys::HtmlVideoElement,
) -> Result<Option<String>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let detect = js_sys::Reflect::get(detector, &JsValue::from_str("detect"))
        .map_err(|_| "BarcodeDetector no tiene detect()".to_string())?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "BarcodeDetector.detect no es una función".to_string())?;
    let promise = detect
        .call1(detector, video)
        .map_err(|_| "BarcodeDetector.detect falló".to_string())?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "BarcodeDetector.detect no devolvió una promesa".to_string())?;
    let results = JsFuture::from(promise)
        .await
        .map_err(|_| "BarcodeDetector.detect rechazó la lectura".to_string())?;
    let results = results
        .dyn_into::<js_sys::Array>()
        .map_err(|_| "BarcodeDetector no devolvió una lista".to_string())?;
    for index in 0..results.length() {
        let item = results.get(index);
        let raw = js_sys::Reflect::get(&item, &JsValue::from_str("rawValue"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
        if !raw.trim().is_empty() {
            return Ok(Some(raw));
        }
    }
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
async fn start_camera(video: &web_sys::HtmlVideoElement) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| "No hay ventana del navegador".to_string())?;
    let navigator = window.navigator();
    let devices = navigator
        .media_devices()
        .map_err(|_| "El navegador no expone getUserMedia".to_string())?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_video(&JsValue::TRUE);
    constraints.set_audio(&JsValue::FALSE);
    let promise = devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|_| "No se pudo abrir la cámara".to_string())?;
    let stream = JsFuture::from(promise)
        .await
        .map_err(|_| "El usuario o el navegador denegó la cámara".to_string())?;
    let stream: web_sys::MediaStream = stream
        .dyn_into()
        .map_err(|_| "getUserMedia no devolvió un MediaStream".to_string())?;
    video.set_src_object(Some(&stream));
    let _ = video.play();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn stop_camera(video: &web_sys::HtmlVideoElement) {
    use wasm_bindgen::JsCast;
    if let Some(stream) = video.src_object() {
        if let Ok(stream) = stream.dyn_into::<web_sys::MediaStream>() {
            let tracks = stream.get_tracks();
            for index in 0..tracks.length() {
                if let Ok(track) = tracks.get(index).dyn_into::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }
        }
        video.set_src_object(None);
    }
}

/// Sleep without a timer crate: `setTimeout` is the one scheduler every target
/// the Desk supports already has.
#[cfg(target_arch = "wasm32")]
async fn sleep(milliseconds: i32) {
    use wasm_bindgen_futures::JsFuture;

    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds);
        }
    });
    let _ = JsFuture::from(promise).await;
}
