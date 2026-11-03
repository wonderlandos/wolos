// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol audio doctor|reset|appmix` (docs/modules/audio.md, docs/creators-audio.md): the sound
//! stack checked with the exact fix, a reset that forgets per-user state (dry run unless `--yes`),
//! and the OBS App Mix. Rules and parsers in `wol_core::audio`; all I/O goes through [`Env`].
//! `wol audio`, `volume`, `mute` keep the `wpctl` path in `verbs.rs`.
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use shared::errors::ErrorCode;
use std::path::Path;
use wol_core::audio as a;

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "audio.doctor" => doctor(env, json),
        "audio.reset" => reset(env, inv.flag("yes"), json),
        "audio.appmix" => appmix(env, inv.get("state").unwrap_or("status"), json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: String,
}

fn check(id: &'static str, status: Status, detail: impl Into<String>, fix: &str) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
        fix: fix.into(),
    }
}

/// The shipped fragments and the one line each that carries its rule.
const FRAGMENTS: &[(&str, &str)] = &[
    (
        "/usr/share/pipewire/pipewire.conf.d/70-wolos-latency.conf",
        "default.clock.min-quantum = 64",
    ),
    (
        "/usr/share/pipewire/pipewire-pulse.conf.d/70-wolos-latency.conf",
        "pulse.min.quantum = 256/48000",
    ),
    (
        "/usr/share/wireplumber/wireplumber.conf.d/71-wolos-input.conf",
        "device.routes.default-source-volume = 0.064",
    ),
    (
        "/usr/share/wireplumber/wireplumber.conf.d/72-wolos-alsa.conf",
        "api.alsa.soft-mixer = false",
    ),
];

/// The file under `WOL_ROOT` (tests, an installed disk) or `/`.
fn at(env: &dyn Env, p: &str) -> String {
    format!("{}{p}", env.var("WOL_ROOT").unwrap_or_default())
}

fn exists(env: &dyn Env, p: &str) -> bool {
    let full = at(env, p);
    let path = Path::new(&full);
    env.read_file(path).is_some()
        || path
            .parent()
            .is_some_and(|d| env.list_dir(d).iter().any(|e| Path::new(e) == path))
}

fn active(env: &mut dyn Env, user: bool, unit: &str) -> bool {
    let out = if user {
        env.capture(&["systemctl", "--user", "is-active", unit])
    } else {
        env.capture(&["systemctl", "is-active", unit])
    };
    out.is_some_and(|o| o.trim() == "active")
}

