# webengine: the web-app runtime's engine (ADR 0042)

The web-app runtime's engine and its adaptor crate: WolOS's own web apps and the site-as-app entries run on it in a Wolkit window, one app per process.

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
| Exports | the 16 `wwe_*` functions of `crates/webengine/engine/wolwebengine.map` at version `WOLWEBENGINE_1`, and from WWE_1.1 `wwe_engine_set_host_channel` and `wwe_view_post_message` at version `WWE_1.1` (the host channel of system web apps, ADR 0048), and from WWE_1.2 `wwe_engine_set_scheme` (the host's `fed:` scheme, ADR 0052); append-only within the major |
| Source | `crates/webengine/src/engine/lib.rs` (not a module of `webengine`; counted by its unsafe ratchet row), manifest `crates/webengine/engine/Cargo.toml` (excluded from the workspace) |
| Build | `webengine-wolos` copies `crates/webengine` into the engine's workspace as `ports/wolwebengine`, builds `-p wolwebengine` under the engine's `Cargo.lock` (prepare() refuses any other change to the lock), links with `crates/webengine/engine/link.sh` |
| Threads | every call on the thread that made the engine; `wake` may be called from any thread |
| One per process | a second `wwe_engine_new` is refused ("one engine per process") |
| Errors | NULL or -1 and one sentence in the caller's `err` buffer |
| Callbacks | `allow` (required, every navigation and resource load), `wake`, `clip_get`, `clip_set`, all with the host's `ctx`; WWE_1.2 `load` (one `fed:` load with its kind, on the library's scheme workers, may block) and `map` (an http(s) page URL spelled as `fed://`, engine thread, never blocks) |

## The host process of a system web app (`wol-webhost`, ADR 0048)

