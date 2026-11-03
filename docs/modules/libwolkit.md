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

`wolkit_window_render_rgba` and the native `wl_shm` path draw the same frame (`window::pipeline::render_rgba`): the scene lowered to a display list with its computed styles, the token colours of the built-in light or dark theme under the person's accent and contrast (`src/paint/palette.rs`), and text shaped with real fonts (`src/paint/scene.rs`), then the CPU rasteriser. Layout measures text with the same shaper through a per-window paragraph cache keyed by text, width and style (`src/text/measure.rs`), so paint reuses what layout shaped. Fonts are one set per process: an engine made through the C ABI indexes the person's font folders (lazily, at the first window); an engine made in-process without that (the Rust tests) has no fonts and draws every glyph as the missing-glyph box, which is the same on every machine.

Kit lowering, and what does not survive: `box` becomes a row or a stack (direction), spacing the nearest of 0, 4, 8, 12, 16, 24; `text` a label with its role; `button`, `toggle` and `text_input` take their label from their text, else their first text child, else their accessibility name (none is an error); `slider`, `image` (size snapped to the Kit's sizes) and `vlist` (all mounted rows as a list) map directly. Custom style properties, padding, borders, shadows, transforms and animations are ignored. `canvas` and `surface` become one placeholder sentence; `overlay`, `divider` and hidden parts are omitted. More than 200 parts or 12 levels is `WOLKIT_ERR_LIMIT`. A part's wire id is its node id, so a click on the wire maps straight back. Golden: `tests/kit_golden/kit_app.json` (rerun with `WOLKIT_BLESS=1` and review the diff).

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
| Decoded image | 16,384 px a side, 256 megapixels; 512 MB of images |
| Style table | 4 MiB |

## Security and untrusted input

The patch stream, style tables, style sheets, images and fonts are untrusted. Each is bounded, validated before use and fuzzed (`fuzz/fuzz_targets/wolkit_patch.rs`, `wolkit_wsb.rs`, `wolkit_wolcss.rs`; seeds in `fuzz/regressions/`; `crates/wolkit/tests/patch_fuzz.rs` replays them and a deterministic mutation on stable). A panic stops at the boundary as `WOLKIT_ERR_INTERNAL`. The engine opens only the Kit socket (kit backend) and, on the native backend, the Wayland socket; it has no privilege and sends no telemetry.

## Tests

`cargo test -p wolkit`: `tests/abi.rs` (exports, linker map, header against the Rust signatures and constants, shipped declarations and property ids never change), `tests/c_hello.rs` (a C program, `tests/c/hello.c`, builds a window, clicks and reads the event), `tests/conformance.rs` (the same scenarios on every backend, with the reasons for each difference), `tests/alloc_free.rs` (zero heap allocations in a steady-state apply and frame, measured with a counting allocator), `tests/patch_fuzz.rs`. Shared test helpers (the fake Kit host, the `libwolkit.so.1` link, the C compile) are in `tests/support/`. `anim/` is the animation clock: `ANIM` ops run as tweens or springs on the headless virtual clock (`wolkit_window_advance_time`) and the native frame clock; the Kit host gets the final value at once.
