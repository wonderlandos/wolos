//! Verbs for people, scripts and helpers alike (archicture/modules/automation.md): appearance,
//! audio, display, network, focus, apps, windows, files, history/undo, restore, helpers, api, mcp.
//! Reads print JSON with `--json` or when stdout is not a terminal; setters print the new state.
//! Settings changes go through the journal (`wol undo`). All I/O goes through [`Env`].
use crate::catalog::{self, HelperVerb};
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use wol_core::automation::{self, Outcome, Policy, Record, audit, policy, tidy};
use wol_core::controls::{ControlError, Controls};
use wol_core::journal::{self, Journal};
use wol_core::theme::{ThemeCatalog, accent, active, search_paths};
use wol_core::update::be;

/// Wallpaper image path, one line, relative to the config home (read by wol-shell).
pub const WALLPAPER_FILE: &str = "wolos/wallpaper";
/// Do Not Disturb flag file (present = on), relative to the config home.
pub const DND_FILE: &str = "wol/dnd";
const IMAGE_EXT: [&str; 6] = ["png", "jpg", "jpeg", "webp", "svg", "avif"];
const URL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];
const PASSWORD_TRIES: usize = 3;

/// Handles `inv` when it is one of these verbs.
pub fn execute(inv: &Invocation, env: &mut dyn Env) -> Option<i32> {
    let json = inv.flag("json") || !env.stdout_tty();
    let a = |k: &str| inv.get(k).unwrap_or_default().to_string();
    Some(match inv.action {
        "api" => api(env, json),
        "mcp" => crate::mcp::serve(env),
        "theme.get" => get(env, |e| theme_state(e, false), json),
        "appearance.get" => get(env, |e| theme_state(e, true), json),
        "appearance.set" => appearance_set(env, &a("mode"), json),
        "accent.set" => accent_set(env, &a("accent"), json),
        "wallpaper.get" => get(env, wallpaper_state, json),
        "wallpaper.set" => wallpaper_set(env, &a("path"), json),
        "audio.get" => get(env, audio_state, json),
        "audio.volume" => match percent(&a("percent"), 0) {
            Some(p) => control(env, json, audio_state, move |c| c.set_volume(p)),
            None => fail(env, ErrorCode::Usage, "<percent> must be 0..100"),
        },
        "audio.mute" | "audio.unmute" => {
            let m = inv.action == "audio.mute";
            control(env, json, audio_state, move |c| c.set_muted(m))
        }
        "display.get" => get(env, display_state, json),
        "display.brightness" => match percent(&a("percent"), 1) {
            Some(p) => control(env, json, display_state, move |c| c.set_brightness(p)),
            None => fail(env, ErrorCode::Usage, "<percent> must be 1..100"),
        },
        "battery" => get(env, battery_state, json),
        "network" => get(env, network_state, json),
        "wifi.get" => get(env, wifi_state, json),
        "wifi.on" | "wifi.off" => {
            let on = inv.action == "wifi.on";
            with_password(env, json, wifi_state, &move |c, pw| c.set_wifi(on, pw))
        }
        "wifi.join" => wifi_join(env, &a("ssid"), json),
        "bluetooth.get" => get(env, bluetooth_state, json),
        "bluetooth.on" | "bluetooth.off" => {
            let on = inv.action == "bluetooth.on";
            with_password(env, json, bluetooth_state, &move |c, pw| {
                c.set_bluetooth(on, pw)
            })
        }
        "focus.get" => get(env, focus_state, json),
        "focus.on" | "focus.off" => {
            let text = (inv.action == "focus.on").then_some("");
            save(env, "focus", DND_FILE, text, json, focus_state)
        }
        "settings" => get(env, settings_state, json),
        "open" => open(env, &a("target")),
        "notify" => notify(env, &a("title"), &inv.all("body").join(" ")),
        "screenshot" => screenshot(env, json),
        "windows.list" => windows(env, json),
        "windows.focus" => windows_focus(env, &a("window")),
        "files.tidy" => files_tidy(env, inv.get("plan"), json),
        "history" => history(env, json),
        "undo" => undo(env, inv.get("id"), json),
        "restore.list" => restore_list(env, json),
        "restore.activate" => restore_activate(env, &a("be")),
        "helpers.status" => get(env, helpers_state, json),
        "helpers.on" | "helpers.off" => {
            let on = inv.action == "helpers.on";
            helpers_change(env, json, |p| {
                p.enabled = on;
                Ok(())
            })
        }
        "helpers.allow" | "helpers.deny" => {
            let (cap, allow) = (a("capability"), inv.action == "helpers.allow");
            helpers_change(env, json, |p| p.set(&cap, allow).map_err(|e| e.to_string()))
        }
        "helpers.forget" => {
            let h = a("helper");
            helpers_change(env, json, |p| {
                p.forget(&h);
                Ok(())
            })
        }
        "helpers.log" => helpers_log(env, json),
        _ => return None,
    })
}

