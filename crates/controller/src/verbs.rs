// SPDX-License-Identifier: GPL-2.0-or-later
//! Verbs for people, scripts and helpers alike (docs/modules/automation.md): appearance,
//! audio, display, network, focus, apps, windows, files, history/undo, restore, helpers, api, mcp.
//! Reads print JSON with `--json` or when stdout is not a terminal; setters print the new state.
//! Settings changes go through the journal (`wol undo`). All I/O goes through [`Env`].
use crate::catalog::{self, HelperVerb};
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use shared::par::join;
use std::path::{Path, PathBuf};
use wol_core::automation::{self, Outcome, Policy, Record, audit, policy, tidy};
use wol_core::controls::{ControlError, Controls};
use wol_core::journal::{self, Journal};
use wol_core::theme::{ThemeCatalog, accent, active};
use wol_core::update::be;
use wol_core::{toggles, webapps};

/// Wallpaper image path, one line, relative to the config home (read by wol-shell).
pub const WALLPAPER_FILE: &str = "wolos/wallpaper";
/// Do Not Disturb flag file (present = on), relative to the config home.
pub const DND_FILE: &str = wol_core::toggles::DND_FILE;
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
            match with_wifi_cause(env, json, &move |c, pw| c.set_wifi(on, pw)) {
                Ok(code) => code,
                Err(e) => wifi_error(env, e),
            }
        }
        "wifi.join" => wifi_join(env, &a("ssid"), inv.get("hidden").is_some(), json),
        "wifi.disconnect" => wifi_disconnect(env, json),
        "wifi.hotspot" => get(env, hotspot_state, json),
        "wifi.hotspot.on" => hotspot_on(env, inv.get("name"), json),
        "wifi.hotspot.off" => control(env, json, hotspot_state, |c| c.set_hotspot(None)),
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
        "webapp.list" => webapp_list(env, json),
        "webapp.add" => webapp_add(
            env,
            &a("name"),
            &a("url"),
            inv.get("icon"),
            inv.flag("own-profile"),
            json,
        ),
        "webapp.remove" => webapp_remove(env, &a("name"), json),
        "toggle" => match inv.get("toggle") {
            None => get(env, toggles_state, json),
            Some(w) => toggle(env, w, inv.get("state").map(|s| s == "on"), json),
        },
        "remind" => remind(env, &a("minutes"), &inv.all("text").join(" "), json),
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

pub(crate) fn show(env: &mut dyn Env, v: Value, json: bool) -> i32 {
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

pub(crate) fn control_error(env: &mut dyn Env, e: ControlError) -> i32 {
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
        ControlError::NotFound => (ErrorCode::Failed, "no network of that name is in range"),
        ControlError::Unsupported => (
            ErrorCode::Unavailable,
            "this network needs a user name or a certificate (802.1X Enterprise), which WolOS cannot join yet",
        ),
    };
    fail(env, code, msg)
}

/// A Wi-Fi change that did not work, with its cause (QA-PWR-21, QA-NET-8): no device or no
/// NetworkManager is said so, never "the change did not work" with exit 0.
fn wifi_error(env: &mut dyn Env, e: ControlError) -> i32 {
    match e {
        ControlError::Unavailable => fail(
            env,
            ErrorCode::Unavailable,
            "no Wi-Fi device was found, or NetworkManager is not running",
        ),
        ControlError::Failed => fail(
            env,
            ErrorCode::Failed,
            "NetworkManager did not make the change (see `journalctl -u NetworkManager`)",
        ),
        e => control_error(env, e),
    }
}