`wol-webhost /usr/share/wolos/webapps/<slug>/app.toml` (`src/bin/wol-webhost.rs`; the launcher entry's `Exec=`) is one app: it loads the manifest, enters the app's scope (it re-executes under `systemd-run --scope` once, same pid, unit `app-wol-<app id>-<pid>-<n>.scope`), opens a Wolkit window titled with the app's name holding one `surface` node, runs the page under `Manifest::policy`, and answers the page's `window.wolos` calls. It is also the engine's content process (`--content-process TOKEN` first). No shell is started and nothing from the page or the manifest is run.

| Part | Where | Means |
|---|---|---|
| Channel | `webengine::App::channel` (`Channel {origin, init, on_message}`), `WebView::post_message`, `WebView::has_channel` | the page's `window.__wolosHost` over the library's WWE_1.1 pair |
| Pump | `webengine::host::Host` | page message to `Bridge` (refuses what the app did not declare or hold), an allowed call on a worker thread (at most 8 in flight, then `unavailable`), the reply queued for the window loop; no text over 64 KiB either way |
| wold | `host::WoldClient` | one `webapp-call <method> <params>` request per call on `/var/run/wold.sock` after the hello of ADR 0033; the app id is not in the request |
| Start-up | `host::initial_grants`, `host::locale` | a system app starts with what it declares, an app of others with the consent grants for its folder; locale from `LC_ALL`, `LC_MESSAGES`, `LANG` |
| Window | `webengine::window` | `native_engine` opts in to Wolkit's native Wayland backend and sets its session (the host calls `Engine::new` itself, not the C entry point that reads `WOLKIT_BACKEND`; without the opt-in `create_window` fails with "The WolOS desktop is not running"); `surface_patch` is the one `surface` node filling the window, bound to the events the page needs (CLICK, POINTER_ENTER and LEAVE, KEY, SCROLL, FOCUS, SURFACE_RESIZED), focusable and focused from the first frame |
| Loop | `wol-webhost` `'run`, `window::wait_ms`, `App::on_wake`, `Host::set_notify`, `wolkit::engine::Engine::wake` | the loop sleeps in `Engine::dispatch(-1)` and is woken by a window event, by the page engine's `wake` (any of its threads) or by a worker's reply from wold; a system app has no timer at all, another app wakes every 2 s for a changed grant. It used to poll `dispatch(8)`: 125 wakeups a second by construction |
| Frames | `src/engine/frame.rs`, `Delegate::notify_new_frame_ready` | each new frame is made current, painted into the back buffer and presented (the Wayland surface gets its buffer); painting alone left the window blank while the engine reported `load=3 frames=5` |

Tests without the engine: `tests/engine_channel.rs` (the injected script in a JavaScript interpreter), `tests/host_path.rs` (the real `bridge.js` and the Hello app's calls through the shim's scripts, the host and recorded answers), `tests/shim_loader_10.rs` and `tests/shim_loader_11.rs` (the loader and the view against a C stand-in library, with and without the channel; the second also checks that the engine's wake reaches `App::on_wake` from the engine's thread), `tests/host_window.rs` (the native opt-in, and the page surface taking keys), `tests/engine_frame.rs` (a new frame is painted and then presented).

## The origin gate (`src/sandbox.rs`)

| Load | Allowed when |
|---|---|
| `https://h[:p]`, `wss://` | the policy names `https://h[:p]`, or `https://*.base` and `h` is a subdomain of `base` |
| `http://`, `ws://` | the policy names `http://h[:p]` |
| `file:///path` | the policy names a folder `file:///dir/` and the path is inside it; never the profile directory (refused when the policy is read) |
| `about:blank`, `about:srcdoc` | always |
| `data:` | as a resource, never as a navigation |
| `blob:<origin>/...` | when its origin is allowed |
| `fed://label.tld/...` | the policy names `fed://label.tld` (ADR 0052) |
| anything else | never |

A host or path prefix in `telemetry_off` (`hosts`, `beacon_paths`; a path covers itself and what is below it) is refused even inside an allowed origin. A refused navigation of the page ends with the page as it was (the shim answers `204 No Content`) and is kept for the host (`WebView::take_refused`); a refused frame gets an empty `403`; a refused resource is a network error. Refused loads are counted (`WebView::refused_loads`).

## Federate Network addresses (`src/fed.rs`, ADR 0052)

Every view serves `fed://label.tld/path` natively: no setting, no extension. The library registers the `fed` scheme with the engine when the engine is made (`src/engine/scheme.rs`, std only); `WebView::new` hands it `fed::load_cb` and `fed::map_cb` whenever the library has WWE_1.2. Each load runs the `federate` crate on its own thread: the signed root zone (pinned root key), the TLD record, the domain record, the owner-signed manifest and the content block, every signature and BLAKE3 address checked before a byte is served; a failed check is a page naming the layer and the reason (502), never the content. A page load of `http://home.fed` (the network's compatibility spelling, a TLD of the verified zone) ends with `204` and loads `fed://home.fed`. The load of `fed://home.fed` is started from the engine's spin and tried again every 100 ms (at most 5 s) until the `fed:` handler is asked for a document: the engine drops an embedder load while the navigation the 204 ended is still pending (`Constellation::load_url`), and nothing reports when that is over. Which names an app may open is still its policy's (`fed://label.tld` origins in the gate). Tests without the engine: `tests/engine_scheme.rs` (the library's half) and `tests/shim_loader_12.rs` (the loader, the view and the host's handler against the C stand-in, on the network's captured data); the chain itself is `cargo test -p federate`.

## Process model and sandbox

- One app = one systemd scope and one host process: the host (`org.wolos.webapp.<slug>`, `process::app_id`) is started by `process::scope_argv` in the user manager's scope `app-wol-<app id>-<pid>.scope` (`process::scope_unit`), the name the Memory Saver gives its own app scopes.
- Content processes per site, inside that scope: the engine starts one per registrable domain of the app's frames (each `file:` document counts as its own), each a child of the host started as `<host> --content-process TOKEN`. A page whose frames are one site has exactly one (`tests/content_process.rs`).
- The Memory Saver and the frontmost QoS act on the scope: they see and move the whole app, whatever its number of content processes.
- The content process is confined by the engine library before it runs a line of the page (`src/engine/confine.rs`, ADR 0042): `no_new_privs`, a Landlock ruleset and a seccomp filter. It reads the system (`/usr`, the fonts, a few files of `/etc`, its own `/proc`, the CPU and cgroup facts of `/sys`) and nothing of the user's: no home, no profile directory (storage, cookies and `cache/` are the host's), no `/tmp`; it writes only `/dev/null`. No TCP, no UDP, no raw or packet socket: `socket` and `socketpair` are `AF_UNIX` only, so the network is the host's; with Landlock `RESOLVE_UNIX` the one pathname socket it reaches is the engine's bootstrap socket. No `execve`, `ptrace`, mount, namespace, `bpf`, `io_uring` or `perf_event_open`. The host sets `ffi::SANDBOX` with `ffi::MULTIPROCESS` (`view::engine_flags`); the library refuses one without the other.
- It fails closed: Landlock ABI 6 (Linux 6.12) or newer and an architecture with a filter table (x86-64, AArch64) or `WebView::new` returns the library's error; a content process that cannot be confined prints why and returns without running the page. A host's `main` calls `process::content_token` and `ffi::run_content_process` first, while it has one thread (the confinement needs that).
- Residual: below Landlock ABI 9 a pathname unix socket of the user (session bus) is reachable from the content process; the library prints a line when that is so. `confine::REQUIRE_UNIX_RESOLVE` makes it a refusal.
- The engine's own content sandbox stays off: at the pin it never loads a page (forge, 2026-11-03). The scope (`systemd-run --scope` takes cgroup properties only, not `ProtectHome` or `SystemCallFilter`), the origin gate and the profile directory bound the host process.