// ---- output ----

fn get(env: &mut dyn Env, state: fn(&mut dyn Env) -> Value, json: bool) -> i32 {
    let v = state(env);
    show(env, v, json)
}

fn show(env: &mut dyn Env, v: Value, json: bool) -> i32 {
    if json {
        emit_json(env, &v);
    } else {
        env.out(&crate::run::plain(&v));
    }
    0
}

fn percent(s: &str, min: u8) -> Option<u8> {
    s.parse::<u8>().ok().filter(|p| (min..=100).contains(p))
}

fn control_error(env: &mut dyn Env, e: ControlError) -> i32 {
    let (code, msg) = match e {
        ControlError::Unavailable => (ErrorCode::Unavailable, "not available on this computer"),
        ControlError::NeedsAuth => (ErrorCode::Denied, "administrator password required"),
        ControlError::Denied => (ErrorCode::Denied, "incorrect password"),
        ControlError::NotAdmin => (
            ErrorCode::Denied,
            "only administrators can make this change",
        ),
        ControlError::Invalid => (
            ErrorCode::Invalid,
            "the change was refused (invalid request)",
        ),
        ControlError::Failed => (ErrorCode::Failed, "the change did not work"),
        ControlError::WrongPassword => (
            ErrorCode::Denied,
            "the network refused the password; check it and try again",
        ),
    };
    fail(env, code, msg)
}

/// User-level control change, then print the new state.
fn control(
    env: &mut dyn Env,
    json: bool,
    state: fn(&mut dyn Env) -> Value,
    f: impl Fn(&Controls) -> Result<(), ControlError>,
) -> i32 {
    match f(&env.controls()) {
        Ok(()) => get(env, state, json),
        Err(e) => control_error(env, e),
    }
}

/// Change that may need the administrator password (wol-admind on FreeBSD): password from stdin,
/// up to [`PASSWORD_TRIES`] times. Helpers on `wol mcp` never get here: they get the on-screen prompt.
fn with_password(
    env: &mut dyn Env,
    json: bool,
    state: fn(&mut dyn Env) -> Value,
    f: &dyn Fn(&Controls, &str) -> Result<(), ControlError>,
) -> i32 {
    let c = env.controls();
    let mut pw = String::new();
    for _ in 0..=PASSWORD_TRIES {
        let r = f(&c, &pw);
        wipe(&mut pw);
        match r {
            Ok(()) => return get(env, state, json),
            Err(e @ (ControlError::NeedsAuth | ControlError::Denied)) => {
                if e == ControlError::Denied {
                    env.err("wol: incorrect password\n");
                }
                match env.stdin_line() {
                    Some(p) if !p.is_empty() => pw = p,
                    _ => {
                        return fail(
                            env,
                            ErrorCode::Denied,
                            "administrator password required (on stdin)",
                        );
                    }
                }
            }
            Err(e) => return control_error(env, e),
        }
    }
    fail(env, ErrorCode::Denied, "incorrect password")
}

fn wipe(s: &mut String) {
    let mut b = std::mem::take(s).into_bytes();
    crate::authcheck::wipe(&mut b);
}

fn journal_of(env: &dyn Env) -> Journal {
    Journal::new(&env.state_home(), &env.config_home())
}

/// Journaled settings file change, then print the new state (+ `undo` id).
fn save(
    env: &mut dyn Env,
    what: &str,
    rel: &str,
    text: Option<&str>,
    json: bool,
    state: fn(&mut dyn Env) -> Value,
) -> i32 {
    let by = env.actor();
    match journal_of(env).save(what, &by, &[(rel, text)], env.now_secs()) {
        Ok(id) => {
            let mut v = state(env);
            if let (Some(id), Some(o)) = (id, v.as_object_mut()) {
                o.insert("undo".into(), id.into());
            }
            show(env, v, json)
        }
        Err(e) => fail(env, ErrorCode::Failed, &format!("{rel}: {e}")),
    }
}

/// Best effort: tell a running wol-shell to reread its files.
fn shell_reload(env: &mut dyn Env) {
    if let Some(rt) = env.var("XDG_RUNTIME_DIR") {
        let _ = env.request(&Path::new(&rt).join(crate::run::SHELL_SOCKET), b"reload\n");
    }
}

// ---- appearance ----

fn catalog(env: &dyn Env) -> ThemeCatalog {
    accent::variants().into_iter().fold(
        ThemeCatalog::load(&search_paths(&env.config_home())),
        ThemeCatalog::with,
    )
}