/// A Wi-Fi change: NetworkManager lets the person at the computer make it (polkit), so no
/// administrator password is asked; a failure comes back with its cause for [`wifi_error`].
fn with_wifi_cause(
    env: &mut dyn Env,
    json: bool,
    f: &dyn Fn(&Controls, &str) -> Result<(), ControlError>,
) -> Result<i32, ControlError> {
    f(&env.controls(), "")?;
    Ok(get(env, wifi_state, json))
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
pub(crate) fn with_password(
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
        ThemeCatalog::installed(
            &env.config_home(),
            wol_core::theme::apps::data_home(|k| env.var(k)).as_deref(),
        ),
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
    let (on, networks) = join(|| c.wifi_enabled(), || c.wifi_networks());
    json!({"on": on, "networks": networks})
}

fn bluetooth_state(env: &mut dyn Env) -> Value {
    json!({"on": env.controls().bluetooth()})
}

fn network_state(env: &mut dyn Env) -> Value {
    let c = env.controls();
    // Independent probes (ip, nmcli twice, bluetoothctl): fork/join, fields keep their order.
    let ((info, networks), (on, bluetooth)) = join(
        || join(|| wol_core::netinfo::read(&*c.host), || c.wifi_networks()),
        || join(|| c.wifi_enabled(), || c.bluetooth()),
    );
    let joined = networks.into_iter().find(|n| n.connected);
    // QA-NET-28: an active tunnel is a VPN, listed with its own DNS.
    let vpn: Vec<&str> = info
        .interfaces
        .iter()
        .filter(|i| i.kind == wol_core::netinfo::IfKind::Vpn && i.up)
        .map(|i| i.name.as_str())
        .collect();
    json!({
        "online": info.online(),
        "limited": info.limited(),
        "portal": info.portal(),
        "metered": info.metered,
        "gateway": info.gateway,
        "dns": info.dns,
        "interfaces": info.interfaces,
        "vpn": vpn,
        "wifi": {"on": on, "network": joined.as_ref().map(|n| n.ssid.clone()), "signal": joined.map(|n| n.signal)},
        "bluetooth": bluetooth,
    })
}

fn wifi_join(env: &mut dyn Env, ssid: &str, hidden: bool, json: bool) -> i32 {
    let c = env.controls();
    let net = c.wifi_networks().into_iter().find(|n| n.ssid == ssid);
    if net
        .as_ref()
        .is_some_and(|n| n.security == wol_core::wifi::Security::Enterprise)
    {
        return control_error(env, ControlError::Unsupported);
    }
    let secure = net.is_none_or(|n| n.secure);
    let wifi_pw = if secure { env.stdin_line() } else { None };
    let wifi_pw = wifi_pw.filter(|p| !p.is_empty());
    if let Some(e) = wol_core::wifi::validate(ssid, wifi_pw.as_deref()) {
        return fail(env, ErrorCode::Invalid, e);
    }
    let ssid = ssid.to_string();
    // QA-NET-8: every failure exits non-zero with its own cause.
    match with_wifi_cause(env, json, &move |c, pw| {
        c.wifi_join_hidden(&ssid, wifi_pw.as_deref(), hidden, pw)
    }) {
        Ok(code) => code,
        Err(e) => wifi_error(env, e),
    }
}

/// `wol wifi disconnect`: the interface on the network now (QA-NET-17).
fn wifi_disconnect(env: &mut dyn Env, json: bool) -> i32 {
    let c = env.controls();
    let iface = wol_core::netinfo::read(&*c.host)
        .interfaces
        .into_iter()
        .find(|i| i.kind == wol_core::netinfo::IfKind::WiFi && i.up)
        .map(|i| i.name);
    let Some(iface) = iface else {
        return fail(env, ErrorCode::Unavailable, "Wi-Fi is not on a network");
    };
    match c.wifi_disconnect(&iface) {
        Ok(()) => get(env, wifi_state, json),
        Err(e) => wifi_error(env, e),
    }
}

fn hotspot_state(env: &mut dyn Env) -> Value {
    json!({"on": env.controls().hotspot(), "name": wol_core::wifi::HOTSPOT_PROFILE})
}

/// `wol wifi hotspot on [name]` (NET2-2): password on stdin; the name defaults to the computer name.
fn hotspot_on(env: &mut dyn Env, name: Option<&str>, json: bool) -> i32 {
    let c = env.controls();
    let name = name
        .map(str::to_owned)
        .or_else(|| c.hostname())
        .unwrap_or_else(|| "WolOS".into());
    let Some(pw) = env.stdin_line().filter(|p| !p.is_empty()) else {
        return fail(
            env,
            ErrorCode::Usage,
            "the hotspot password (8 to 63 characters) goes on stdin",
        );
    };
    if let Some(e) = wol_core::wifi::validate(&name, Some(&pw)) {
        return fail(env, ErrorCode::Invalid, e);
    }
    match c.set_hotspot(Some((&name, &pw))) {
        Ok(()) => get(env, hotspot_state, json),
        Err(e) => wifi_error(env, e),
    }
}

fn focus_state(env: &mut dyn Env) -> Value {
    json!({"do_not_disturb": env.config_home().join(DND_FILE).exists()})
}

fn settings_state(env: &mut dyn Env) -> Value {
    use wol_core::prefs;
    let home = env.config_home();
    let read = |p: &str| env.read_file(&home.join(p));
    let shell = prefs::ShellPrefs::parse(read(prefs::SHELL_FILE).as_deref());
    // desktop.json is the source of truth (Settings deletes keyboard.lua after its first save); the
    // setup assistant's keyboard.lua only answers before that.
    let desk = wol_core::hyprprefs::DesktopPrefs::from_files(
        read(wol_core::hyprprefs::PREFS_FILE).as_deref(),
        read(prefs::KEYBOARD_FILE).as_deref(),
    );
    let layout = desk
        .layouts
        .first()
        .and_then(|l| prefs::LAYOUTS.iter().find(|x| x.0 == l))
        .map_or(prefs::LAYOUTS[0].0, |x| x.0);
    let rep = prefs::REPEATS
        .iter()
        .find(|x| x.1 == desk.repeat_rate)
        .map_or(prefs::REPEATS[1].0, |x| x.0);
    let (lock, sleep) = prefs::parse_hypridle(read(prefs::IDLE_FILE).as_deref());
    json!({
        "appearance": theme_state(env, true),
        "do_not_disturb": focus_state(env)["do_not_disturb"],
        "shell": shell,
        "keyboard": {"layout": layout, "repeat": rep},
        "lock_after_secs": lock,
        "lock_on_sleep": sleep,
    })
}

// ---- apps, notifications, screen, windows ----

/// `Name=` of a desktop entry and whether it is hidden from menus (`wol_core::desktop` rules:
/// trimmed, first wins, an empty name is none).
pub fn desktop_name(text: &str) -> Option<(String, bool)> {
    let e = wol_core::desktop::Entry::parse(text);
    e.name().map(|n| (n.to_owned(), e.hidden()))
}

/// Desktop file for an app id (`org.gnome.TextEditor`) or its visible name (`Text Editor`).
fn find_app(env: &dyn Env, app: &str) -> Option<PathBuf> {
    let data_home = env.data_home();
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
        return fail(env, ErrorCode::Unavailable, "no compositor answered");
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

/// Compositor window selector for an address (`0x55d1…`) or an app id.
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
        None => fail(env, ErrorCode::Unavailable, "no compositor answered"),
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

/// The restore points: bootctl's slots that still have a root subvolume (QA-STORE-5: an
/// interrupted update leaves a boot entry behind; `be::real`).
fn boot_envs(env: &mut dyn Env) -> Option<Vec<be::BootEnv>> {
    // QA-LIVE4-5: an account's own `bootctl list` fails (the ESP is root's): boot-ok's list.
    let text = be::list_text(
        env.capture(&be::LIST_ARGV),
        env.read_file(Path::new(be::SLOTS_FILE)),
    )?;
    let bes = be::parse(&text);
    let top = env.capture(&be::TOP_ARGV);
    Some(be::real(bes, top.as_deref()))
}

/// Why there is nothing to list (QA-STORE-5): a one-copy install has no second copy by design.
pub const ONE_COPY_SENTENCE: &str =
    "This computer has one copy of WolOS, so there is no earlier copy to go back to.";

fn restore_list(env: &mut dyn Env, json: bool) -> i32 {
    let Some(list) = boot_envs(env) else {
        if env
            .read_file(Path::new(wol_core::update::SINGLE_COPY))
            .is_some()
        {
            return fail(env, ErrorCode::Unavailable, ONE_COPY_SENTENCE);
        }
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

// ---- web apps (ADR 0031 W5, wol_core::webapps) ----

fn webapps_of(env: &dyn Env) -> Vec<webapps::WebApp> {
    let dir = env.data_home().join("applications");
    env.list_dir(&dir)
        .iter()
        .filter_map(|f| {
            let p = Path::new(f);
            let id = webapps::id_of_file(p.file_name()?.to_str()?)?;
            webapps::parse_entry(id, &env.read_file(p)?)
        })
        .collect()
}

fn webapp_json(a: &webapps::WebApp) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "url": a.url,
        "icon": a.icon,
        "own_profile": a.profile == webapps::Profile::Own,
    })
}

fn webapp_list(env: &mut dyn Env, json: bool) -> i32 {
    let apps: Vec<Value> = webapps_of(env).iter().map(webapp_json).collect();
    show(env, json!({ "webapps": apps }), json)
}

/// Downloads the first icon that answers into the web app's icon file; None when none does.
fn fetch_icon(env: &mut dyn Env, id: &str, list: Vec<(String, &str)>) -> Option<String> {
    let data = env.data_home();
    env.make_dir(&data.join(webapps::DATA_DIR).join("icons"))
        .ok()?;
    for (url, ext) in list {
        let out = webapps::icon_path(&data, id, ext);
        if env.exec_quiet(&webapps::icon_fetch_argv(&url, &out)).ok() == Some(0) {
            return Some(out.display().to_string());
        }
        // A partial download is not an icon: try the next one, or use the fallback.
        let _ = env.remove_file(&out);
    }
    None
}

/// The icon: a file the person named, an icon theme name, an https address, or (none given)
/// the site's own icon over https; [`webapps::FALLBACK_ICON`] when there is none. A value with a
/// control character or backslash is never used (audit C6-1: a newline would add launcher keys).
fn webapp_icon(env: &mut dyn Env, app: &webapps::WebApp, given: Option<&str>) -> String {
    match given.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) if !webapps::check_icon(g) => None,
        Some(g) if g.starts_with("https://") && webapps::parse_url(g).is_ok() => {
            let ext = if g.ends_with(".svg") {
                "svg"
            } else if g.ends_with(".ico") {
                "ico"
            } else {
                "png"
            };
            fetch_icon(env, &app.id, vec![(g.to_string(), ext)])
        }
        Some(g) if g.starts_with('/') || g.starts_with("~/") => {
            let p = expand(env, g);
            let s = p.display().to_string();
            (p.is_file() && webapps::check_icon(&s)).then_some(s)
        }
        Some(g) if shared::plugins::is_entry_id(&g.to_ascii_lowercase().replace('.', "_")) => {
            Some(g.to_string())
        }
        Some(_) => None,
        None => {
            let u = webapps::parse_url(&app.url).ok();
            u.and_then(|u| fetch_icon(env, &app.id, webapps::icon_candidates(&u)))
        }
    }
    .unwrap_or_else(|| webapps::FALLBACK_ICON.into())
}

