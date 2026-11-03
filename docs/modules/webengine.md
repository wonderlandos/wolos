# webengine: the web-app runtime's engine (ADR 0042)

**Concept.** WolOS's own web apps and the site-as-app entries (track 2 of ADR 0042) run on the Rust browser engine in a Wolkit window, one app per process. `crates/webengine` (Rust, BSD-2-Clause, depends on `shared`, `core` and `wolkit` only) is the adaptor: a `WebView` made from a frozen `webengine::Policy` and an `App` paints into a wolkit `surface` node, or headless into memory. The engine itself is the library `libwolwebengine.so.1` (C ABI 1, `crates/webengine/engine/include/wolwebengine.h`) from the `[wolos]` package `webengine-wolos`, loaded at run time (`libloading`).

**Why.** The engine is large (about 900 crates, a C++ JavaScript engine) and its embedding API still moves. Pinned by commit and built once, it never enters the WolOS workspace, its lock or its build time, and every web app uses the one copy. What the system decides (which origins, which files, which process, which input) is plain Rust in this crate and tested without the engine; the engine side is a thin shim.

**How.**

```rust
use webengine::{App, Policy, Target, WebView};
let policy = Policy { origins: vec!["https://notes.example".into()], ..Policy::default() };
let app = App::new("org.wolos.webapp.notes", profile_dir); // wol_core::webapps::profile_dir
// SAFETY: `sub` (Engine::create_subsurface) outlives the view.
let view = WebView::new(&policy, app, unsafe { Target::surface(&sub) }, "https://notes.example/")?;
// per wolkit event record of the window:
for b in webengine::input::bridge(record, surface_node) { view.apply(&b); }
if view.take_woken() { view.spin(); }
for url in view.take_refused() { /* offer it in the system browser */ }
```

A host process starts as `<host> --content-process TOKEN` when the engine wants its content process: `webengine::process::content_token(&args)` says so, and the host calls `webengine::ffi::run_content_process(token)` before anything else, then returns from `main`.

## The C boundary (`libwolwebengine.so.1`, ABI 1)

| Part | Means |
|---|---|
| Exports | the 16 `wwe_*` functions of `crates/webengine/engine/wolwebengine.map`, at version `WOLWEBENGINE_1`; append-only within the major |
| Source | `crates/webengine/src/engine/lib.rs` (not a module of `webengine`; counted by its unsafe ratchet row), manifest `crates/webengine/engine/Cargo.toml` (excluded from the workspace) |
| Build | `webengine-wolos` copies `crates/webengine` into the engine's workspace as `ports/wolwebengine`, builds `-p wolwebengine` under the engine's `Cargo.lock` (prepare() refuses any other change to the lock), links with `crates/webengine/engine/link.sh` |
| Threads | every call on the thread that made the engine; `wake` may be called from any thread |
| One per process | a second `wwe_engine_new` is refused ("one engine per process") |
| Errors | NULL or -1 and one sentence in the caller's `err` buffer |
| Callbacks | `allow` (required, every navigation and resource load), `wake`, `clip_get`, `clip_set`, all with the host's `ctx` |

## The origin gate (`src/sandbox.rs`)

| Load | Allowed when |
|---|---|
| `https://h[:p]`, `wss://` | the policy names `https://h[:p]`, or `https://*.base` and `h` is a subdomain of `base` |
| `http://`, `ws://` | the policy names `http://h[:p]` |
| `file:///path` | the policy names a folder `file:///dir/` and the path is inside it; never the profile directory (refused when the policy is read) |
| `about:blank`, `about:srcdoc` | always |
| `data:` | as a resource, never as a navigation |
| `blob:<origin>/...` | when its origin is allowed |
| anything else | never |

A host or path prefix in `telemetry_off` (`hosts`, `beacon_paths`; a path covers itself and what is below it) is refused even inside an allowed origin. A refused navigation of the page ends with the page as it was (the shim answers `204 No Content`) and is kept for the host (`WebView::take_refused`); a refused frame gets an empty `403`; a refused resource is a network error. Refused loads are counted (`WebView::refused_loads`).

## Process model and sandbox

- One app = one systemd scope and one host process: the host (`org.wolos.webapp.<slug>`, `process::app_id`) is started by `process::scope_argv` in the user manager's scope `app-wol-<app id>-<pid>.scope` (`process::scope_unit`), the name the Memory Saver gives its own app scopes.
- Content processes per site, inside that scope: the engine starts one per registrable domain of the app's frames (each `file:` document counts as its own), each a child of the host started as `<host> --content-process TOKEN`. A page whose frames are one site has exactly one (`tests/content_process.rs`).
- The Memory Saver and the frontmost QoS act on the scope: they see and move the whole app, whatever its number of content processes.
- The engine's own content sandbox is off: at the pin it never loads a page (forge, 2026-11-03). Until the jail helper (NS10) holds the content process, the scope, the origin gate and the profile directory (`App::profile_dir`: storage, cookies and `cache/`) are the bounds.

## Policy (`webengine::Policy`, frozen)

`origins`, `heap_mb` (the script heap cap, MiB), `telemetry_off` (`hosts`, `beacon_paths`), `background_throttle` (a hidden window is throttled by the engine; `WebView::set_visible`), `features`. Switchable features at the pin: `webgl2`, `webgpu`, `service-workers`, `indexeddb`, `notifications`, `webrtc` (`policy::FEATURES`); others are reported by `Policy::unswitchable` for the probes to judge. As JSON an unknown field is refused.

## Input (`src/input.rs`)

| wolkit record (surface node) | Page |
|---|---|
| CLICK | move, press, release at the point |
| POINTER_ENTER, POINTER_LEAVE | move to the point, leave |
| KEY | key value and code from the keysym (US layout for printable codes: the record has no scancode), modifiers, repeat |
| FOCUS, SURFACE_RESIZED, THEME_CHANGED | focus, size and scale, dark or light |

## Gaps

| Gap | Where it is fixed |
|---|---|
| Pointer motion inside a surface, separate button presses, wheel deltas | wolkit record v1 (the engine side takes them: `input::Input::{MouseMove, MouseButton, Wheel}`) |
| IME preedit and commit for a surface; an app clipboard | wolkit (`input::Input::{ImePreedit, ImeCommit}`, `view::Clipboard`) |
| On-screen presentation through the surface (`Target::surface`) | not run yet: needs a compositor on the test machine (CR-8) |
| The engine's own content sandbox | the jail helper (NS10) |
| Media (`<video>`, audio) | the shim is built without the engine's media feature; WA-6's probes |

## Verify

```sh
cargo test -p webengine                       # unit tests; the engine tests need the library:
WOL_WEBENGINE_LIB=/path/libwolwebengine.so.1 WOL_WEBENGINE_REQUIRED=1 cargo test -p webengine
WOL_BLESS=1 ... cargo test -p webengine --test headless_golden   # rewrite tests/golden/golden.png
cargo test -p shared --test architecture --test unsafe_ratchet
cargo deny check
```

Bumping the pin: change `_engine_rev`, `_mozjs` and both checksums in `webengine-wolos/PKGBUILD`, the version string in `src/engine/lib.rs`, the THIRD-PARTY section, then build the package (its check() runs the engine tests) and bless the golden if the rendering moved.