fn saved_theme(env: &dyn Env) -> Option<String> {
    active::get(&active::default_file(&env.config_home()))
}

/// Local hour 0..23 (auto appearance follows the time of day).
fn local_hour(t: u64) -> u8 {
    let t = t as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, a zeroed plain C struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 12;
    }
    tm.tm_hour.clamp(0, 23) as u8
}

/// Theme id the session shows now (Auto resolved by the time of day).
fn effective_theme(env: &dyn Env, saved: Option<&str>) -> String {
    let dark = accent::auto_is_dark(local_hour(env.now_secs()));
    let id = accent::effective(saved, dark);
    active::effective(id.as_deref(), dark)
}

fn theme_state(env: &mut dyn Env, with_wallpaper: bool) -> Value {
    let saved = saved_theme(env);
    let (variant, acc) = accent::split(saved.as_deref());
    let id = effective_theme(env, saved.as_deref());
    let t = catalog(env).resolve(&id).ok();
    let mode = match (accent::is_auto(saved.as_deref()), variant) {
        (true, _) | (_, None) => "auto",
        (_, Some(v)) => v.name(),
    };
    let mut v = json!({
        "theme": id,
        "name": t.as_ref().map(|t| t.name.clone()),
        "variant": t.as_ref().map(|t| t.variant.name()),
        "mode": if saved.is_some() && variant.is_none() && !accent::is_auto(saved.as_deref()) { "custom" } else { mode },
        "accent": acc,
        "dark": t.is_some_and(|t| t.variant == shared::theme::Variant::Dark),
    });
    if with_wallpaper {
        v["wallpaper"] = wallpaper_state(env)["path"].clone();
    }
    v
}

/// Writes the theme file through the journal and restyles third-party apps.
fn set_theme(env: &mut dyn Env, id: Option<String>, json: bool) -> i32 {
    let text = id.as_ref().map(|i| format!("{i}\n"));
    let code = save(
        env,
        "theme",
        shared::theme::ACTIVE_FILE,
        text.as_deref(),
        json,
        |e| theme_state(e, true),
    );
    if code == 0 {
        let now = effective_theme(env, id.as_deref());
        if let Ok(t) = catalog(env).resolve(&now) {
            crate::run::restyle_apps(env, &t);
        }
    }
    code
}

fn appearance_set(env: &mut dyn Env, mode: &str, json: bool) -> i32 {
    let saved = saved_theme(env);
    let (_, acc) = accent::split(saved.as_deref());
    let variant = match mode {
        "light" => Some(shared::theme::Variant::Light),
        "dark" => Some(shared::theme::Variant::Dark),
        _ => None,
    };
    set_theme(env, accent::theme_id(variant, acc), json)
}

fn accent_set(env: &mut dyn Env, name: &str, json: bool) -> i32 {
    let saved = saved_theme(env);
    let variant = if accent::is_auto(saved.as_deref()) {
        None
    } else {
        accent::split(saved.as_deref()).0
    };
    if !accent::ACCENTS.iter().any(|a| a.0 == name) {
        return fail(env, ErrorCode::Usage, &format!("unknown accent '{name}'"));
    }
    set_theme(env, accent::theme_id(variant, name), json)
}

fn expand(env: &dyn Env, p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => env.home().join(rest),
        None => PathBuf::from(p),
    }
}

fn wallpaper_state(env: &mut dyn Env) -> Value {
    let path = env
        .read_file(&env.config_home().join(WALLPAPER_FILE))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    json!({"path": path})
}