fn webapp_add(
    env: &mut dyn Env,
    name: &str,
    url: &str,
    icon: Option<&str>,
    own: bool,
    json: bool,
) -> i32 {
    let profile = if own {
        webapps::Profile::Own
    } else {
        webapps::Profile::Shared
    };
    let mut app = match webapps::WebApp::new(name, url, profile) {
        Ok(a) => a,
        Err(e) => return fail(env, ErrorCode::Invalid, &e.to_string()),
    };
    let data = env.data_home();
    let file = webapps::desktop_path(&data, &app.id);
    // Same id, ours or a file of that name that is not ours: never overwrite it.
    if webapps_of(env).iter().any(|a| a.id == app.id) || env.read_file(&file).is_some() {
        let e = webapps::Error::Exists(app.name);
        return fail(env, ErrorCode::Invalid, &e.to_string());
    }
    app.icon = webapp_icon(env, &app, icon);
    if own && let Err(e) = env.make_dir(&webapps::profile_dir(&data, &app.id)) {
        return fail(
            env,
            ErrorCode::Failed,
            &format!("web app profile folder: {e}"),
        );
    }
    let text = webapps::desktop_entry(&app, &data);
    let wrote = env
        .make_dir(&data.join("applications"))
        .and_then(|()| env.write_file(&file, &text));
    match wrote {
        Ok(()) => {
            let mut v = webapp_json(&app);
            v["file"] = file.display().to_string().into();
            show(env, v, json)
        }
        Err(e) => fail(env, ErrorCode::Failed, &format!("{}: {e}", file.display())),
    }
}