pub fn checks(env: &mut dyn Env) -> Vec<Check> {
    let mut v = Vec::new();
    // Services.
    let pw = env.capture(&["pw-cli", "info", "0"]);
    v.push(match &pw {
        Some(o) => {
            let ver = o
                .lines()
                .find_map(|l| l.trim().strip_prefix("version: "))
                .unwrap_or("?")
                .trim_matches('"')
                .to_owned();
            check(
                "pipewire",
                Status::Ok,
                format!("running, version {ver}"),
                "",
            )
        }
        None => check(
            "pipewire",
            Status::Fail,
            "PipeWire is not running",
            "systemctl --user start pipewire.service pipewire-pulse.service",
        ),
    });
    v.push(if active(env, true, "wireplumber.service") {
        check("wireplumber", Status::Ok, "session manager running", "")
    } else {
        check(
            "wireplumber",
            Status::Fail,
            "WirePlumber is not running: no default devices, no volumes",
            "systemctl --user start wireplumber.service",
        )
    });
    v.push(if active(env, false, "rtkit-daemon.service") {
        check("realtime", Status::Ok, "rtkit grants realtime threads", "")
    } else {
        let limits = env
            .read_file(Path::new(&at(
                env,
                "/etc/security/limits.d/70-wolos-audio.conf",
            )))
            .is_some_and(|t| t.contains("@audio - rtprio 95"));
        if limits {
            check(
                "realtime",
                Status::Warn,
                "rtkit not running; the audio group's rtprio 95 limit still applies",
                "systemctl start rtkit-daemon.service",
            )
        } else {
            check(
                "realtime",
                Status::Fail,
                "no realtime scheduling for audio threads",
                "pacman -S rtkit; systemctl enable --now rtkit-daemon.service",
            )
        }
    });
    // Shipped rules.
    let mut missing: Vec<String> = Vec::new();
    for (f, line) in FRAGMENTS {
        let present = env
            .read_file(Path::new(&at(env, f)))
            .is_some_and(|t| t.lines().any(|l| l.trim() == *line));
        if !present {
            missing.push(f.rsplit('/').next().unwrap_or(f).to_owned());
        }
    }
    v.push(if missing.is_empty() {
        check(
            "config",
            Status::Ok,
            "WolOS fragments present: 64-frame floor for games, 256 for pulse clients, inputs start at 40%, hardware mixer kept",
            "",
        )
    } else {
        check(
            "config",
            Status::Fail,
            format!("missing or changed: {}", missing.join(", ")),
            "pacman -S wolos-system (reinstalls /usr/share/pipewire and /usr/share/wireplumber fragments)",
        )
    });
    // Input level: the historical problem.
    let dump = env.capture(&["pw-dump"]).unwrap_or_default();
    let devices = a::parse_pw_dump(&dump);
    let mic = devices
        .iter()
        .find(|d| d.kind == a::DeviceKind::Input && d.default)
        .or_else(|| devices.iter().find(|d| d.kind == a::DeviceKind::Input));
    v.push(match mic {
        None => check(
            "input-level",
            Status::Warn,
            "no input device",
            "plug in or pair a microphone",
        ),
        Some(d) => {
            let db = a::db_of(d.volume, d.volume_base);
            if d.volume > d.volume_base + 1e-4 {
                check(
                    "input-level",
                    Status::Warn,
                    format!(
                        "\"{}\" is {:+.1} dB above 0 dB ({}% of hardware max): it may clip",
                        d.name,
                        db,
                        a::percent_of(d.volume, 1.0, false)
                    ),
                    &format!(
                        "wpctl set-volume {} {:.4} (0 dB) or Settings > Sound > Input",
                        d.id, d.volume_base
                    ),
                )
            } else {
                check(
                    "input-level",
                    Status::Ok,
                    format!(
                        "\"{}\" at {}% ({:+.1} dB){}",
                        d.name,
                        a::percent_of(d.volume, d.volume_base, false),
                        db,
                        if d.muted { ", muted" } else { "" }
                    ),
                    "",
                )
            }
        }
    });
    // Clock and xruns.
    let md = env
        .capture(&["pw-metadata", "-n", "settings", "0"])
        .unwrap_or_default();
    let (rate, quantum) = a::parse_settings_metadata(&md);
    v.push(check(
        "clock",
        Status::Ok,
        match (rate, quantum) {
            (0, 0) => "automatic (48000 Hz default, 64..2048 frames)".to_string(),
            (r, 0) => format!("rate forced to {r} Hz (Pro Audio)"),
            (0, q) => format!("buffer forced to {q} frames (Pro Audio)"),
            (r, q) => format!(
                "forced to {r} Hz, {q} frames = {:.1} ms (Pro Audio)",
                a::quantum_ms(q, r)
            ),
        },
        "",
    ));
    let top = env
        .capture(&["pw-top", "-b", "-n", "1"])
        .unwrap_or_default();
    let xruns = a::parse_pw_top_xruns(&top);
    v.push(if xruns == 0 {
        check("xruns", Status::Ok, "no dropouts reported", "")
    } else {
        check(
            "xruns",
            Status::Warn,
            format!("{xruns} dropout(s) since the devices started"),
            "Settings > Sound > Pro Audio: a larger buffer; USB: another port or a powered hub",
        )
    });
    // Creators: the on-demand pieces.
    v.push(if exists(env, "/usr/lib/ladspa/librnnoise_ladspa.so") {
        check(
            "rnnoise",
            Status::Ok,
            "noise-suppression-for-voice installed",
            "",
        )
    } else {
        check(
            "rnnoise",
            Status::Warn,
            "RNNoise plugin missing: \"Reduce background noise\" has nothing to load",
            "pacman -S noise-suppression-for-voice",
        )
    });
    let obs = exists(env, "/usr/share/applications/com.obsproject.Studio.desktop");
    // OBS >= 33 keeps plugins in obs-modules/plugins; older builds in obs-plugins.
    let plugin = exists(env, "/usr/lib/obs-modules/plugins/linux-pipewire-audio.so")
        || exists(env, "/usr/lib/obs-plugins/linux-pipewire-audio.so");
    v.push(match (obs, plugin) {
        (true, true) => check("obs", Status::Ok, "OBS with PipeWire audio capture", ""),
        (true, false) => check(
            "obs",
            Status::Warn,
            "OBS installed without the PipeWire audio capture plugin (app and desktop audio sources)",
            "pacman -S obs-pipewire-audio-capture",
        ),
        (false, _) => check("obs", Status::Ok, "OBS not installed", ""),
    });
    v.push(
        if exists(env, "/usr/share/xdg-desktop-portal/portals/hyprland.portal") {
            check(
                "portal",
                Status::Ok,
                "xdg-desktop-portal-hyprland for screen capture",
                "",
            )
        } else {
            check(
                "portal",
                Status::Warn,
                "no compositor portal: OBS screen capture has nothing to ask",
                "pacman -S xdg-desktop-portal-hyprland",
            )
        },
    );
    v
}