fn wallpaper_set(env: &mut dyn Env, path: &str, json: bool) -> i32 {
    let p = expand(env, path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !p.is_absolute() || !p.is_file() || !IMAGE_EXT.contains(&ext.as_str()) {
        return fail(
            env,
            ErrorCode::Invalid,
            &format!("{path}: not an image file (png, jpg, webp, svg, avif)"),
        );
    }
    let text = format!("{}\n", p.display());
    let code = save(
        env,
        "wallpaper",
        WALLPAPER_FILE,
        Some(&text),
        json,
        wallpaper_state,
    );
    if code == 0 {
        shell_reload(env);
    }
    code
}

// ---- audio, display, power, network ----

fn audio_state(env: &mut dyn Env) -> Value {
    let c = env.controls();
    let v = c.volume();
    json!({
        "volume": v.map(|v| v.percent),
        "muted": v.map(|v| v.muted),
        "outputs": c.outputs(),
    })
}

fn display_state(env: &mut dyn Env) -> Value {
    let c = env.controls();
    json!({"brightness": c.brightness(), "monitors": c.monitors()})
}

fn battery_state(env: &mut dyn Env) -> Value {
    match wol_core::power::battery(&*env.controls().host) {
        Some(b) => {
            let mut v = serde_json::to_value(b).unwrap_or_default();
            v["present"] = true.into();
            v
        }
        None => json!({"present": false}),
    }
}

fn wifi_state(env: &mut dyn Env) -> Value {
    let c = env.controls();
    json!({"on": c.wifi_enabled(), "networks": c.wifi_networks()})
}

fn bluetooth_state(env: &mut dyn Env) -> Value {
    json!({"on": env.controls().bluetooth()})
}

fn network_state(env: &mut dyn Env) -> Value {
    let c = env.controls();
    let info = wol_core::netinfo::read(&*c.host);
    let joined = c.wifi_networks().into_iter().find(|n| n.connected);
    json!({
        "online": info.gateway.is_some(),
        "gateway": info.gateway,
        "dns": info.dns,
        "interfaces": info.interfaces,
        "wifi": {"on": c.wifi_enabled(), "network": joined.as_ref().map(|n| n.ssid.clone()), "signal": joined.map(|n| n.signal)},
        "bluetooth": c.bluetooth(),
    })
}

fn wifi_join(env: &mut dyn Env, ssid: &str, json: bool) -> i32 {
    let c = env.controls();
    let secure = c
        .wifi_networks()
        .iter()
        .find(|n| n.ssid == ssid)
        .is_none_or(|n| n.secure);
    let wifi_pw = if secure { env.stdin_line() } else { None };
    let wifi_pw = wifi_pw.filter(|p| !p.is_empty());
    if let Some(e) = wol_core::wifi::validate(ssid, wifi_pw.as_deref()) {
        return fail(env, ErrorCode::Invalid, e);
    }
    let ssid = ssid.to_string();
    with_password(env, json, wifi_state, &move |c, pw| {
        c.wifi_join(&ssid, wifi_pw.as_deref(), pw)
    })
}

fn focus_state(env: &mut dyn Env) -> Value {
    json!({"do_not_disturb": env.config_home().join(DND_FILE).exists()})
}

fn settings_state(env: &mut dyn Env) -> Value {
    use wol_core::prefs;
    let home = env.config_home();
    let read = |p: &str| env.read_file(&home.join(p));
    let shell = prefs::ShellPrefs::parse(read(prefs::SHELL_FILE).as_deref());
    let kb = read(&format!("{}/keyboard.conf", prefs::HYPR_DIR));
    let (l, r) = prefs::parse_keyboard(kb.as_deref());
    let (lock, sleep) = prefs::parse_hypridle(read(prefs::IDLE_FILE).as_deref());
    json!({
        "appearance": theme_state(env, true),
        "do_not_disturb": focus_state(env)["do_not_disturb"],
        "shell": shell,
        "keyboard": {"layout": prefs::LAYOUTS.get(l).map(|x| x.0), "repeat": prefs::REPEATS.get(r).map(|x| x.0)},
        "lock_after_secs": lock,
        "lock_on_sleep": sleep,
    })
}

// ---- apps, notifications, screen, windows ----

/// `Name=` of a desktop entry and whether it is hidden from menus.
pub fn desktop_name(text: &str) -> Option<(String, bool)> {
    let mut in_entry = false;
    let (mut name, mut hidden) = (None, false);
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
        } else if in_entry {
            if let Some(n) = l.strip_prefix("Name=") {
                name.get_or_insert_with(|| n.to_string());
            } else if l == "NoDisplay=true" || l == "Hidden=true" {
                hidden = true;
            }
        }
    }
    name.map(|n| (n, hidden))
}

/// Desktop file for an app id (`org.gnome.TextEditor`) or its visible name (`Text Editor`).
fn find_app(env: &dyn Env, app: &str) -> Option<PathBuf> {
    let home = env.home();
    let data_home = env
        .var("XDG_DATA_HOME")
        .map_or_else(|| home.join(".local/share"), PathBuf::from);
    let dirs = env
        .var("XDG_DATA_DIRS")
        .unwrap_or_else(|| "/usr/share:/usr/share".into());
    let roots = std::iter::once(data_home)
        .chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    let want = app.trim_end_matches(".desktop").to_lowercase();
    let mut by_name = None;
    for dir in roots.map(|r| r.join("applications")) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        files.sort();
        for f in files {
            let stem = f.file_stem().map(|s| s.to_string_lossy().to_lowercase());
            if stem.as_deref() == Some(want.as_str()) {
                return Some(f);
            }
            if by_name.is_none()
                && let Some((n, false)) = env.read_file(&f).as_deref().and_then(desktop_name)
                && n.to_lowercase() == want
            {
                by_name = Some(f);
            }
        }
    }
    by_name
}