fn webapp_remove(env: &mut dyn Env, what: &str, json: bool) -> i32 {
    let apps = webapps_of(env);
    let Some(app) = webapps::find(&apps, what) else {
        let e = webapps::Error::NotFound(what.trim().into());
        return fail(env, ErrorCode::NotFound, &e.to_string());
    };
    let data = env.data_home();
    let file = webapps::desktop_path(&data, &app.id);
    if let Err(e) = env.remove_file(&file) {
        return fail(env, ErrorCode::Failed, &format!("{}: {e}", file.display()));
    }
    // Only an icon we downloaded is ours to delete; a named file or a theme icon stays.
    let icons = data.join(webapps::DATA_DIR).join("icons");
    if Path::new(&app.icon).parent() == Some(icons.as_path()) {
        let _ = env.remove_file(Path::new(&app.icon));
    }
    let mut v = webapp_json(app);
    v["removed"] = true.into();
    if app.profile == webapps::Profile::Own {
        // Cookies and sign-ins stay until the person deletes the folder.
        let dir = webapps::profile_dir(&data, &app.id);
        v["profile_kept"] = dir.display().to_string().into();
    }
    show(env, v, json)
}

// ---- toggles (wol_core::toggles) ----

fn toggle_on(env: &dyn Env, t: toggles::Toggle) -> bool {
    toggles::is_on(t, &env.config_home(), &env.state_home(), |p| {
        env.read_file(p).is_some()
    })
}

