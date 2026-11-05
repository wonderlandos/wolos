# libWolkit: the UI engine (ADR 0039)

One engine for every WolOS app that draws its own window: `wolkit.h` + `libwolkit.so.1`. Code: `crates/wolkit` (Rust, BSD-2-Clause, depends on `shared` and `core` only). The Crystal binding is the `wolkit` shard (`docs/modules/wolkit.md`); the style language is `docs/modules/wolkit-style.md`. Amends ADR 0036.

An app builds a scene with **patch streams** (`wolkit_window_apply`) and hears back through **event records** (`wolkit_engine_read_events`). There are no callbacks and no per-node calls.

## Rules (the header comment has the full text)

| Rule | Means |
|---|---|
| Opaque handles | `_create` returns a reference to `_release`; `_retain` adds one; `_get_` borrows; `_release(NULL)` is fine; a wrong or released handle is refused, not used (`src/ffi/handle.rs`) |
| Strings and buffers | UTF-8 plus length; caller-owned, borrowed for the call; the engine copies what it keeps |
| Errors | `wolkit_status` return (0 OK, 1 INVALID_ARGUMENT, 2 PERMISSION_DENIED, 3 UNAVAILABLE, 4 INTERNAL, 5 LIMIT, 6 DEVICE_LOST); `wolkit_error **` filled only on failure, with one sentence for a person |
| "needed" buffers | `*needed` is always set; a buffer too small gets `WOLKIT_ERR_LIMIT` and is left alone |
| Threads | `retain`/`release` anywhere; the rest one thread at a time per engine (a lock makes a mistake a wait) |
| Event loop | poll `wolkit_engine_get_fd`, call `wolkit_engine_dispatch(engine, 0, &flags, &err)`, drain with `wolkit_engine_read_events` |
| Stability | symbols versioned by `abi/wolkit.map`; ids in the header are append-only |

## The pipeline

patch stream -> scene arena (dirty bits) -> style -> layout -> paint -> CPU raster or Vulkan. `src/window/pipeline.rs` runs style then layout; each stage reads the scene through `SceneRead` and writes only what it computed through `SceneWrite` (`src/scene/read.rs`). Rectangles are stored in window space, before scroll offsets. A patch marks only `STYLE` (properties, classes, state) or `LAYOUT|PAINT` (text, structure); the style stage derives the rest by comparing computed styles.

| Part | Path | Owner |
|---|---|---|
| Scene arena, patch decoder and writer, atomic apply | `src/scene/` | W1 |
| Event records and queue | `src/event/` | W1 |
| Engine, windows, dispatch | `src/engine.rs`, `src/window/` | W1 |
| C ABI | `src/ffi/` | W1 |
| Style, layout | `src/style/`, `src/layout/` | W2 |
| Text, paint, CPU rasteriser | `src/text/`, `src/paint/`, `src/cpu/` | W3 |
| Native window, input, accessibility | `src/window/native/`, `src/input/`, `src/a11y/` | W4 |
| GPU | `src/gpu/` | W5 |

## Patch stream v1 (interface I10)

Little-endian. Header (24 bytes): `u32 magic "WKP1"`, `u16 format` (1), `u16 0`, `u32 seq`, `u32 op_count`, `u32 pool_len`, `u32 0`. Then the ops, then the string pool (the last `pool_len` bytes). A patch is a multiple of 4 bytes and at most 1 MiB.

An op is `u16 opcode`, `u16 (bit 15 must-understand | length in 4-byte words, header included)`, then payload words. An unknown opcode without the flag is skipped; with it the whole patch is refused. A known op has exactly its number of words.

The string pool holds entries of `u32 length`, the bytes, zero padding to 4. A string reference is the byte offset of an entry; `WOLKIT_NO_STR` is the empty string. UTF-8 is checked once, over the pool, before anything is applied.

A node id is `generation << 24 | slot`. The slot is below 65,536; slot 0 is the window itself and always exists. The app chooses ids, so creating a node needs no round trip; a stale id (wrong generation) is refused.