fn is_url(s: &str) -> Option<&str> {
    let (scheme, _) = s.split_once(':')?;
    let ok = !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    ok.then_some(scheme)
}

fn open(env: &mut dyn Env, target: &str) -> i32 {
    if target.is_empty() || target.starts_with('-') {
        return fail(env, ErrorCode::Usage, "nothing to open");
    }
    let path = expand(env, target);
    let argv: Vec<String> = if path.is_absolute() && path.exists() {
        vec!["xdg-open".into(), path.display().to_string()]
    } else if let Some(scheme) = is_url(target) {
        if !URL_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
            return fail(
                env,
                ErrorCode::Unsupported,
                &format!("only {} links open this way", URL_SCHEMES.join(", ")),
            );
        }
        vec!["xdg-open".into(), target.into()]
    } else if let Some(f) = find_app(env, target) {
        vec!["gio".into(), "launch".into(), f.display().to_string()]
    } else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no app, file or link named '{target}'"),
        );
    };
    crate::run::exec(env, &argv)
}

/// Display name of the actor for notifications.
fn actor_name(actor: &str) -> String {
    actor
        .split_once(':')
        .map_or_else(|| "wol".to_string(), |(_, n)| n.to_string())
}

fn notify(env: &mut dyn Env, title: &str, body: &str) -> i32 {
    if title.trim().is_empty() {
        return fail(env, ErrorCode::Usage, "missing <title>");
    }
    let app = actor_name(&env.actor());
    let mut argv = vec![
        "notify-send".to_string(),
        format!("--app-name={app}"),
        "--".into(),
        title.into(),
    ];
    if !body.is_empty() {
        argv.push(body.into());
    }
    crate::run::exec(env, &argv)
}

fn screenshot(env: &mut dyn Env, json: bool) -> i32 {
    match env.capture(&["wol-screenshot", "full"]) {
        Some(p) if !p.trim().is_empty() => show(env, json!({"path": p.trim()}), json),
        _ => fail(
            env,
            ErrorCode::Unavailable,
            "screenshot failed (needs a running WolOS session)",
        ),
    }
}

/// `hyprctl -j clients` + the focused address.
pub fn parse_clients(json: &str, focused: Option<&str>) -> Vec<Value> {
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    a.iter()
        .filter(|c| c["mapped"].as_bool() != Some(false))
        .filter_map(|c| {
            let address = c["address"].as_str()?;
            Some(json!({
                "address": address,
                "app": c["class"].as_str().unwrap_or(""),
                "title": c["title"].as_str().unwrap_or(""),
                "workspace": c["workspace"]["name"].as_str().unwrap_or(""),
                "focused": Some(address) == focused,
            }))
        })
        .collect()
}

fn windows(env: &mut dyn Env, json: bool) -> i32 {
    let Some(list) = env.capture(&["hyprctl", "-j", "clients"]) else {
        return fail(
            env,
            ErrorCode::Unavailable,
            "no compositor (Hyprland) answered",
        );
    };
    let focused = env
        .capture(&["hyprctl", "-j", "activewindow"])
        .and_then(|j| serde_json::from_str::<Value>(&j).ok())
        .and_then(|v| v["address"].as_str().map(String::from));
    let w = parse_clients(&list, focused.as_deref());
    if json {
        emit_json(env, &Value::from(w));
    } else {
        for x in &w {
            env.out(&format!(
                "{} {} {:<24} {}\n",
                if x["focused"] == true { "*" } else { " " },
                x["address"].as_str().unwrap_or(""),
                x["app"].as_str().unwrap_or(""),
                x["title"].as_str().unwrap_or("")
            ));
        }
    }
    0
}

/// Hyprland window selector for an address (`0x55d1…`) or an app id.
pub fn window_selector(w: &str) -> Option<String> {
    let hex = |s: &str| !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(h) = w.strip_prefix("0x") {
        return hex(h).then(|| format!("address:0x{h}"));
    }
    let id = !w.is_empty()
        && w.len() <= 128
        && w.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    id.then(|| format!("class:^({})$", w.replace('.', "\\.")))
}

fn windows_focus(env: &mut dyn Env, w: &str) -> i32 {
    let Some(sel) = window_selector(w) else {
        return fail(env, ErrorCode::Usage, &format!("bad window '{w}'"));
    };
    match env.capture(&["hyprctl", "dispatch", "focuswindow", &sel]) {
        Some(o) if o.trim() == "ok" => 0,
        Some(o) => fail(env, ErrorCode::NotFound, o.trim()),
        None => fail(
            env,
            ErrorCode::Unavailable,
            "no compositor (Hyprland) answered",
        ),
    }
}

// ---- files ----

