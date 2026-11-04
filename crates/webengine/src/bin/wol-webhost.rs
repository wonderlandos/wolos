// SPDX-License-Identifier: BSD-2-Clause
//! `wol-webhost <path/to/app.toml>` (ADR 0048): the host process of one system web app. It loads
//! the app's manifest, takes the app's scope (ADR 0042), opens a Wolkit window titled with the app's
//! name, runs the page in the engine under the app's policy, and carries the page's `window.wolos`
//! calls to wold as `webapp-call` requests, after refusing early what the app did not declare.
//!
//! It also is the engine's content process: the engine starts that as `<this executable>
//! --content-process TOKEN`, and `main` hands it to the library before anything else.
//!
//! No shell, no command from the page or the manifest is ever run. The only program it starts is
//! `systemd-run`, with arguments built from its own path and the manifest's path, to enter the app's
//! scope. A page the library cannot carry a channel for (an engine library of the 1.0 build) is
//! opened in development mode (`?dev`), where every call fails with a sentence.
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use webengine::bridge::manifest::{KIT, Manifest, SYSTEM_DIR};
use webengine::bridge::{AppInfo, Bridge, Theme, accent_id};
use webengine::host::{Host, WoldClient, initial_grants, locale};
use webengine::{App, Channel, Target, WebView, ffi, input, process};
use wolkit::event::{HEADER, bit, kind};
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::{DIM_PERCENT, id};

/// Set on the re-exec into the app's scope, so a system without a user manager does not loop.
const SCOPED: &str = "WOL_WEBHOST_SCOPED";
/// First window size (device-independent pixels).
const SIZE: (u32, u32) = (1040, 720);
/// The Wolkit node of the page.
const SURFACE: u32 = 1;
/// How often the consent file is looked at for a changed grant.
const GRANTS_EVERY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // The engine library, from a build tree when the variable names one.
    let lib = std::env::var_os(ffi::LIB_ENV).map(PathBuf::from);
    if let Some(token) = process::content_token(&args) {
        // First thing, while this process has one thread (the confinement needs that).
        if let Err(e) = ffi::load(lib.as_deref()).and(ffi::run_content_process(token)) {
            eprintln!("wol-webhost: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    match run(&args, lib.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wol-webhost: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `file://<dir>/<entry>` for the page.
fn page_url(dir: &Path, entry: &str, dev: bool) -> String {
    let base = format!("file://{}/{entry}", dir.display());
    if dev { format!("{base}?dev") } else { base }
}

/// Enters the app's scope by re-executing under `systemd-run` (same pid, so the scope's name carries
/// it), unless this process already is in it. Returns only when it is (or cannot be, with
/// [`SCOPED`] set, where the host goes on and wold will not answer).
fn ensure_scope(id: &str, name: &str, manifest_arg: &str) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let pid = std::process::id();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if wol_core::sysapp::app_of_cgroup(&cgroup, i32::try_from(pid).unwrap_or(0)).is_some() {
        return Ok(());
    }
    if std::env::var_os(SCOPED).is_some() {
        eprintln!("wol-webhost: no app scope (no user manager?): wold will not answer this app");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    let unit = process::pick_unit(id, pid, process::unit_exists);
    let host = vec![exe.to_string_lossy().into_owned(), manifest_arg.to_owned()];
    let argv = process::scope_argv(&unit, name, &host);
    let err = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env(SCOPED, "1")
        .exec();
    Err(format!("systemd-run: {err}"))
}

fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(d) => d.into(),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    }
}

fn data_home() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => d.into(),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"),
    }
}

/// What the page hears of the pointer, from the surface node: a click, motion (the position on each
/// move), leaving, and the wheel (QA-WEB-13: wolkit sends a part only the events it asked for, and
/// this one asked for none, so the page got no pointer at all).
const SURFACE_EVENTS: u32 = bit(kind::CLICK)
    | bit(kind::POINTER_ENTER)
    | bit(kind::POINTER_LEAVE)
    | bit(kind::SCROLL)
    | bit(kind::FOCUS);
/// What the page hears of the keyboard, from the window: nothing in the page takes wolkit's focus,
/// so a key goes to the window node.
const WINDOW_EVENTS: u32 = bit(kind::KEY);

/// The patch that gives the window one `surface` node filling it, and asks for the input above.
fn surface_patch() -> Vec<u8> {
    let full = [DIM_PERCENT, 100f32.to_bits()];
    let mut p = Writer::new();
    p.create(SURFACE, Kind::Surface).insert(0, SURFACE, APPEND);
    p.set_prop(SURFACE, id::WIDTH, &full);
    p.set_prop(SURFACE, id::HEIGHT, &full);
    p.bind_event(SURFACE, SURFACE_EVENTS);
    p.bind_event(0, WINDOW_EVENTS);
    p.finish(1).to_vec()
}