| Op | Words after the header | Meaning |
|---|---|---|
| CREATE | node, kind | allocate a node (not yet in the tree) |
| INSERT | parent, child, index | place a detached node; `WOLKIT_APPEND` as index appends |
| REMOVE | node | detach (the node and its subtree stay) |
| MOVE | node, parent, index | detach and place |
| SET_PROP | node, prop, value words | inline property; words by type (below) |
| SET_TEXT | node, strref | text of text, button, toggle, text input |
| SET_STYLE | node, n, class ids (n at most 8) | classes from the loaded style table |
| SET_STATE | node, mask | app-owned state bits (`WOLKIT_STATE_DISABLED`, `SELECTED`, `CHECKED`, `EXPANDED`, `BUSY`) |
| SET_KEY | node, lo, hi | stable key for keyed children |
| BIND_EVENT | node, mask | which events this node wants (`WOLKIT_BIND_*`) |
| SET_A11Y | node, role, name strref, description strref | accessibility |
| ANIM | node, prop, target f32, mode, param | tween or spring on a number property |
| LIST_RANGE | node, count, row height f32, overscan | virtual list description |
| FOCUS | node | request focus (0 clears) |
| FREE | node | release the node and its subtree |

Property value words by type: `f32`, `i32`, `bool`, `enum`, `token`, `str`, `u32` one word; `dim` two (unit 0 auto, 1 px, 2 percent; then `f32`); `pair` two `f32`; `border` (width `f32`, colour token); `gradient` five (angle `f32`, count, three tokens); `shadow` four (dx, dy, blur `f32`, token); `transition` three. The table is `src/scene/prop.rs`; `abi/wolkit-1.0.props` snapshots it.

**Atomic apply.** The patch is validated first (`Patch::parse`: framing, every op's shape and values, the pool, every string reference). Then every change is journaled; the first op that cannot apply rolls the journal back, so the window is exactly as before and the caller gets one sentence. `FREE` takes effect at the end of the patch for the same reason. A backend that cannot show the result (the Kit wire's 200 parts) refuses the patch the same way.

**How to add a property.** Append a row to `props!` in `src/scene/prop.rs` with the next free id for its group, run `tests/abi.rs` (it tells you the header and `abi/wolkit-1.0.props` need the row), add the `#define` to `include/wolkit.h` and, if the version is already shipped, a line to a new snapshot (`abi/wolkit-1.1.props`) instead of editing the shipped one. A style meaning belongs to `src/style/`, not here.

## Event record v1 (interface I12)

`u32 len` (whole record, a multiple of 8), `u16 kind`, `u16 flags`, `u32 window`, `u32 node`, `u32 seq`, then the payload from byte 20 (read it unaligned), zero padded. `seq` is the sequence number of the last patch the engine had applied: an edit event made before a patch the app has since sent can be told from news. Flag 1 says events were dropped just before this record. Events go to the nearest ancestor (or the node) that bound the kind; window events (resize, close request, theme, frame tick, visibility, diagnostic) always arrive.

| Kind | Payload |
|---|---|
| CLICK | `u32 button`, `u32 mods`, `f32 x`, `f32 y` |
| POINTER_ENTER, POINTER_LEAVE | `f32 x`, `f32 y` |
| KEY | `u32 keysym`, `u32 pressed`, `u32 mods`, `u32 repeat` |
| TEXT_CHANGED, TEXT_SUBMIT | `u32 caret`, `u32 anchor`, `u32 length`, then the text bytes |
| TOGGLE | `u32 on` |
| SLIDER | `f32 value` |
| SCROLL | `f32 x`, `f32 y` |
| FOCUS | `u32 gained` |
| RESIZE | `u32 width`, `u32 height`, `f32 scale` |
| THEME_CHANGED | `u32 dark`, `u32 accent_rgb`, `u32 flags` |
| DIAGNOSTIC | `u32 level` (0 note, 1 warning, 2 error), `u32 length`, the text bytes |
| others | defined with the part that makes them (drop, drag, list range, surface, a11y action) |