fn files_tidy(env: &mut dyn Env, plan_id: Option<&str>, json: bool) -> i32 {
    let home = env.home();
    let dirs = env.read_file(&env.config_home().join("user-dirs.dirs"));
    let dir = tidy::downloads_dir(&home, dirs.as_deref());
    let dir = match tidy::in_home(&home, &dir) {
        Ok(d) => d,
        Err(e) => return fail(env, ErrorCode::NotFound, &e),
    };
    let names = match tidy::scan(&dir) {
        Ok(n) => n,
        Err(e) => return fail(env, ErrorCode::Failed, &format!("{}: {e}", dir.display())),
    };
    let plan = tidy::plan(&dir, &names, &|p| p.exists());
    let Some(id) = plan_id else {
        let v = json!({"dry_run": true, "dir": plan.dir, "plan": plan.id, "moves": plan.moves});
        if json {
            emit_json(env, &v);
        } else if plan.moves.is_empty() {
            env.out(&format!("{} is tidy\n", dir.display()));
        } else {
            for m in &plan.moves {
                let rel = m.to.strip_prefix(&dir).unwrap_or(&m.to);
                env.out(&format!(
                    "{} -> {}\n",
                    m.from.file_name().unwrap_or_default().to_string_lossy(),
                    rel.display()
                ));
            }
            env.out(&format!("apply: wol files tidy {}\n", plan.id));
        }
        return 0;
    };
    if id != plan.id {
        return fail(
            env,
            ErrorCode::Invalid,
            "Downloads changed since that dry run; run `wol files tidy` again and review the new plan",
        );
    }
    let (done, err) = tidy::apply(&plan);
    let by = env.actor();
    let undo = journal_of(env).record_moves("files", &by, &done, env.now_secs());
    if let Some(e) = err {
        return fail(
            env,
            ErrorCode::Failed,
            &format!(
                "moved {} of {} files, then: {e}",
                done.len(),
                plan.moves.len()
            ),
        );
    }
    let v = json!({"moved": done.len(), "undo": undo.ok().flatten()});
    show(env, v, json)
}

// ---- history, undo ----

fn history(env: &mut dyn Env, json: bool) -> i32 {
    let all = journal_of(env).entries();
    let rows: Vec<Value> = all
        .iter()
        .rev()
        .map(|e| {
            let files: Vec<String> = e
                .changes
                .iter()
                .map(|c| match c {
                    journal::Change::File { path, .. } => path.clone(),
                    journal::Change::Move { to, .. } => to.clone(),
                })
                .collect();
            json!({"id": e.id, "t": e.t, "by": e.by, "what": e.what, "files": files, "undoes": e.undoes, "undone": journal::is_undone(&all, e.id)})
        })
        .collect();
    if json {
        emit_json(env, &Value::from(rows));
        return 0;
    }
    for r in &rows {
        let what = match r["undoes"].as_u64() {
            Some(u) => format!("undo of #{u}"),
            None => r["what"].as_str().unwrap_or("").to_string(),
        };
        env.out(&format!(
            "#{:<4} {:<16} {:<12} {}{}\n",
            r["id"],
            what,
            r["by"].as_str().unwrap_or(""),
            r["files"].as_array().map_or(0, Vec::len),
            if r["undone"] == true {
                " file(s), undone"
            } else {
                " file(s)"
            }
        ));
    }
    0
}

fn undo(env: &mut dyn Env, id: Option<&str>, json: bool) -> i32 {
    let id = match id.map(str::parse::<u64>) {
        None => None,
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => return fail(env, ErrorCode::Usage, "<id> must be a number (wol history)"),
    };
    let by = env.actor();
    let e = match journal_of(env).undo(id, &by, env.now_secs()) {
        Ok(e) => e,
        Err(journal::UndoError::Nothing) => {
            return fail(env, ErrorCode::NotFound, "nothing to undo");
        }
        Err(e @ journal::UndoError::NotFound(_)) => {
            return fail(env, ErrorCode::NotFound, &e.to_string());
        }
        Err(e) => return fail(env, ErrorCode::Denied, &e.to_string()),
    };
    let r = journal::reload_for(&e);
    if let Some(theme) = &r.theme {
        let now = effective_theme(env, theme.as_deref());
        if let Ok(t) = catalog(env).resolve(&now) {
            crate::run::restyle_apps(env, &t);
        }
    }
    if r.shell {
        shell_reload(env);
    }
    if r.hyprland {
        let _ = env.capture(&["hyprctl", "reload"]);
    }
    let v = json!({"undid": e.undoes, "what": e.what, "change": e.id});
    if json {
        emit_json(env, &v);
    } else {
        env.out(&format!(
            "undid #{} ({}); redo: wol undo {}\n",
            e.undoes.unwrap_or(0),
            e.what,
            e.id
        ));
    }
    0
}