## Policy (`webengine::Policy`, frozen)

`origins`, `heap_mb` (the script heap cap, MiB), `telemetry_off` (`hosts`, `beacon_paths`), `background_throttle` (a hidden window is throttled by the engine; `WebView::set_visible`), `features`. Switchable features at the pin: `webgl2`, `webgpu`, `service-workers`, `indexeddb`, `notifications`, `webrtc` (`policy::FEATURES`); others are reported by `Policy::unswitchable` for the probes to judge. As JSON an unknown field is refused.

## Input (`src/input.rs`)

| wolkit record (surface node) | Page |
|---|---|
| CLICK | move, press, release at the point |
| POINTER_ENTER, POINTER_LEAVE | move to the point, leave |
| KEY | key value and code from the keysym (US layout for printable codes: the record has no scancode), modifiers, repeat |
| SCROLL (the surface node binds it) | the wheel at the pointer: `f32 dx, dy` as the compositor sends them (down is positive), then `f32 x, y`; the engine's wheel runs the other way, so the bridge negates. The queue adds the steps of two pending records of one node instead of replacing one with the other. The keys need nothing more from the host: the engine scrolls on PageDown (measured in the VM), and by its source on Space, the arrows, Home and End, once the key reaches the focused page |
| FOCUS, SURFACE_RESIZED, THEME_CHANGED | focus, size and scale, dark or light |

## Gaps

| Gap | Where it is fixed |
|---|---|
| Pointer motion inside a surface and separate button presses (a click is move, press, release at once; the wheel is carried since ENGINE-FIX) | wolkit record v1 (the engine side takes them: `input::Input::{MouseMove, MouseButton}`) |
| IME preedit and commit for a surface; an app clipboard | wolkit (`input::Input::{ImePreedit, ImeCommit}`, `view::Clipboard`) |
| Key to repaint (about 50 to 100 ms in the VM against 22 ms in Chromium) and scroll frame interval (about 50 ms against 27) in the no-GPU test VM | not the host or the shim: a profile of the interaction (perf, all CPUs, 2026-11-04) puts 92% of the samples in the compositor's llvmpipe threads (which burns 250% CPU of the 4 vCPUs with no app open) and under 1% in `wol-webhost`; the page's own WebRender also rasterises in llvmpipe there. Revisit on hardware with a GPU |
| Pathname unix sockets of the user below Landlock ABI 9 | the kernel (`RESOLVE_UNIX`); `confine::REQUIRE_UNIX_RESOLVE` |
| Media (`<video>`, audio) | the shim is built without the engine's media feature; WA-6's probes |
| `fed:` through the real engine (the shim's `HostScheme` and the http(s) mapping) | built at the pin and run on the forge guest (ENGINE-FED, 2026-11-05): `fed://home.fed`, `fed://lobo.pagina`, `fed://voce.pagina` open from the live network, a changed byte shows the failing layer, and a page's link to `http://home.fed` opens `fed://home.fed`. A view's start URL `http://name.tld` is mapped by the host before the policy check (`view.rs`, `checked_start`), so a policy names `fed://name.tld` only |
| Mapping `http(s)://name.tld` for frames and subresources | not done: only a top-level page load is mapped (the engine's `load_web_resource` offers no frame mapping, and a frame's navigation request cannot be told from the page's), so an `<iframe src="http://home.fed">` is an ordinary web load |
| A 'navigation settled' signal from the engine | the real fix for the mapped load's retry in the `fed:` section above: Servo reports nothing when the 204-ended navigation is over, so the shim retries `webview.load` every 100 ms for 5 s (`scheme::Retry`, tested with a short deadline). An upstream embedder signal would replace the retry; until then a load that never takes leaves the page as it was |
| Federate's native transport (TCP 4077), delegated TLD registries, a disk cache of verified data | ADR 0052 follow-ups; today the HTTPS door and root-managed TLDs |

## Verify

```sh
cargo test -p webengine                       # unit tests; the engine tests need the library:
WOL_WEBENGINE_LIB=/path/libwolwebengine.so.1 WOL_WEBENGINE_REQUIRED=1 cargo test -p webengine
WOL_BLESS=1 ... cargo test -p webengine --test headless_golden   # rewrite tests/golden/golden.png
cargo test -p shared --test architecture --test unsafe_ratchet
cargo deny check
```

Bumping the pin: change `_engine_rev`, `_mozjs` and both checksums in `webengine-wolos/PKGBUILD`, the version string in `src/engine/lib.rs`, the THIRD-PARTY section, then build the package (its check() runs the engine tests) and bless the golden if the rendering moved.