The queue holds at most 4,096 records and 4 MiB. A newer scroll, slider, resize, frame tick or hover event replaces the pending one of the same node; when the queue is full the oldest of those is dropped first and the next record carries the overflow flag. A buffer of `WOLKIT_EVENT_MAX_RECORD` (about 64 KiB) bytes always makes progress.

## Backends

| Backend | When | Notes |
|---|---|---|
| headless | `WOLKIT_ENGINE_HEADLESS` | no display; virtual clock (`wolkit_window_advance_time`); input injection; `wolkit_window_render_rgba`; the only one that accepts `wolkit_engine_set_environment` |
| kit | `WOLKIT_ENGINE_FORCE_KIT`, or no native backend | lowers to `shared::view` (`src/window/kit/lower.rs`), one socket per engine (`$WOL_KIT_SOCKET`), one reader thread `wolkit-kit-rx`, lines at most 256 KiB, a slow or silent host never blocks the app |
| native | default once it exists | the engine's own window (`src/window/native/`) |

`wolkit_window_render_rgba` and the native `wl_shm` path draw the same frame (`window::pipeline::render_rgba`): the scene lowered to a display list with its computed styles, the token colours of the built-in light or dark theme under the person's accent and contrast (`src/paint/palette.rs`), and text shaped with real fonts (`src/paint/scene.rs`), then the CPU rasteriser. Layout measures text with the same shaper through a per-window paragraph cache keyed by text, width and style (`src/text/measure.rs`), so paint reuses what layout shaped. Fonts are one set per process (the first engine decides it): an engine made through the C ABI indexes the person's font folders (lazily, at the first window), except a headless one (`WOLKIT_ENGINE_HEADLESS`), which reads only the pinned folder `/usr/share/fonts/inter` (the UI face the goldens use; `WOLKIT_FONT_DIR` names another), so its screenshots are the same on every machine that has that folder; an engine made in-process without that (the Rust tests) has no fonts and draws every glyph as the missing-glyph box, which is the same on every machine.