/// Is `t` on? Night Light is what hyprsunset reports, the state the Control Center tile shows
/// and sets; the flag file is the answer only while hyprsunset is not running (QA-SET-46: the tile
/// and the hotkey kept two states, so the hotkey needed two presses after a click).
fn toggle_now(env: &mut dyn Env, t: toggles::Toggle) -> bool {
    if t == toggles::Toggle::NightLight
        && let Some(on) = env
            .capture(&["hyprctl", "hyprsunset", "temperature"])
            .as_deref()
            .and_then(toggles::night_light_from)
    {
        return on;
    }
    toggle_on(env, t)
}

fn toggles_state(env: &mut dyn Env) -> Value {
    let list: Vec<Value> = toggles::Toggle::ALL
        .iter()
        .map(|t| json!({"id": t.id(), "label": t.label(), "on": toggle_now(env, *t)}))
        .collect();
    json!({ "toggles": list })
}

fn toggle(env: &mut dyn Env, word: &str, wanted: Option<bool>, json: bool) -> i32 {
    let Some(t) = toggles::Toggle::from_word(word) else {
        let msg = format!("<toggle> must be one of {}", crate::cli::TOGGLES.join(", "));
        return fail(env, ErrorCode::Usage, &msg);
    };
    let on = toggles::target(toggle_now(env, t), wanted);
    if t == toggles::Toggle::Silence {
        // Do Not Disturb keeps its journaled file, as `wol focus on|off` writes it.
        return save(env, "focus", DND_FILE, on.then_some(""), json, focus_state);
    }
    for argv in toggles::commands(t, on) {
        let code = env.exec_quiet(&argv).unwrap_or(-1);
        if code != 0 && toggles::must_succeed(t, on) {
            let program = Path::new("/usr/bin").join(toggles::NIGHT_LIGHT_PROGRAM);
            let installed = env
                .list_dir(Path::new("/usr/bin"))
                .iter()
                .any(|n| Path::new(n) == program);
            // Installed but not running (it ended, or came after login): start it.
            let started = installed
                && toggles::start_command(t, on)
                    .is_some_and(|a| env.exec_quiet(&a).unwrap_or(-1) == 0);
            if !started {
                return fail(
                    env,
                    ErrorCode::Unavailable,
                    &toggles::unavailable(t, installed),
                );
            }
        }
    }
    let flag = toggles::state_file(t, &env.config_home(), &env.state_home());
    let r = match (on, flag.parent()) {
        (true, Some(d)) => env.make_dir(d).and_then(|()| env.write_file(&flag, "")),
        (true, None) => Ok(()),
        (false, _) => env.remove_file(&flag),
    };
    if let Err(e) = r {
        return fail(env, ErrorCode::Failed, &format!("{}: {e}", flag.display()));
    }
    show(
        env,
        json!({"id": t.id(), "label": t.label(), "on": on}),
        json,
    )
}

// ---- reminders ----

/// Longest reminder text, in characters (one notification line).
const MAX_REMINDER: usize = 200;
/// Longest wait: one day, in minutes.
const MAX_REMIND_MINUTES: u32 = 1440;