fn run(args: &[String], lib: Option<&Path>) -> Result<(), String> {
    let arg = args.get(1).ok_or("usage: wol-webhost <path/to/app.toml>")?;
    let toml = std::fs::canonicalize(arg).map_err(|e| format!("{arg}: {e}"))?;
    if toml.file_name().and_then(|n| n.to_str()) != Some(webengine::bridge::manifest::FILE) {
        return Err(format!(
            "{arg}: not an {}",
            webengine::bridge::manifest::FILE
        ));
    }
    let dir = toml.parent().ok_or("no folder")?.to_path_buf();
    let manifest = Manifest::load(&dir)?;
    let root = dir.parent().ok_or("no apps folder")?;
    let kit = root.join(KIT);
    if !kit.is_dir() {
        return Err(format!("{}: the kit is missing", kit.display()));
    }
    let system = std::fs::canonicalize(SYSTEM_DIR).is_ok_and(|s| s == root);
    ensure_scope(&manifest.id, &manifest.name, &toml.to_string_lossy())?;
    // One host per app (QA-WEB-10): after the re-exec into the scope, whose exec closes a lock
    // taken before it. A second launch raises the window of the first and ends.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let _single = match process::claim(runtime.as_deref(), &manifest.id) {
        process::Single::First(lock) => Some(lock),
        process::Single::Running => {
            let argv = process::raise_argv(&manifest.id);
            let _ = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();
            return Ok(());
        }
        process::Single::Unchecked(why) => {
            eprintln!(
                "wol-webhost: could not tell whether {} runs already: {why}",
                manifest.id
            );
            None
        }
    };
    let policy = manifest.policy(&dir, &kit);
    let env = |k: &str| std::env::var(k).ok();

    // Grants the host starts with; wold decides each call again.
    let consent = |_: ()| {
        std::fs::read_to_string(wol_core::consent::FILE)
            .map(|t| wol_core::consent::parse(&t))
            .unwrap_or_default()
    };
    let folder = dir.to_string_lossy().into_owned();
    let grants = |c: &[(String, String)]| initial_grants(&manifest.permissions, system, &folder, c);
    let info = |dark: bool, accent: &str| AppInfo {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        locale: locale(&env),
        theme: if dark { Theme::Dark } else { Theme::Light },
        accent: accent.to_owned(),
    };
    // The look the toolkit paints now (QA-WEB-2: the page started light violet whatever was set).
    let look = wolkit::window::native::theme::read(
        &config_home(),
        wolkit::window::native::sys::local_time(),
    )
    .env;
    let start = info(look.dark, accent_id(look.dark, look.accent_rgb));
    let bridge = Bridge::new(manifest.clone(), start, &grants(&consent(())));
    let init = bridge.init().to_string();
    let host = Rc::new(RefCell::new(Host::new(bridge, Arc::new(WoldClient::new()))));

    // The window and its page surface.
    let engine = wolkit::engine::Engine::new(&manifest.id, 0).map_err(|e| e.message)?;
    let win = engine
        .create_window(&manifest.name, SIZE.0, SIZE.1, 0)
        .map_err(|e| e.message)?;
    engine.apply(win, &surface_patch()).map_err(|e| e.message)?;
    engine.show(win).map_err(|e| e.message)?;
    let sub = engine
        .create_subsurface(win, SURFACE, SIZE.0, SIZE.1)
        .map_err(|e| e.message)?;

    // The page, with the channel when the library carries it.
    ffi::load(lib)?;
    let carries = ffi::lib()?.set_host_channel.is_some();
    if !carries {
        eprintln!(
            "wol-webhost: this engine library has no host channel (WWE_1.1): development mode"
        );
    }
    let profile = wol_core::webapps::profile_dir(&data_home(), &manifest.id);
    let origin = format!("file://{}/", dir.display());
    // The page in its own view, with the channel at every document of the app's folder: the error
    // page of the kit is shown in a view like this one, so its Try again returns to a page that
    // has the channel (QA-WEB-9).
    let make_view = |url: &str| -> Result<WebView, String> {
        let mut app = App::new(&manifest.id, profile.clone());
        let sink = host.clone();
        app.channel = Some(Channel {
            origin: origin.clone(),
            init: init.clone(),
            on_message: Box::new(move |text| {
                if let Ok(mut h) = sink.try_borrow_mut() {
                    h.on_page_message(text);
                }
            }),
        });
        // SAFETY: `sub` outlives every view: the view is dropped before the subsurface is
        // destroyed below.
        let target = unsafe { Target::surface(&sub) };
        WebView::new(&policy, app, target, url).map_err(|e| e.to_string())
    };
    let url = page_url(&dir, &manifest.entry, !carries);
    let mut view = Some(make_view(&url)?);
    let mut error_shown = false;

    let mut buf = vec![0u8; 1 << 16];
    let mut mapper = input::Mapper::default();
    let mut last_grants = Instant::now();
    let mut held = host.borrow().bridge().granted().to_vec();
    'run: loop {
        engine.dispatch(8).map_err(|e| e.message)?;
        let n = engine.read_events(&mut buf);
        let mut at = 0;
        while at + HEADER <= n {
            let len = usize::try_from(u32::from_le_bytes(
                buf[at..at + 4].try_into().unwrap_or([0; 4]),
            ))
            .unwrap_or(usize::MAX);
            if len < HEADER || at + len > n {
                break;
            }
            let record = &buf[at..at + len];
            let k = u16::from_le_bytes([record[4], record[5]]);
            if k == kind::CLOSE_REQUEST {
                break 'run;
            }
            for b in mapper.bridge(record, SURFACE) {
                if let input::Bridged::Theme { dark, accent_rgb } = &b {
                    host.borrow_mut()
                        .set_app(info(*dark, accent_id(*dark, *accent_rgb)));
                }
                if let Some(v) = &view {
                    v.apply(&b);
                }
            }
            at += len;
        }
        if let Some(v) = &view {
            v.spin();
            let _ = v.take_woken();
            for url in v.take_refused() {
                eprintln!("wol-webhost: refused to leave the app: {url}");
            }
        }
        // The page's content process crashed: a blank window says nothing (QA-WEB-9). Show the
        // kit's error page in a new view (the old one goes first: one engine at a time).
        if !error_shown && view.as_ref().is_some_and(|v| v.state().crashed > 0) {
            eprintln!("wol-webhost: the page's content process crashed");
            error_shown = true;
            view = None;
            let retry = page_url(&dir, &manifest.entry, !carries);
            let to = process::error_url(&kit, "crashed", &manifest.name, &retry);
            match make_view(&to) {
                Ok(v) => view = Some(v),
                Err(e) => eprintln!("wol-webhost: the error page could not be shown: {e}"),
            }
        }
        if !system && last_grants.elapsed() >= GRANTS_EVERY {
            last_grants = Instant::now();
            let now = grants(&consent(()));
            if now != held {
                held = now;
                host.borrow_mut().set_granted(&held);
            }
        }
        let ready = host.borrow().take_ready();
        for text in ready {
            if view.as_ref().is_some_and(|v| !v.post_message(&text)) {
                eprintln!("wol-webhost: a message could not be handed to the page");
            }
        }
    }
    // The view first, then the surface it paints into, then the window.
    drop(view);
    engine.destroy_subsurface(win, sub.id);
    engine.close_window(win);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_url_is_the_entry_in_the_apps_folder() {
        let d = Path::new("/usr/share/wolos/webapps/doctor");
        assert_eq!(
            page_url(d, "index.html", false),
            "file:///usr/share/wolos/webapps/doctor/index.html"
        );
        assert!(page_url(d, "index.html", true).ends_with("index.html?dev"));
    }

    #[test]
    fn the_surface_node_fills_the_window() {
        let bytes = surface_patch();
        assert_eq!(&bytes[..4], b"WKP1");
    }

    /// QA-WEB-13: wolkit sends a node the events it bound and no others, so the surface must ask
    /// for the pointer (motion and the wheel included) and the window for the keys, or the page
    /// hears nothing.
    #[test]
    fn the_patch_asks_for_the_pointer_and_the_keys() {
        use wolkit::scene::patch::{Op, Patch, Scratch};
        let bytes = surface_patch();
        let mut scratch = Scratch::default();
        let patch = Patch::parse(&bytes, &mut scratch).expect("a valid patch");
        let binds: Vec<(u32, u32)> = patch
            .ops()
            .filter_map(|op| match op {
                Op::BindEvent { node, mask } => Some((node, mask)),
                _ => None,
            })
            .collect();
        assert_eq!(binds, [(SURFACE, SURFACE_EVENTS), (0, WINDOW_EVENTS)]);
        for k in [
            kind::CLICK,
            kind::POINTER_ENTER,
            kind::POINTER_LEAVE,
            kind::SCROLL,
        ] {
            assert_ne!(SURFACE_EVENTS & bit(k), 0, "kind {k}");
        }
        assert_ne!(WINDOW_EVENTS & bit(kind::KEY), 0);
    }
}