Kit lowering, and what does not survive: `box` becomes a row or a stack (direction), spacing the nearest of 0, 4, 8, 12, 16, 24; `text` a label with its role; `button`, `toggle` and `text_input` take their label from their text, else their first text child, else their accessibility name (none is an error); `slider`, `image` (size snapped to the Kit's sizes) and `vlist` (all mounted rows as a list) map directly. Custom style properties, padding, borders, shadows, transforms and animations are ignored. `canvas` and `surface` become one placeholder sentence; `overlay`, `divider` and hidden parts are omitted. More than 200 parts or 12 levels is `WOLKIT_ERR_LIMIT`. A part's wire id is its node id, so a click on the wire maps straight back. Golden: `tests/kit_golden/kit_app.json` (rerun with `WOLKIT_BLESS=1` and review the diff).

## Render targets (ADR 0060)

The pipeline ends in a `DisplayList` and the instance batcher; what draws it is a `wolkit_render::Renderer` (`crates/wolkit-render`, BSD-2-Clause): `render_image(FrameIn) -> Image`, premultiplied RGBA8 out.

| Target | Where | Notes |
|---|---|---|
| `CpuRenderer` | everywhere | `wolkit::cpu::raster`, the reference every other target is held to |
| `VulkanRenderer` | native (feature `native`) | the ash renderer of the Wayland window (`wolkit::gpu`), wrapped, not copied |
| `WgpuRenderer` | native (Vulkan, lavapipe in CI) and `wasm32-unknown-unknown` (the browser's WebGPU) | `src/shaders/ui.wgsl`, a port of `shaders/ui.{vert,frag}`; a 256-byte uniform slot per draw replaces push constants |

Both GPU targets use `wolkit::gpu::prepare` (shadow placement, layer composites, the byte images of the instance ring and the clip table). The tolerance against the CPU image is the Vulkan golden one: no channel off by more than 2, at most 0.1 % of pixels off by more than 1 (`wolkit_render::MAX_CHANNEL_DIFF`, `MAX_DIFF_SHARE`).

The `native` feature of `wolkit` (default) holds everything that needs Linux: Wayland, ash, zbus, xkbcommon, libc, `wol_core`, the engine on file descriptors, the Kit socket and the C ABI. Without it the crate is the portable pipeline (scene, style, layout, text, paint, CPU raster, input, headless window) and builds for `wasm32-unknown-unknown`; the built-in themes then come from the same JSON files as `ThemeCatalog::builtin` (`paint::palette::portable`, held to the catalog by a native test). In the web build there are no decode threads: pictures and icons decode inline, and `image::icon::register` hands the page its symbolic icons (no icon folder).

`wolkit_render::page::Page` is a window without a display: the scene with the kit look and the shell's tokens, fonts from memory, input records in (`pointer`, `key`, `text`), a display list out; `wolkit_render::view::raise` shows a `shared::view` tree in it. `crates/wolkit-web` mounts a `Page` in a `<canvas>` (wasm-bindgen) and `site/public/kit/kit.js` forwards DOM events. Build: `bash crates/wolkit-web/build-site.sh` (needs the `wasm32-unknown-unknown` target and `wasm-bindgen` CLI 0.2.130). Check: `node crates/wolkit-web/check-browser.mjs SITE NATIVE_DIR` (headless Chromium with SwiftShader WebGPU; compares the browser frame with the native wgpu frame, light and dark, and clicks the switch).

Tests: `cargo test -p wolkit-render` (`tests/wgpu_golden.rs`: the 13 shared golden scenes on every target against the CPU image, `WOLKIT_GPU_REQUIRED=1` turns a missing adapter into a failure, `WOLKIT_GPU_HARDWARE=1` uses a hardware adapter; `tests/page.rs`: the page against the CPU, against the GTK golden `app-window-parts` (at most 4 % of pixels more than 48 off), a click on the switch, the named icon; `tests/wgsl_port.rs`: the WGSL against the GLSL).

## Pictures (`image` nodes, ABI 1.1)

An `image` node shows a picture: `WOLKIT_PROP_IMAGE_ID` names one the app made with the `wolkit_image_*` verbs, else `WOLKIT_PROP_IMAGE_SOURCE` with an absolute path (or a `file://` URL) names a file the engine reads itself; `icon:<name>` names an icon of the theme (below). `WOLKIT_PROP_IMAGE_FIT` places it: `contain` (the default: the whole picture, centred), `cover` (the box filled, the overflow cut), `fill` (stretched) or `none` (one picture pixel per logical pixel, centred and cut; appended in 1.1). The node's `border-radius` rounds the picture (a rounded clip), its `opacity` fades it, its `background` shows where `contain` leaves room. Code: `src/image/` (decoding, fit, the path cache), `src/paint/scene.rs` (lowering), `src/cpu/raster.rs` and `src/gpu/images.rs` (drawing).

| Verb | Since | Does |
|---|---|---|
| `wolkit_image_create(engine, bytes, len, &out, &err)` | 1.0 | PNG or JPEG bytes, no size cap (SVG is accepted and kept as FAILED: nothing draws it yet) |
| `wolkit_image_create_encoded(engine, bytes, len, max_side, &out, &err)` | 1.1 | the same with a cap on the longer side (0: none) |
| `wolkit_image_create_file(engine, path, len, max_side, &out, &err)` | 1.1 | an absolute path, read and decoded on a worker; a missing or broken file is FAILED, not an error |
| `wolkit_image_create_rgba(engine, w, h, stride, px, len, &out, &err)` | 1.0 | straight RGBA8, ready at once |
| `wolkit_image_get_id`, `_retain`, `_release` | 1.0 | the id a node names it by; references |
| `wolkit_image_get_size(img, &w, &h)` | 1.1 | the upright natural size; known at once from the header for bytes, after the header is read for a file (`UNAVAILABLE` before) |
| `wolkit_image_get_state(img)` | 1.1 | `WOLKIT_IMAGE_PENDING`, `READY` or `FAILED` |

**Decoding.** PNG (every colour type, 16-bit stripped, palettes expanded) and baseline or progressive JPEG, by the first bytes. The header is read first and its size checked against the limits before any pixel memory is allocated. The EXIF orientation (APP1 for JPEG, `eXIf` for PNG; the eight turns and mirrors) is applied, so the size and pixels are upright. Then the cap (halved while that stays at or above it, then one bilinear step to the exact size), premultiplied alpha, and a mip chain halved down to 16 pixels whose largest level is at most 2,048 a side (the atlas page; "actual size" of a larger photo is drawn from that level). Decoding runs on up to four `wolkit-decode` threads (half the cores), off the app's thread; a picture released before its turn is skipped.

**Placeholder and redraw.** Until a picture is ready (and when it failed) its node shows a `control`-token fill with the node's radius. When a decode finishes the picture generation moves: the chunk caches see it in their version, and each native window has a wake-up descriptor among its `fds()` (`image::subscribe`) and draws again from `pump`. The headless backend asks for every picture of the window and waits (at most 5 s) before `wolkit_window_render_rgba` draws, so its frames do not depend on timing.

**Paths.** A source path is decoded at a size bucket: twice the node's longer side, rounded up to 64, 128, 192, 256, 384, 512, 768, 1024, 1536 or 2048, so a thumbnail grid decodes small. One cache per process keyed by path and bucket; a larger ready bucket serves a smaller node, and a smaller ready one stands in while a larger one decodes (a resize does not flash the placeholder). Least recently used entries go past 512 entries or 256 MB.

**Drawing.** Both rasterisers choose the smallest level at least the device size of the picture's destination, and sample it bilinearly with weights quantised to 0..255 and integer mixing (`image::fit::bilinear`, `K_IMAGE` in `shaders/ui.frag`), so the GPU and CPU images agree (`tests/golden/scenes/13_images.png`: 0 pixels differ on the software Vulkan device). On the GPU a picture is one instanced quad (kind 6) sampling an RGBA8 texture array (binding 4; 2,048-pixel pages, at most four, shelf-packed); a level is uploaded once, when first placed, and a frame uploads only newly placed levels (200 thumbnails: 1 draw, 0 bytes after the first frame). A full atlas starts over and the frame is built again with only what it shows. On the CPU, rows at a time: interior pixels skip the edge coverage and an exactly hit texel (a 1:1 blit) skips the mix. A translucent bare picture (no background, border, shadow or focus ring) folds its opacity into the picture instead of a layer.

**Size of a part.** An `image` node with no width or height of its own takes its picture's natural size, one picture pixel per logical pixel (an icon: 16). While the size is not known yet (a path whose header was not read; a picture still decoding) the part is 64 pixels square; learning the size moves `image::size_generation`, which is part of the layout key (`window::pipeline::ensure_layout`) and makes the next layout measure the image nodes again. A path's header is read on a worker (the first 1 MiB of the file) and cached with the pictures.

**Icons.** `image-source` `icon:<name>` draws the icon `<name>` of the icon theme (the DSL's `icon_button :trash` sends it). The name is looked up as `<name>-symbolic.svg`, `<name>.svg`, `<name>-symbolic.png`, `<name>.png`, with `_` read as `-` and a few short names mapped to their theme names (`back` is `go-previous`, `trash` is `user-trash`; the table is `ALIASES` in `src/image/icon.rs`). Folders: `WOLKIT_ICON_DIR` when set, else `icons/WolOS/symbolic/{actions,status}`, `icons/WolOS/scalable/{apps,places,devices,mimetypes}` and `icons/hicolor/symbolic/apps` under the data folders; a headless engine reads only `WOLKIT_ICON_DIR` or `/usr/share/icons/WolOS/symbolic/{actions,status}`, so its screenshots do not depend on the machine. A symbolic SVG is read by a small reader (`viewBox`, `path` with `d`, `fill="none"`, `opacity`; every path command, arcs turned into cubic curves with square roots only), filled non-zero by the exact-area rasteriser and tinted with the part's text colour, at twice the part's size (the mip chain serves 1x); a PNG icon keeps its colours. Anything else in an SVG is ignored. Reading and rasterising run on the decode workers; an icon that is not ready, or not found, draws nothing (no placeholder). Memory Saver (hidden and critical) forgets the path cache, the size reads and the icons.

**Stability of 1.1.** The four 1.1 verbs are in the `WOLKIT_1.1` node of `abi/wolkit.map` with their baseline in `abi/wolkit-1.1.symbols`; the header snapshot `abi/wolkit-1.1.h` is frozen at the release that ships 1.1 (0.13), not before, so until then the 1.1 declarations may still change.

## Dumps

`wolkit_window_dump` returns text; the shard's golden patch-stream specs compare against it. `WOLKIT_DUMP_SCENE`: one line per node, indented by depth: `id kind`, then `classes=`, `state=`, `key=`, `on=` (the bind mask), each inline property as `name=value`, then the text in quotes. `WOLKIT_DUMP_LAYOUT`: `id kind x y w h`. `WOLKIT_DUMP_A11Y`: `id kind role=… name="…"`, `focused`, `state=…`.

## Limits (every list bounded)

| Limit | Value |
|---|---|
| Nodes per window | 65,536 (the kit backend: 200) |
| Tree depth | 64 (the kit backend: 12) |
| Windows per engine | 32 |
| Patch bytes per apply | 1 MiB |
| Text per node | 64 KiB |
| Classes per node | 8 |
| Pending events | 4,096 records, 4 MiB |
| Decoded image | 16,384 px a side, 256 megapixels; 512 MB of images per process |
| Encoded image | 64 MiB (bytes or file) |
| Drawn picture level | 2,048 px a side; GPU picture atlas 4 pages of 2,048 x 2,048 |
| Picture path cache | 512 entries, 256 MB |
| Style table | 4 MiB |

## Security and untrusted input

The patch stream, style tables, style sheets, images and fonts are untrusted. Each is bounded, validated before use and fuzzed (`fuzz/fuzz_targets/wolkit_patch.rs`, `wolkit_wsb.rs`, `wolkit_wolcss.rs`, `wolkit_image.rs`, and `wolkit_text.rs` for the text buffer behind `text_input`; seeds in `fuzz/regressions/`; `crates/wolkit/tests/patch_fuzz.rs` replays them and a deterministic mutation on stable, `tests/image.rs` mutates the picture fixtures). The picture decoders are permissively licensed crates `deny.toml` allows; a decoder panic on a worker becomes a FAILED picture. A panic stops at the boundary as `WOLKIT_ERR_INTERNAL`. The engine opens the Kit socket (kit backend), on the native backend the Wayland socket, and the picture files the app names (an `image-source` path or `wolkit_image_create_file`, read on a worker, at most 64 MiB); it has no privilege and sends no telemetry.

## Tests

`cargo test -p wolkit`: `tests/abi.rs` (exports, linker map, header against the Rust signatures and constants, shipped declarations and property ids never change), `tests/c_hello.rs` (a C program, `tests/c/hello.c`, builds a window, clicks and reads the event), `tests/conformance.rs` (the same scenarios on every backend, with the reasons for each difference), `tests/alloc_free.rs` (zero heap allocations in a steady-state apply and frame, measured with a counting allocator), `tests/patch_fuzz.rs`, `tests/image.rs` (decoding, orientation, the cap, the 1.1 verbs, lowering, the placeholder, a headless window drawing a file). Shared test helpers (the fake Kit host, the `libwolkit.so.1` link, the C compile) are in `tests/support/`. `anim/` is the animation clock: `ANIM` ops run as tweens or springs on the headless virtual clock (`wolkit_window_advance_time`) and the native frame clock; the Kit host gets the final value at once.