// ---- restore points ----

fn boot_envs(env: &mut dyn Env) -> Option<Vec<be::BootEnv>> {
    env.capture(&be::LIST_ARGV).map(|t| be::parse(&t))
}

fn restore_list(env: &mut dyn Env, json: bool) -> i32 {
    let Some(list) = boot_envs(env) else {
        return fail(
            env,
            ErrorCode::Unavailable,
            "no restore points: this system does not boot WolOS root slots",
        );
    };
    let target = be::rollback_target(&list).map(|b| b.name.clone());
    let rows: Vec<Value> = list
        .iter()
        .map(|b| {
            let mut v = serde_json::to_value(b).unwrap_or_default();
            v["rollback"] = (Some(&b.name) == target.as_ref()).into();
            v
        })
        .collect();
    if json {
        emit_json(env, &Value::from(rows));
    } else {
        for b in &list {
            let flag = match (b.active, b.next_boot) {
                (true, true) => "now, next boot",
                (true, false) => "now",
                (false, true) => "next boot",
                _ => "",
            };
            env.out(&format!(
                "{:<36} {:<16} {:<16} {}\n",
                b.name, b.created, flag, b.space
            ));
        }
    }
    0
}

/// Boot `name` next, through wol-admind (password on stdin).
pub fn restore_activate(env: &mut dyn Env, name: &str) -> i32 {
    if !wol_core::update::is_valid_be(name) {
        return fail(
            env,
            ErrorCode::Usage,
            &format!("bad restore point '{name}'"),
        );
    }
    if let Some(list) = boot_envs(env)
        && !list.iter().any(|b| b.name == name)
    {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no restore point '{name}'"),
        );
    }
    let req = shared::admin::AdminRequest {
        action: shared::admin::AdminAction::BootEnvironment,
        args: vec![name.into()],
    };
    let code = crate::run::admin_send(env, &req);
    if code == 0 {
        env.out(&format!("{name} starts next time you restart\n"));
    }
    code
}

/// `wol update rollback [be]`: the named or the previous restore point.
pub fn rollback(env: &mut dyn Env, be: Option<&str>) -> i32 {
    let name = match be {
        Some(n) => n.to_string(),
        None => match boot_envs(env).as_deref().and_then(be::rollback_target) {
            Some(b) => b.name.clone(),
            None => return fail(env, ErrorCode::NotFound, "no earlier restore point"),
        },
    };
    if env.is_root() {
        return crate::run::exec(env, &be::activate_argv(&name));
    }
    restore_activate(env, &name)
}

// ---- helpers ----

fn policy_of(env: &dyn Env) -> Policy {
    Policy::parse(
        env.read_file(&env.config_home().join(policy::FILE))
            .as_deref(),
    )
}

fn helpers_state(env: &mut dyn Env) -> Value {
    let s = automation::status(&policy_of(env), &audit::read(&env.state_home()));
    serde_json::to_value(s).unwrap_or_default()
}

fn helpers_change(
    env: &mut dyn Env,
    json: bool,
    f: impl FnOnce(&mut Policy) -> Result<(), String>,
) -> i32 {
    let mut p = policy_of(env);
    if let Err(e) = f(&mut p) {
        return fail(env, ErrorCode::Usage, &e);
    }
    let text = p.to_json();
    save(
        env,
        journal::HELPERS,
        policy::FILE,
        Some(&text),
        json,
        helpers_state,
    )
}

fn helpers_log(env: &mut dyn Env, json: bool) -> i32 {
    let mut r = audit::read(&env.state_home());
    r.reverse();
    r.truncate(200);
    if json {
        emit_json(env, &serde_json::to_value(&r).unwrap_or_default());
        return 0;
    }
    for x in &r {
        let o = serde_json::to_value(x.result).unwrap_or_default();
        env.out(&format!(
            "{} {:<20} {:<24} {:<16} {}\n",
            x.t,
            x.helper,
            format!("{} {}", x.tool, x.args.join(" ")),
            o.as_str().unwrap_or(""),
            x.detail
        ));
    }
    0
}

/// Audit record for one helper action (args never include secrets).
pub fn record(
    helper: &str,
    via: &str,
    tool: &str,
    args: &[&str],
    result: Outcome,
    detail: &str,
    t: u64,
) -> Record {
    Record {
        t,
        helper: helper.into(),
        via: via.into(),
        tool: tool.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        result,
        detail: detail.into(),
    }
}