pub fn ready(all: &[Check]) -> bool {
    all.iter().all(|c| c.status != Status::Fail)
}

/// One line per check, its fix under it, then the verdict.
pub fn report(all: &[Check]) -> String {
    let mut t = String::new();
    for c in all {
        t.push_str(&format!(
            "{:<5} {:<12} {}\n",
            c.status.name(),
            c.id,
            c.detail
        ));
        if !c.fix.is_empty() && c.status != Status::Ok {
            t.push_str(&format!("{:<18} fix: {}\n", "", c.fix));
        }
    }
    let failed = all.iter().filter(|c| c.status == Status::Fail).count();
    let warned = all.iter().filter(|c| c.status == Status::Warn).count();
    t.push_str(&match (failed, warned) {
        (0, 0) => "Sound is ready.\n".to_string(),
        (0, w) => format!("Sound works; {w} thing(s) worth a look above.\n"),
        (f, _) => {
            format!("Not ready: {f} problem(s). Run the fix lines above, then check again.\n")
        }
    });
    t
}

fn doctor(env: &mut dyn Env, json: bool) -> i32 {
    let all = checks(env);
    let ok = ready(&all);
    if json {
        emit_json(env, &serde_json::json!({ "ready": ok, "checks": all }));
    } else {
        env.out(&report(&all));
    }
    if ok { 0 } else { ErrorCode::Failed.exit() }
}

/// Unload every pipewire-pulse module whose arguments name one of `markers`.
fn unload(env: &mut dyn Env, markers: &[&str]) -> Vec<u32> {
    let list = env
        .capture(&["pactl", "list", "short", "modules"])
        .unwrap_or_default();
    let ids = a::modules_matching(&list, markers);
    for id in &ids {
        let _ = env.exec(&[
            "pactl".to_string(),
            "unload-module".to_string(),
            id.to_string(),
        ]);
    }
    ids
}