/// `systemd-run` argv of a reminder: a transient user timer that survives `wol` exiting and is
/// gone after it fired (`--collect`). The text is one argument, never a shell string. Audit
/// C10-4: the notification carries `app`, the name of whoever asked for it (a helper's name, as
/// a direct `wol notify` from it would), never a plain WolOS one.
pub fn remind_argv(minutes: u32, text: &str, app: &str) -> Vec<String> {
    [
        "systemd-run",
        "--user",
        "--collect",
        "--quiet",
        &format!("--on-active={minutes}min"),
        "--timer-property=AccuracySec=1s",
        "--",
        "notify-send",
        &format!("--app-name={app}"),
        "--",
        "Reminder",
        text,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn remind(env: &mut dyn Env, minutes: &str, text: &str, json: bool) -> i32 {
    let Some(m) = minutes
        .parse::<u32>()
        .ok()
        .filter(|m| (1..=MAX_REMIND_MINUTES).contains(m))
    else {
        return fail(env, ErrorCode::Usage, "<minutes> must be 1..1440");
    };
    let text = text.trim();
    let ok = !text.is_empty()
        && text.chars().count() <= MAX_REMINDER
        && !text.chars().any(char::is_control);
    if !ok {
        let msg = "Say what to remind you of, in one short line.";
        return fail(env, ErrorCode::Usage, msg);
    }
    let app = actor_name(&env.actor());
    match env.exec_quiet(&remind_argv(m, text, &app)) {
        Ok(0) => {
            let at = env.now_secs().saturating_add(u64::from(m) * 60);
            show(env, json!({"minutes": m, "text": text, "at": at}), json)
        }
        _ => {
            let msg = "Reminders need the desktop session; log in and try again.";
            fail(env, ErrorCode::Unavailable, msg)
        }
    }
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
        // One reader (dedup A7): trimmed, an empty name is none, `NoDisplay = true` hides.
        assert_eq!(
            desktop_name("[Desktop Entry]\nName= Mail \nNoDisplay = true\n"),
            Some(("Mail".into(), true))
        );
        assert_eq!(desktop_name("[Desktop Entry]\nName=\nExec=x\n"), None);
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

#[cfg(test)]
mod port_tests {
    use super::*;
    use crate::run::FakeEnv;

    fn run(env: &mut FakeEnv, line: &str) -> i32 {
        let argv: Vec<&str> = line.split_whitespace().collect();
        let inv = crate::cli::parse(&argv).expect("parses");
        execute(&inv, env).expect("one of these verbs")
    }

    fn fake() -> FakeEnv {
        FakeEnv {
            home: PathBuf::from("/h"),
            vars: vec![("XDG_DATA_HOME".into(), "/h/.local/share".into())],
            ..FakeEnv::default()
        }
    }

    #[test]
    fn webapp_add_writes_a_launcher_with_the_site_icon() {
        let mut e = fake();
        // touch icon refused, favicon answers
        e.exec_codes = vec![22, 0];
        assert_eq!(
            run(&mut e, "webapp add Mail https://mail.example.com --json"),
            0,
            "{}",
            e.err
        );
        let (file, text) = e.written.last().cloned().unwrap();
        assert_eq!(
            file,
            PathBuf::from("/h/.local/share/applications/wolos-webapp-mail.desktop")
        );
        assert!(
            text.contains("\nIcon=/h/.local/share/wolos/webapps/icons/mail.ico\n"),
            "{text}"
        );
        assert!(text.contains("--app=https://mail.example.com"));
        assert_eq!(e.execs[0][0], "curl");
        assert!(e.execs[0].contains(&"https://mail.example.com/apple-touch-icon.png".into()));
        assert_eq!(
            e.removed,
            vec![PathBuf::from(
                "/h/.local/share/wolos/webapps/icons/mail.png"
            )],
            "the refused download is dropped"
        );
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v["id"], "mail");
    }

    #[test]
    fn webapp_add_refuses_bad_input_and_duplicates() {
        let mut e = fake();
        assert_ne!(run(&mut e, "webapp add Mail file:///etc/passwd"), 0);
        assert!(e.written.is_empty() && e.execs.is_empty());
        let mut e = fake();
        let app = webapps::WebApp::new("Mail", "https://m.example.com", webapps::Profile::Shared)
            .unwrap();
        let data = PathBuf::from("/h/.local/share");
        e.files.push((
            webapps::desktop_path(&data, "mail"),
            webapps::desktop_entry(&app, &data),
        ));
        assert_ne!(run(&mut e, "webapp add mail https://other.example.com"), 0);
        assert!(e.written.is_empty());
        // listed, then removed by name
        assert_eq!(run(&mut e, "webapp list --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v["webapps"][0]["url"], "https://m.example.com");
        e.out.clear();
        assert_eq!(run(&mut e, "webapp remove MAIL --json"), 0);
        assert_eq!(e.removed, vec![webapps::desktop_path(&data, "mail")]);
        assert_ne!(run(&mut e, "webapp remove Maps"), 0);
    }

    /// C6-1: an icon file whose name carries a newline and launcher keys is not used, and the
    /// launcher keeps exactly one `Exec=` line.
    #[test]
    fn webapp_icon_with_a_newline_is_refused() {
        let d = std::env::temp_dir().join(format!("wol-icon-nl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let evil = d.join("x.png\nExec=sh -c evil");
        std::fs::write(&evil, b"png").unwrap();
        let ok = d.join("ok.png");
        std::fs::write(&ok, b"png").unwrap();
        let mut e = fake();
        let mut app =
            webapps::WebApp::new("Chat", "https://chat.example.com", webapps::Profile::Shared)
                .unwrap();
        let okp = ok.display().to_string();
        assert_eq!(webapp_icon(&mut e, &app, Some(&okp)), okp);
        let got = webapp_icon(&mut e, &app, Some(&evil.display().to_string()));
        assert_eq!(got, webapps::FALLBACK_ICON);
        assert_eq!(
            webapp_icon(&mut e, &app, Some("a\\b")),
            webapps::FALLBACK_ICON
        );
        app.icon = got;
        let text = webapps::desktop_entry(&app, std::path::Path::new("/data"));
        assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn webapp_icon_named_or_theme() {
        let mut e = fake();
        assert_eq!(
            run(
                &mut e,
                "webapp add Chat https://chat.example.com mail-send --json"
            ),
            0
        );
        assert!(e.execs.is_empty(), "a theme icon downloads nothing");
        assert!(e.written[0].1.contains("\nIcon=mail-send\n"));
        let mut e = fake();
        assert_eq!(
            run(
                &mut e,
                "webapp add Chat https://chat.example.com ../../x --json"
            ),
            0
        );
        assert!(e.written[0].1.contains("\nIcon=web-browser\n"));
    }

    #[test]
    fn toggles_run_their_command_then_keep_the_flag() {
        let mut e = fake();
        assert_eq!(run(&mut e, "toggle nightlight on --json"), 0, "{}", e.err);
        assert_eq!(
            e.execs[0],
            vec!["hyprctl", "hyprsunset", "temperature", "4500"]
        );
        assert_eq!(
            e.written[0].0,
            PathBuf::from("/h/state/wolos/toggles/org.wolos.nightlight")
        );
        // flips off when the flag is there
        let mut e = fake();
        e.files.push((
            PathBuf::from("/h/state/wolos/toggles/org.wolos.stay-awake"),
            String::new(),
        ));
        assert_eq!(run(&mut e, "toggle idle --json"), 0);
        assert_eq!(e.execs[0][0], "systemctl");
        assert_eq!(
            e.removed,
            vec![PathBuf::from("/h/state/wolos/toggles/org.wolos.stay-awake")]
        );
        // no hyprsunset: Night Light stays as it was, and says why (e2e D2)
        let mut e = fake();
        e.exec_codes = vec![1];
        assert_ne!(run(&mut e, "toggle nightlight on"), 0);
        assert!(e.written.is_empty());
        assert!(e.err.contains("not installed"), "{}", e.err);
        // installed but not running: the compositor starts it, the flag is kept
        let mut e = fake();
        e.exec_codes = vec![1, 0];
        e.files
            .push((PathBuf::from("/usr/bin/hyprsunset"), String::new()));
        assert_eq!(run(&mut e, "toggle nightlight on"), 0, "{}", e.err);
        assert_eq!(
            e.execs[1],
            ["hyprctl", "dispatch", "exec", "hyprsunset -t 4500"]
        );
        assert_eq!(
            e.written[0].0,
            PathBuf::from("/h/state/wolos/toggles/org.wolos.nightlight")
        );
        // and no compositor to ask: not a desktop session
        let mut e = fake();
        e.exec_codes = vec![1, 1];
        e.files
            .push((PathBuf::from("/usr/bin/hyprsunset"), String::new()));
        assert_ne!(run(&mut e, "toggle nightlight on"), 0);
        assert!(e.err.contains("desktop session"), "{}", e.err);
        // off with hyprsunset not running: off, the flag goes
        let mut e = fake();
        e.exec_codes = vec![1];
        e.files.push((
            PathBuf::from("/h/state/wolos/toggles/org.wolos.nightlight"),
            String::new(),
        ));
        assert_eq!(run(&mut e, "toggle nightlight off"), 0, "{}", e.err);
        assert_eq!(
            e.removed,
            vec![PathBuf::from("/h/state/wolos/toggles/org.wolos.nightlight")]
        );
        // QA-SET-46: the tile turned Night Shift on (hyprsunset at 4500 K, no flag file): the
        // hotkey turns it off at the first press.
        let mut e = fake();
        e.captures
            .push(("hyprctl hyprsunset temperature".into(), "4500\n".into()));
        assert_eq!(run(&mut e, "toggle nightlight --json"), 0, "{}", e.err);
        assert_eq!(e.execs[0], vec!["hyprctl", "hyprsunset", "identity"]);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v["on"], false);
        // list
        let mut e = fake();
        assert_eq!(run(&mut e, "toggle --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v["toggles"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn reminders_are_a_user_timer_with_the_text_as_one_argument() {
        let mut e = fake();
        assert_eq!(run(&mut e, "remind 10 tea is; ready --json"), 0);
        let a = &e.execs[0];
        assert_eq!(a[0], "systemd-run");
        assert!(a.contains(&"--on-active=10min".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("tea is; ready"));
        assert!(a.contains(&"--app-name=wol".to_string()), "{a:?}");
        // C10-4: a helper's reminder carries the helper's name, like its direct `notify`.
        let a = remind_argv(5, "tea", &actor_name("mcp:Claude Desktop"));
        assert!(
            a.contains(&"--app-name=Claude Desktop".to_string()),
            "{a:?}"
        );
        let at = a.iter().position(|x| x == "notify-send").unwrap();
        assert_eq!(a[at + 2..], ["--", "Reminder", "tea"]);
        let mut e = fake();
        assert_ne!(run(&mut e, "remind 0 x"), 0);
        assert_ne!(run(&mut e, "remind 99999 x"), 0);
        assert_ne!(run(&mut e, "remind 5"), 0);
        assert!(e.execs.is_empty());
    }

    /// The layout Settings shows comes from desktop.json; keyboard.lua (deleted after the first
    /// Settings save) answers only before that.
    #[test]
    fn settings_reads_the_keyboard_through_the_desktop_prefs() {
        let keyboard = |e: &mut FakeEnv| {
            e.out.clear();
            assert_eq!(run(e, "settings --json"), 0, "{}", e.err);
            let v: Value = serde_json::from_str(&e.out).expect("json");
            (
                v["keyboard"]["layout"].as_str().map(str::to_owned),
                v["keyboard"]["repeat"].as_str().map(str::to_owned),
            )
        };
        let pair = |a: &str, b: &str| (Some(a.to_owned()), Some(b.to_owned()));
        let mut e = fake();
        assert_eq!(keyboard(&mut e), pair("us", "Normal"));
        // First run: the setup assistant's file.
        e.files.push((
            PathBuf::from("/h/hypr/wolos/keyboard.lua"),
            wol_core::prefs::keyboard_lua("br", 2).unwrap(),
        ));
        assert_eq!(keyboard(&mut e), pair("br", "Fast"));
        // After a Settings save only desktop.json is left, and it wins over a stale keyboard.lua.
        let d = wol_core::hyprprefs::DesktopPrefs {
            layouts: vec!["us(intl)".into(), "br".into()],
            repeat_rate: 15,
            repeat_delay: 600,
            ..Default::default()
        };
        e.files
            .push((PathBuf::from("/h/wolos/desktop.json"), d.to_json()));
        assert_eq!(keyboard(&mut e), pair("us(intl)", "Slow"));
        e.files.remove(0);
        assert_eq!(keyboard(&mut e), pair("us(intl)", "Slow"));
    }

    /// QA-PWR-21: `wol wifi on` that does not work names the cause (no device or no
    /// NetworkManager), never only "the change did not work".
    #[test]
    fn wifi_on_failure_names_cause() {
        let mut e = fake();
        let code = run(&mut e, "wifi on");
        assert_ne!(code, 0);
        assert!(
            e.err.contains("no Wi-Fi device") && e.err.contains("NetworkManager"),
            "{}",
            e.err
        );
        assert!(!e.err.contains("the change did not work"), "{}", e.err);
        // A device, NetworkManager refuses: its log is named.
        let h = std::sync::Arc::new(wol_core::system::fake::FakeHost::linux());
        h.command("nmcli -t -f TYPE dev", "wifi\n")
            .command("nmcli radio wifi", "enabled\n");
        let mut e = FakeEnv {
            host: Some(h),
            ..fake()
        };
        assert_ne!(run(&mut e, "wifi on"), 0);
        assert!(
            e.err.contains("NetworkManager did not make the change"),
            "{}",
            e.err
        );
    }
}