/// A named script (`WOL_HELPER=<name>`) calling a helper verb: the same gate as `wol mcp` clients
/// ([`crate::approve`]): policy, the approval sheet for changes, the password sheet for
/// administrator verbs. Changes are logged.
pub fn as_script(inv: &Invocation, env: &mut dyn Env, name: &str, rule: &HelperVerb) -> i32 {
    use crate::approve::{self, Verdict};
    let tool = inv.path.join(" ");
    let args: Vec<&str> = inv.args.iter().map(|a| a.1.as_str()).collect();
    let helper = format!("script:{name}");
    let state = env.state_home();
    let log = |env: &mut dyn Env, o: Outcome, detail: &str| {
        let rec = record(&helper, "cli", &tool, &args, o, detail, env.now_secs());
        let _ = audit::append(&state, &rec);
    };
    let changes = catalog::changes(inv);
    // The Wi-Fi password of `wifi join` comes on stdin, as for people.
    let secret = match rule.secret {
        Some(_) if changes => env.stdin_line().filter(|p| !p.is_empty()),
        _ => None,
    };
    let (approved, note) = match approve::decide(env, &helper, &tool, rule, inv, secret) {
        Verdict::Stop(o, msg) => {
            log(env, o, &msg);
            let code = match o {
                Outcome::Failed => ErrorCode::Failed,
                _ if msg == approve::NO_SESSION => ErrorCode::Unavailable,
                _ => ErrorCode::Denied,
            };
            return fail(env, code, &msg);
        }
        Verdict::Done(what) => {
            log(env, Outcome::Approved, &what);
            if inv.flag("json") {
                emit_json(env, &json!({"status": "done", "request": what}));
            } else {
                env.out(&format!("done: {what}\n"));
            }
            return 0;
        }
        Verdict::Run { approved, note } => (approved, note),
    };
    let code = crate::run::dispatch(inv, env);
    if changes {
        let o = match (code, approved) {
            (0, true) => Outcome::Approved,
            (0, false) => Outcome::Ok,
            _ => Outcome::Failed,
        };
        log(env, o, note);
    }
    code
}

// ---- api ----

fn api(env: &mut dyn Env, json: bool) -> i32 {
    if json {
        emit_json(env, &catalog::api_json());
        return 0;
    }
    let mut s = String::from(
        "Helpers (wol mcp, scripts) may call the verbs marked with an access level.\n\n",
    );
    for v in catalog::verbs() {
        let access = match v.helper {
            Some(h) => format!("{:?}/{}", h.access, h.capability).to_lowercase(),
            None => "-".into(),
        };
        s.push_str(&format!(
            "  {:<44} {:<22} {}\n",
            v.usage(),
            access,
            v.cmd.help
        ));
    }
    env.out(&s);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_names() {
        assert_eq!(
            desktop_name("[Desktop Entry]\nName=Text Editor\nName[pt]=Editor\nExec=x\n"),
            Some(("Text Editor".into(), false))
        );
        assert_eq!(
            desktop_name(
                "[Desktop Entry]\nName=Hidden\nNoDisplay=true\n[Desktop Action new]\nName=New\n"
            ),
            Some(("Hidden".into(), true))
        );
        assert_eq!(desktop_name("[Other]\nName=x\n"), None);
    }

    #[test]
    fn window_selectors_and_urls() {
        for (w, want) in [
            ("0x55d1a2", Some("address:0x55d1a2")),
            (
                "org.gnome.TextEditor",
                Some("class:^(org\\.gnome\\.TextEditor)$"),
            ),
            ("foot", Some("class:^(foot)$")),
            ("foot; exec x", None),
            ("0xzz", None),
            ("", None),
        ] {
            assert_eq!(window_selector(w).as_deref(), want, "{w}");
        }
        assert_eq!(is_url("https://wolos.org"), Some("https"));
        assert_eq!(is_url("mailto:a@b"), Some("mailto"));
        assert_eq!(is_url("Text Editor"), None);
        assert_eq!(actor_name("mcp:Claude Desktop"), "Claude Desktop");
        assert_eq!(actor_name("cli"), "wol");
    }

    #[test]
    fn clients_fixture() {
        let j = r#"[{"address":"0x1","mapped":true,"class":"foot","title":"~","workspace":{"id":1,"name":"1"}},
                    {"address":"0x2","mapped":false,"class":"x","title":"","workspace":{"name":"special:hidden"}},
                    {"address":"0x3","class":"firefox","title":"WolOS","workspace":{"name":"2"}}]"#;
        let w = parse_clients(j, Some("0x3"));
        assert_eq!(w.len(), 2, "unmapped windows hidden");
        assert_eq!(
            (w[1]["app"].as_str(), w[1]["focused"].as_bool()),
            (Some("firefox"), Some(true))
        );
        assert!(parse_clients("nope", None).is_empty());
    }
}