fn reset(env: &mut dyn Env, yes: bool, json: bool) -> i32 {
    let home = env.home();
    let state = Some(env.state_home())
        .filter(|s| s.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"));
    let Some(plan) = a::reset_plan(&home, &state) else {
        return fail(
            env,
            ErrorCode::Usage,
            "HOME is not set to an absolute path, so there is nothing safe to reset",
        );
    };
    if !yes {
        if json {
            emit_json(env, &serde_json::json!({ "dry_run": true, "plan": plan }));
        } else {
            env.out(&format!(
                "Would:\n{}Run `wol audio reset --yes` to do it.\n",
                plan.describe()
            ));
        }
        return 0;
    }
    unload(
        env,
        &[
            a::AGC_SOURCE,
            a::RNNOISE_SOURCE,
            a::APPMIX_SINK,
            a::MICMIX_SOURCE,
        ],
    );
    for p in &plan.remove {
        let _ = env.exec(&[
            "rm".into(),
            "-rf".into(),
            "--".into(),
            p.to_string_lossy().into_owned(),
        ]);
    }
    let mut failed = 0;
    for c in &plan.commands {
        if env.exec(c).ok() != Some(0) {
            failed += 1;
        }
    }
    if json {
        emit_json(
            env,
            &serde_json::json!({ "dry_run": false, "plan": plan, "failed": failed }),
        );
    } else {
        env.out(&format!(
            "{}Sound reset: PipeWire restarted.\n",
            plan.describe()
        ));
    }
    if failed == 0 {
        0
    } else {
        ErrorCode::Failed.exit()
    }
}

const APPMIX_MARKERS: [&str; 2] = [a::APPMIX_SINK, a::MICMIX_SOURCE];

fn appmix(env: &mut dyn Env, state: &str, json: bool) -> i32 {
    match state {
        "on" => {
            let mic = env
                .capture(&["pactl", "get-default-source"])
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            let Some(mic) = mic else {
                return fail(env, ErrorCode::Unavailable, "no input device to mix");
            };
            unload(env, &APPMIX_MARKERS);
            for argv in a::appmix_argv(&mic) {
                if env.exec(&argv).ok() != Some(0) {
                    return fail(
                        env,
                        ErrorCode::Failed,
                        "pipewire-pulse refused a module (is pipewire-pulse running?)",
                    );
                }
            }
        }
        "off" => {
            unload(env, &APPMIX_MARKERS);
        }
        _ => {}
    }
    let list = env
        .capture(&["pactl", "list", "short", "modules"])
        .unwrap_or_default();
    let modules = a::modules_matching(&list, &APPMIX_MARKERS);
    let on = !modules.is_empty();
    if json {
        emit_json(env, &serde_json::json!({ "on": on, "modules": modules }));
    } else if on {
        env.out(
            "App Mix: on. Send apps to the output \"WolOS App Mix\"; in OBS add an Audio Input Capture \
             on \"Mic + App Mix\" (or capture \"WolOS App Mix\" alone with the PipeWire Audio Capture plugin).\n",
        );
    } else {
        env.out("App Mix: off. `wol audio appmix on` creates it.\n");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;
    use std::path::PathBuf;

    const DUMP: &str = r#"[
      { "id": 31, "type": "PipeWire:Interface:Metadata", "metadata": [
          { "subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": { "name": "alsa_input.mic" } } ] },
      { "id": 46, "type": "PipeWire:Interface:Node", "info": {
          "props": { "node.name": "alsa_input.mic", "node.description": "Built-in Mic", "media.class": "Audio/Source" },
          "params": { "Props": [ { "volume": 1.0, "mute": false, "channelVolumes": [ 1.0, 1.0 ], "volumeBase": 0.0316 } ] } } }
    ]"#;

    fn env(mic_at_max: bool) -> FakeEnv {
        let root = PathBuf::from("/r");
        let mut e = FakeEnv {
            tty: true,
            home: PathBuf::from("/home/t"),
            vars: vec![("WOL_ROOT".into(), "/r".into())],
            ..Default::default()
        };
        for (f, line) in FRAGMENTS {
            e.files
                .push((root.join(f.trim_start_matches('/')), format!("{line}\n")));
        }
        e.files.push((
            root.join("etc/security/limits.d/70-wolos-audio.conf"),
            "@audio - rtprio 95\n".into(),
        ));
        e.files.push((
            root.join("usr/lib/ladspa/librnnoise_ladspa.so"),
            String::new(),
        ));
        e.files.push((
            root.join("usr/share/xdg-desktop-portal/portals/hyprland.portal"),
            "[portal]\n".into(),
        ));
        e.captures = vec![
            ("pw-cli info 0".into(), "\tversion: \"1.6.10\"\n".into()),
            ("systemctl --user is-active wireplumber.service".into(), "active\n".into()),
            ("systemctl is-active rtkit-daemon.service".into(), "active\n".into()),
            ("pw-metadata -n settings 0".into(), "update: id:0 key:'clock.force-quantum' value:'256' type:''\n".into()),
            ("pw-top -b -n 1".into(), "S ID QUANT RATE WAIT BUSY W/Q B/Q ERR FORMAT NAME\nR 45 256 48000 1us 2us 0.0 0.0 0 S32LE x\n".into()),
            (
                "pw-dump".into(),
                if mic_at_max {
                    DUMP.into()
                } else {
                    DUMP.replace("[ 1.0, 1.0 ]", "[ 0.0316, 0.0316 ]")
                },
            ),
        ];
        e
    }

    fn by<'a>(all: &'a [Check], id: &str) -> &'a Check {
        all.iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("{id}"))
    }

    #[test]
    fn a_microphone_above_zero_db_is_called_out_with_the_wpctl_fix() {
        let mut e = env(true);
        let all = checks(&mut e);
        let m = by(&all, "input-level");
        assert_eq!(m.status, Status::Warn);
        assert!(
            m.detail.contains("Built-in Mic") && m.detail.contains("+30.0 dB"),
            "{}",
            m.detail
        );
        assert!(m.fix.starts_with("wpctl set-volume 46 0.0316"), "{}", m.fix);
        assert!(
            ready(&all),
            "a loud mic is the user's state, not a broken stack"
        );
        let mut e = env(false);
        let all = checks(&mut e);
        let m = by(&all, "input-level");
        assert_eq!(m.status, Status::Ok);
        assert!(m.detail.contains("at 100% (+0.0 dB)"), "{}", m.detail);
    }

    #[test]
    fn the_healthy_machine_passes_every_check() {
        let mut e = env(false);
        let all = checks(&mut e);
        for id in [
            "pipewire",
            "wireplumber",
            "realtime",
            "config",
            "clock",
            "xruns",
            "rnnoise",
            "obs",
            "portal",
        ] {
            assert_eq!(
                by(&all, id).status,
                Status::Ok,
                "{id}: {}",
                by(&all, id).detail
            );
        }
        assert!(by(&all, "pipewire").detail.contains("1.6.10"));
        assert!(by(&all, "clock").detail.contains("256 frames"));
        assert_eq!(by(&all, "obs").detail, "OBS not installed");
        let argv = ["audio", "doctor"];
        let inv = crate::cli::parse(&argv).unwrap();
        assert_eq!(execute(&inv, &mut e), 0);
        assert!(e.out.contains("Sound is ready."), "{}", e.out);
        assert!(
            e.out.lines().any(|l| l.starts_with("ok    input-level")),
            "{}",
            e.out
        );
    }

    #[test]
    fn missing_services_and_fragments_fail_with_the_exact_fix() {
        let mut e = env(false);
        e.captures
            .retain(|c| !c.0.starts_with("pw-cli") && !c.0.contains("wireplumber"));
        e.files.retain(|f| !f.0.ends_with("72-wolos-alsa.conf"));
        let all = checks(&mut e);
        assert_eq!(by(&all, "pipewire").status, Status::Fail);
        assert!(
            by(&all, "wireplumber")
                .fix
                .contains("systemctl --user start wireplumber")
        );
        let c = by(&all, "config");
        assert_eq!(c.status, Status::Fail);
        assert!(
            c.detail.contains("72-wolos-alsa.conf") && !c.detail.contains("71-wolos"),
            "{}",
            c.detail
        );
        assert!(!ready(&all));
        let t = report(&all);
        assert!(
            t.contains("FAIL  pipewire") && t.contains("Not ready: 3 problem(s)"),
            "{t}"
        );
        e.tty = false;
        let argv = ["audio", "doctor"];
        assert_eq!(
            execute(&crate::cli::parse(&argv).unwrap(), &mut e),
            ErrorCode::Failed.exit()
        );
        let v: serde_json::Value = serde_json::from_str(e.out.trim()).unwrap();
        assert_eq!(v["ready"], false);
        assert_eq!(v["checks"][0]["id"], "pipewire");
    }

    #[test]
    fn obs_without_the_plugin_and_no_rtkit_are_warnings() {
        let mut e = env(false);
        e.files.push((
            PathBuf::from("/r/usr/share/applications/com.obsproject.Studio.desktop"),
            "[Desktop Entry]\n".into(),
        ));
        e.captures.retain(|c| !c.0.contains("rtkit"));
        let all = checks(&mut e);
        assert_eq!(by(&all, "obs").status, Status::Warn);
        assert_eq!(by(&all, "obs").fix, "pacman -S obs-pipewire-audio-capture");
        assert_eq!(by(&all, "realtime").status, Status::Warn);
        assert!(by(&all, "xruns").status == Status::Ok);
        assert!(report(&all).contains("worth a look"));
        e.files.push((
            PathBuf::from("/r/usr/lib/obs-modules/plugins/linux-pipewire-audio.so"),
            String::new(),
        ));
        assert_eq!(by(&checks(&mut e), "obs").status, Status::Ok);
    }

    #[test]
    fn reset_is_a_dry_run_unless_yes() {
        let mut e = env(false);
        let argv = ["audio", "reset"];
        assert_eq!(execute(&crate::cli::parse(&argv).unwrap(), &mut e), 0);
        assert!(
            e.out.contains("remove /home/t/.local/state/wireplumber") && e.out.contains("--yes"),
            "{}",
            e.out
        );
        assert!(e.execs.is_empty(), "nothing ran: {:?}", e.execs);
        e.captures.push((
            "pactl list short modules".into(),
            "7\tmodule-null-sink\tsink_name=wolos_appmix\n".into(),
        ));
        let argv = ["audio", "reset", "--yes"];
        assert_eq!(execute(&crate::cli::parse(&argv).unwrap(), &mut e), 0);
        let lines: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
        assert_eq!(lines[0], "pactl unload-module 7");
        assert!(
            lines.contains(&"rm -rf -- /home/t/.local/state/wireplumber".to_string()),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|l| !l.starts_with("rm") || l.contains(" -- /"))
        );
        assert!(lines.contains(&"wpctl clear-default".to_string()));
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("systemctl --user restart pipewire.service"),
            "{lines:?}"
        );
        assert!(e.out.contains("PipeWire restarted"));
    }

    /// Audit S4-22: without an absolute HOME nothing is removed.
    #[test]
    fn reset_refuses_a_relative_home() {
        let mut e = env(false);
        e.home = PathBuf::new();
        let argv = ["audio", "reset", "--yes"];
        assert_eq!(
            execute(&crate::cli::parse(&argv).unwrap(), &mut e),
            ErrorCode::Usage.exit()
        );
        assert!(e.execs.is_empty(), "{:?}", e.execs);
    }

    #[test]
    fn appmix_on_off_status() {
        let mut e = env(false);
        e.captures
            .push(("pactl get-default-source".into(), "alsa_input.mic\n".into()));
        let argv = ["audio", "appmix", "on"];
        assert_eq!(execute(&crate::cli::parse(&argv).unwrap(), &mut e), 0);
        let lines: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(
            lines[0].contains("module-null-sink") && lines[0].contains("sink_name=wolos_appmix")
        );
        assert!(lines[4].contains("source=alsa_input.mic"));
        assert!(
            e.out.contains("App Mix: off"),
            "status read back from pactl, which the fake lists empty: {}",
            e.out
        );
        e.out.clear();
        e.captures.push((
            "pactl list short modules".into(),
            "1\tmodule-null-sink\tsink_name=wolos_appmix\n2\tmodule-null-sink\tsink_name=wolos_mic_mix\n".into(),
        ));
        let argv = ["audio", "appmix", "status", "--json"];
        assert_eq!(execute(&crate::cli::parse(&argv).unwrap(), &mut e), 0);
        let v: serde_json::Value = serde_json::from_str(e.out.trim()).unwrap();
        assert_eq!(v, serde_json::json!({ "on": true, "modules": [1, 2] }));
        e.execs.clear();
        let argv = ["audio", "appmix", "off"];
        execute(&crate::cli::parse(&argv).unwrap(), &mut e);
        let lines: Vec<String> = e.execs.iter().map(|a| a.join(" ")).collect();
        assert_eq!(lines, ["pactl unload-module 1", "pactl unload-module 2"]);
        // No input device: refused in plain words.
        let mut e = env(false);
        let argv = ["audio", "appmix", "on"];
        assert_eq!(
            execute(&crate::cli::parse(&argv).unwrap(), &mut e),
            ErrorCode::Unavailable.exit()
        );
    }
}
