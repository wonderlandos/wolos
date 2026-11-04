# 0060 - libWolkit on WebGPU and WebAssembly: one renderer trait, wgpu as the portable target
Status: proposed (lead order 2026-11-04: "make libwolkit able to render on WebGPU too, so we can make the site entirely the same way, literally, with optimized WASM"). Builds on 0039 (libWolkit and wolkit), 0036 (the Crystal userland and the GPU surface), 0020 (the view tree libwol apps send) and 0058 (the WolOS design system, whose widgets are the screens this draws). The number is provisional: 0059 is the experience ADR.

## Context
libWolkit already has one pipeline: patch stream -> retained scene -> style (tokens, `.wolcss`) -> layout -> text (harfrust shaping, skrifa outlines, its own glyph atlas) -> `DisplayList` -> instance batcher (`wolkit::paint::batch`). Two things then draw it: the CPU rasteriser (`wolkit::cpu::raster`, the oracle of every golden) and the Vulkan renderer (`wolkit::gpu`, ash, SPIR-V from `shaders/ui.{vert,frag}`) behind the native Wayland window. The site (React/Vite) draws the same look separately, from CSS. The lead wants the site's windows drawn by libWolkit itself, with the same scene, layout, text and tokens, as small as WebAssembly can be.

## Decision
1. **A `Renderer` trait in a new crate `wolkit-render`** (BSD-2-Clause, `crates/wolkit-render`): `render_image(FrameIn) -> Image`, a display list in and premultiplied RGBA8 out. Three implementations: `CpuRenderer` (the rasteriser, the reference), `VulkanRenderer` (the existing ash renderer, wrapped, nothing copied; feature `native`) and `WgpuRenderer`. Every GPU target is held to the CPU reference by the same tolerance the Vulkan goldens use (no channel off by more than 2, at most 0.1 % of pixels off by more than 1).
2. **wgpu is the portable GPU target.** One WGSL uber-shader (`src/shaders/ui.wgsl`), a line-by-line port of the GLSL, runs on Vulkan natively and on the browser's WebGPU under `wasm32-unknown-unknown` (wgpu with `webgpu`, no `wgpu-core` or `naga` in the wasm). The Vulkan renderer stays: the shell's native window keeps its present path. Both GPU renderers share `wolkit::gpu::prepare` (shadow placement, layer composites, the byte images of the instance ring and the clip table), so a target only uploads and records passes. WebGPU has no push constants: a 256-byte uniform slot per draw stands in. A test (`tests/wgsl_port.rs`) holds the WGSL to the GLSL (kinds, flags, bindings, vertex inputs).
3. **The engine core builds for the web.** `wolkit` gets a default feature `native` (Wayland, ash, zbus, xkbcommon, libc, `wol_core`, the C ABI); without it the crate is the portable pipeline and builds for `wasm32-unknown-unknown`. The built-in themes are read from the same JSON files the catalog uses, held to it by a native test. `shared` gates its unix-only modules (`atomic`, `jobs`, `sys`) with `cfg(unix)`. No change to the C ABI or the patch stream.
4. **Text is not rebuilt.** The same harfrust + skrifa shaping and glyph atlas run in wasm; the fonts are bytes the page passes in (Inter regular and semibold). No cosmic-text, no swash, no browser text.
5. **A page is a window without a display** (`wolkit_render::page::Page`): the scene with the kit look, the tokens, input records in, a display list out. The native tests and the browser use this same type. `wolkit_render::view::raise` turns a `shared::view` tree (what libwol apps send, what the shell's Kit host draws in GTK) into scene parts with the kit sheet plus `view.wolcss`, so a widget of the design system is drawn the same by every target.
6. **The browser module** is `crates/wolkit-web` (wasm-bindgen): `mount(canvas, tree, fonts, ...)`, then `frame`, `pointer`, `key`, `text`, `resize`. A 4 KB loader (`site/public/kit/kit.js`) turns DOM events into wolkit input records and draws frames on demand (an event, then frames while a transition runs, then idle). The page is a static route, `/kit/`; the wasm is built with `crates/wolkit-web/build-site.sh` (profile `wasm`: opt-level z, fat LTO, one codegen unit, abort; then wasm-bindgen with the name and producers sections removed).
7. **Dependencies** (all `MIT OR Apache-2.0`): `wgpu` 30.0.1 (default features off), and for the web module only `wasm-bindgen` (pinned `=0.2.130`, equal to the CLI), `wasm-bindgen-futures`, `js-sys`, `web-sys`. `tokio` 1.54.1 (MIT) is in the lock file through a feature of `wasm-bindgen-futures`; no build links it (accepted by the CTO). Native `wolkit-render` also pulls `wgpu-core`, `wgpu-hal` and `naga`; the shell does not depend on `wolkit-render`, so the desktop binaries do not grow.

## Measured (2026-11-04, headless Chromium with SwiftShader WebGPU, local server; not a real GPU)
- Payload of the page: `wolkit_web_bg.wasm` 2.12 MB raw, 797 KB gzip; glue 73 KB / 14 KB; loader 4 KB / 1.7 KB; Inter regular + semibold 481 KB gzip; total 1.29 MB gzip against the 1.5 MB budget.
- First frame: cold 230 to 310 ms from page start (module and fonts about 35 ms, mount to first frame 190 to 270 ms: shaping, atlas, pipeline creation); warm 58 to 86 ms; CPU time of one frame 4 to 13 ms; 52 instances in one draw call.
- Native, lavapipe: the 13 shared golden scenes, wgpu vs CPU, worst channel difference 1, no pixel off by more than 1 beyond the 0.1 % bound; the same numbers as the Vulkan renderer.
- The page against the GTK golden of the same tree (`app-window-parts`): 3.0 % (light) and 3.2 % (dark) of pixels differ by more than 48 in some channel (GTK and wolkit rasterise text and focus differently); the test bound is 4 %.
- The browser frame against the native wgpu frame of the same page: worst channel difference 1, no pixel beyond.

## Consequences
- The site can show any design-system window as the real thing: same tokens, same layout, same shaping. The widget tests of the design system run on the CPU and wgpu targets with no display, and the headless-Chromium check proves the browser path.
- One more shader to keep. `tests/wgsl_port.rs` catches drift in constants and bindings; the goldens catch drift in pixels.
- A browser without WebGPU shows a sentence, not the window. A CPU fallback (the same `Page` drawn by `CpuRenderer` into a 2D canvas) is possible with no new code in the engine and is not built.
- The wasm is a build product (2.1 MB): built by CI (`build-site.sh`), not committed.
- Accessibility: a canvas is opaque to assistive technology. The page keeps an accessible name on the canvas and the keyboard works (Tab, Space, arrows); a full accessibility tree for the web (the AT-SPI work, mirrored into ARIA) is future work.

## Alternatives rejected
- **A second hand-written WebGPU renderer through web-sys, keeping ash**: no shared shader, a second batcher, more code in the wasm than wgpu's thin web backend.
- **Software only: the CPU rasteriser into `putImageData`**: works in every browser, but the lead asked for WebGPU and the GPU path is the one that scales to the full desktop; kept as the fallback idea above.
- **Skia (CanvasKit) or vello**: a second text and paint stack with its own rounding; the goldens would no longer be the oracle, and the wasm is larger.
- **Compiling the GTK shell to the web**: not a thing GTK supports; the shell's look is already reproduced by wolkit's kit sheet.

## Risks
- wgpu moves fast (a major version about every quarter); pinned by the lock file. A wasm-bindgen CLI that differs from the crate's pin fails the build with a message.
- WebGPU is not in every browser yet; see the fallback above.
- Real-GPU timings in a browser are not measured; the numbers above are a software adapter.
