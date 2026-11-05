// SPDX-License-Identifier: BSD-2-Clause
//! The wasm-bindgen surface: `mount` and the `KitPage` methods the loader calls.

use wasm_bindgen::prelude::*;
use wolkit::window::{KeyInput, PointerInput};
use wolkit_render::page::Page;
use wolkit_render::wgpu_backend::{WgpuRenderer, device_descriptor, unpad};

fn js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

#[wasm_bindgen]
pub struct KitPage {
    page: Page,
    r: WgpuRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    last_ms: f64,
    t0: f64,
}

/// Mounts the view tree `tree_json` (a `shared::view::Node`, JSON) into `canvas`, `width` by
/// `height` CSS pixels at `scale` device pixels each, with the font files `fonts` (Uint8Arrays).
/// Rejects when the browser has no WebGPU adapter.
#[wasm_bindgen]
pub async fn mount(
    canvas: web_sys::HtmlCanvasElement,
    tree_json: String,
    fonts: js_sys::Array,
    width: u32,
    height: u32,
    scale: f32,
    dark: bool,
) -> Result<KitPage, JsValue> {
    let t0 = now_ms();
    // panic = "abort" would otherwise end in a bare `unreachable`: say what panicked.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
    let tree: shared::view::Node = serde_json::from_str(&tree_json).map_err(js)?;
    let fonts: Vec<Vec<u8>> = fonts
        .iter()
        .map(|f| js_sys::Uint8Array::new(&f).to_vec())
        .collect();
    let mut page = Page::new(width, height, scale, dark, fonts).map_err(js)?;
    page.mount(&tree).map_err(js)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let (dw, dh) = page.device_size();
    canvas.set_width(dw);
    canvas.set_height(dh);
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(js)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(js)?;
    let (device, queue) = adapter
        .request_device(&device_descriptor(&adapter))
        .await
        .map_err(js)?;
    let caps = surface.get_capabilities(&adapter);
    // Any 8-bit unorm format: the shader writes exact bytes, an sRGB target would re-encode them.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            )
        })
        .ok_or_else(|| js("The canvas offers no 8-bit unorm format."))?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: dw,
        height: dh,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Srgb,
    };
    surface.configure(&device, &config);
    let r = WgpuRenderer::from_device(&adapter, device, queue);
    let mut kit = KitPage {
        page,
        r,
        surface,
        config,
        last_ms: 0.0,
        t0,
    };
    kit.frame()?;
    Ok(kit)
}

#[wasm_bindgen]
impl KitPage {
    /// Draws one frame into the canvas; returns the milliseconds it took on the CPU side.
    pub fn frame(&mut self) -> Result<f64, JsValue> {
        let t = now_ms();
        let tex = self.surface.get_current_texture();
        let tex = match tex {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => return Err(js(format!("{other:?}"))),
        };
        let view = tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self.config.format;
        let r = &mut self.r;
        let enc = self.page.draw(|f| r.render_to(f, &view, format));
        self.r.queue.submit([enc.finish()]);
        self.r.queue.present(tex);
        self.last_ms = now_ms() - t;
        Ok(self.last_ms)
    }

    /// Milliseconds from the start of `mount` to now (the loader reads it after the first frame).
    pub fn since_mount_ms(&self) -> f64 {
        now_ms() - self.t0
    }

    /// The last frame's numbers, as JSON.
    pub fn stats(&self) -> String {
        let s = self.r.stats;
        format!(
            "{{\"adapter\":{:?},\"draws\":{},\"instances\":{},\"passes\":{},\"layers\":{},\"upload_bytes\":{},\"frame_ms\":{:.3}}}",
            self.r.adapter(),
            s.draws,
            s.instances,
            s.passes,
            s.layers,
            s.upload_bytes,
            self.last_ms
        )
    }

    /// A pointer record (CSS pixels in the canvas, `WOLKIT_POINTER_*`, button 1 left).
    /// Returns the events as JSON `[[kind, part], ...]`.
    pub fn pointer(&mut self, kind: u32, x: f32, y: f32, button: u32, mods: u32) -> String {
        let ev = self.page.pointer(PointerInput {
            kind,
            x,
            y,
            button,
            mods,
        });
        events_json(&ev)
    }

    /// A DOM key (`KeyboardEvent.key`) pressed or released; printable keys go through `text`.
    pub fn key(&mut self, key: &str, pressed: bool, mods: u32) -> String {
        let Some(keysym) = crate::keysym(key) else {
            return "[]".into();
        };
        events_json(&self.page.key(KeyInput {
            keysym,
            pressed,
            mods,
        }))
    }

    /// Committed text (a printable key, an input method's result).
    pub fn text(&mut self, t: &str) -> String {
        events_json(&self.page.text(t))
    }

    /// A part's rectangle `[x, y, w, h]` in CSS pixels (empty when there is no such part).
    pub fn part_rect(&mut self, part: u32) -> Vec<f32> {
        self.page
            .node_rect(part)
            .map_or_else(Vec::new, |r| r.to_vec())
    }

    /// Moves animations on by `ms`.
    pub fn advance(&mut self, ms: f64) {
        self.page.advance((ms.max(0.0) * 1.0e6) as u64);
    }

    /// A new CSS size and device scale (the canvas is resized to match).
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.page.resize(width, height, scale);
        let (dw, dh) = self.page.device_size();
        self.config.width = dw;
        self.config.height = dh;
        self.surface.configure(&self.r.device, &self.config);
    }

    /// The frame read back from the GPU (premultiplied RGBA8, device pixels): for tests.
    pub async fn snapshot(&mut self) -> Result<js_sys::Uint8Array, JsValue> {
        let (w, h) = self.page.device_size();
        let r = &mut self.r;
        let (enc, buf, row) = self.page.draw(|f| {
            r.invalidate();
            r.encode_image(f)
        });
        self.r.queue.submit([enc.finish()]);
        let (tx, rx) = futures_oneshot();
        buf.slice(..)
            .map_async(wgpu::MapMode::Read, move |res| tx(res.is_ok()));
        if !rx.await {
            return Err(js("The readback buffer could not be mapped."));
        }
        let data = {
            let view = buf.slice(..).get_mapped_range().map_err(js)?;
            unpad(&view, w, h, row)
        };
        buf.unmap();
        Ok(js_sys::Uint8Array::from(&data[..]))
    }
}

fn events_json(ev: &[wolkit_render::page::PageEvent]) -> String {
    let parts: Vec<String> = ev
        .iter()
        .map(|e| format!("[{},{}]", e.kind, e.node))
        .collect();
    format!("[{}]", parts.join(","))
}

/// A one-shot boolean channel over a JS promise (no futures crate needed).
fn futures_oneshot() -> (
    impl FnOnce(bool) + 'static,
    impl std::future::Future<Output = bool>,
) {
    let mut resolve_slot: Option<js_sys::Function> = None;
    let promise = js_sys::Promise::new(&mut |resolve, _| resolve_slot = Some(resolve));
    let resolve = resolve_slot;
    let send = move |ok: bool| {
        if let Some(f) = resolve {
            let _ = f.call1(&JsValue::NULL, &JsValue::from_bool(ok));
        }
    };
    let fut = async move {
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    (send, fut)
}

/// The libwol sample window's view tree (`wolkit_render::view::SAMPLE_JSON`), so the loader
/// needs no copy of it.
#[wasm_bindgen]
pub fn sample_json() -> String {
    wolkit_render::view::SAMPLE_JSON.into()
}
