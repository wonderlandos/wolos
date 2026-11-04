// SPDX-License-Identifier: BSD-2-Clause
//! Desktop toggles (ADR 0031 W5): one-word switches a person flips from `wol toggle`, a hotkey,
//! a Control Center tile or a plugin: Night Light, Stay Awake and Silence (Do Not Disturb).
//! This file is the table and the plan: which command turns each one on or off, and which file
//! says it is on, so the CLI (crates/controller/src/verbs.rs) and the shell read the same state.
//! Invariant: a toggle's state lives in exactly one file ([`state_file`]); the commands that
//! apply it are fixed argv with no outside text in them.
use std::path::{Path, PathBuf};

/// Do Not Disturb flag file (present = on), relative to the config home; the notification
/// banner code and `wol focus` read the same file.
pub const DND_FILE: &str = "wol/dnd";
/// Plugin id our own toggles use in `$XDG_STATE_HOME/wolos/toggles/<id>.<toggle>`
/// (`shared::plugins::TOGGLES_DIR`), so they sit next to plugin toggles and cannot collide.
pub const OWNER: &str = "org.wolos";
/// User unit that holds the idle inhibitor while Stay Awake is on.
pub const STAY_AWAKE_UNIT: &str = "wol-stay-awake.service";
/// Night Light colour temperature in kelvin (`wol_core::controls::NIGHT_SHIFT_KELVIN`).
pub const NIGHT_LIGHT_KELVIN: u32 = crate::controls::NIGHT_SHIFT_KELVIN;

/// One desktop toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Toggle {
    /// Warmer colours (hyprsunset).
    NightLight,
    /// The screen does not dim, lock or sleep by itself (an idle inhibitor hypridle honours).
    StayAwake,
    /// Do Not Disturb: banners wait in the Notification Center.
    Silence,
}

impl Toggle {
    /// Every toggle, in the order `wol toggle` lists them.
    pub const ALL: [Toggle; 3] = [Toggle::NightLight, Toggle::StayAwake, Toggle::Silence];

    /// The id `wol toggle <id>` takes.
    pub fn id(self) -> &'static str {
        match self {
            Toggle::NightLight => "nightlight",
            Toggle::StayAwake => "stay-awake",
            Toggle::Silence => "silence",
        }
    }

    /// Other words accepted for the same toggle.
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Toggle::NightLight => &["night-light", "nightshift", "night-shift"],
            Toggle::StayAwake => &["idle", "awake", "caffeine"],
            Toggle::Silence => &["dnd", "focus"],
        }
    }

    /// Label for tiles and menus.
    pub fn label(self) -> &'static str {
        match self {
            Toggle::NightLight => "Night Light",
            Toggle::StayAwake => "Stay Awake",
            Toggle::Silence => "Do Not Disturb",
        }
    }

    /// Symbolic icon for tiles.
    pub fn icon(self) -> &'static str {
        match self {
            Toggle::NightLight => "night-light-symbolic",
            Toggle::StayAwake => "preferences-desktop-screensaver-symbolic",
            Toggle::Silence => "notifications-disabled-symbolic",
        }
    }

    /// Looks a toggle up by id or alias, ignoring case. None for an unknown word.
    pub fn from_word(w: &str) -> Option<Toggle> {
        let w = w.trim().to_ascii_lowercase();
        Toggle::ALL
            .into_iter()
            .find(|t| t.id() == w || t.aliases().contains(&w.as_str()))
    }
}

/// Every id and alias, for the CLI's argument check and help.
pub fn words() -> Vec<&'static str> {
    Toggle::ALL
        .iter()
        .flat_map(|t| std::iter::once(t.id()).chain(t.aliases().iter().copied()))
        .collect()
}

/// The one file whose presence means the toggle is on: Silence is the Do Not Disturb file in
/// the config home; the others are flag files under the state home.
pub fn state_file(t: Toggle, config_home: &Path, state_home: &Path) -> PathBuf {
    match t {
        Toggle::Silence => config_home.join(DND_FILE),
        _ => state_home
            .join(shared::plugins::TOGGLES_DIR)
            .join(format!("{OWNER}.{}", t.id())),
    }
}

/// Is `t` on? `exists` is the file check (the shell and the CLI pass their own).
pub fn is_on(
    t: Toggle,
    config_home: &Path,
    state_home: &Path,
    exists: impl Fn(&Path) -> bool,
) -> bool {
    exists(&state_file(t, config_home, state_home))
}

/// Night Light counts as on below this colour temperature (hyprsunset's own 6500 K identity is off).
pub const NIGHT_LIGHT_BELOW_K: u32 = 6000;

/// Night Light as hyprsunset reports it (`hyprctl hyprsunset temperature`): the one truth, since
/// the Control Center tile, the Displays switch and `wol toggle nightlight` all drive it. None
/// when hyprsunset is not running (QA-SET-46: the tile read this, the hotkey read a flag file).
pub fn night_light_from(temperature: &str) -> Option<bool> {
    let k: u32 = temperature.trim().parse().ok()?;
    Some(k < NIGHT_LIGHT_BELOW_K)
}

/// Writes `t`'s flag file (`on`) or removes it, so the file-reading views (`wol toggle` listing,
/// plugin tiles) agree with a change made by the tile.
pub fn set_flag(t: Toggle, config_home: &Path, state_home: &Path, on: bool) -> std::io::Result<()> {
    let f = state_file(t, config_home, state_home);
    if on {
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&f, "")
    } else {
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Commands that apply `t` = `on`, in order, before the state file changes. Silence needs none
/// (the notification code reads the file).
pub fn commands(t: Toggle, on: bool) -> Vec<Vec<String>> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match (t, on) {
        (Toggle::NightLight, true) => vec![v(&[
            "hyprctl",
            "hyprsunset",
            "temperature",
            &NIGHT_LIGHT_KELVIN.to_string(),
        ])],
        (Toggle::NightLight, false) => vec![v(&["hyprctl", "hyprsunset", "identity"])],
        // systemd-run keeps the inhibitor alive after `wol` exits; --collect drops the unit
        // when it stops. hypridle honours logind idle inhibitors (ignore_systemd_inhibit=false).
        (Toggle::StayAwake, true) => vec![v(&[
            "systemd-run",
            "--user",
            "--collect",
            "--quiet",
            &format!("--unit={STAY_AWAKE_UNIT}"),
            "--",
            "systemd-inhibit",
            "--what=idle",
            "--who=WolOS",
            "--why=Stay Awake is on",
            "--mode=block",
            "sleep",
            "infinity",
        ])],
        (Toggle::StayAwake, false) => {
            vec![v(&["systemctl", "--user", "stop", "--", STAY_AWAKE_UNIT])]
        }
        (Toggle::Silence, _) => Vec::new(),
    }
}

/// The program behind Night Light: hyprland.conf starts it at login (`exec-once`), the commands
/// reach it through `hyprctl hyprsunset`.
pub const NIGHT_LIGHT_PROGRAM: &str = "hyprsunset";

/// When `t`'s commands failed because its program is not running (it ended, or was installed
/// after login): the command that starts it already applied (the compositor runs it in the
/// session). Only Night Light on has one.
pub fn start_command(t: Toggle, on: bool) -> Option<Vec<String>> {
    (t == Toggle::NightLight && on).then(|| {
        vec![
            "hyprctl".to_owned(),
            "dispatch".to_owned(),
            "exec".to_owned(),
            format!("{NIGHT_LIGHT_PROGRAM} -t {NIGHT_LIGHT_KELVIN}"),
        ]
    })
}

/// Why `t` could not change, one sentence a person can act on. `installed`: its program is on
/// this system (only Night Light on can fail this way, [`must_succeed`]); then the compositor
/// did not answer either ([`start_command`]): this is not a desktop session.
pub fn unavailable(t: Toggle, installed: bool) -> String {
    if installed {
        format!("{} changes only from a desktop session.", t.label())
    } else {
        format!(
            "{} is not available on this computer: the {NIGHT_LIGHT_PROGRAM} package that tints \
             the screen is not installed.",
            t.label()
        )
    }
}

/// Whether a failed command should stop the toggle. Turning Stay Awake on when it already is
/// fails (the unit exists) and turning it off when it is not fails too: the state is right
/// either way, so only Night Light on (no hyprsunset) stops the change. Night Light off with
/// hyprsunset not running is right as well: the screen has its true colours.
pub fn must_succeed(t: Toggle, on: bool) -> bool {
    t == Toggle::NightLight && on
}

/// What `wol toggle <id> [on|off]` does: `None` flips it.
pub fn target(current: bool, wanted: Option<bool>) -> bool {
    wanted.unwrap_or(!current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_find_their_toggle() {
        assert_eq!(Toggle::from_word("dnd"), Some(Toggle::Silence));
        assert_eq!(Toggle::from_word("Idle"), Some(Toggle::StayAwake));
        assert_eq!(Toggle::from_word("nightlight"), Some(Toggle::NightLight));
        assert_eq!(Toggle::from_word("bogus"), None);
        let w = words();
        let mut d = w.clone();
        d.sort_unstable();
        d.dedup();
        assert_eq!(d.len(), w.len(), "no word names two toggles");
        for t in Toggle::ALL {
            assert!(shared::plugins::is_entry_id(t.id()), "{t:?}");
        }
    }

    #[test]
    fn each_toggle_has_one_state_file() {
        let (c, s) = (Path::new("/c"), Path::new("/s"));
        assert_eq!(
            state_file(Toggle::Silence, c, s),
            PathBuf::from("/c/wol/dnd")
        );
        assert_eq!(
            state_file(Toggle::NightLight, c, s),
            PathBuf::from("/s/wolos/toggles/org.wolos.nightlight")
        );
        assert!(shared::plugins::is_plugin_id(OWNER));
        let on = PathBuf::from("/s/wolos/toggles/org.wolos.stay-awake");
        assert!(is_on(Toggle::StayAwake, c, s, |p| p == on));
        assert!(!is_on(Toggle::NightLight, c, s, |p| p == on));
    }

    #[test]
    fn commands_are_fixed() {
        assert_eq!(
            commands(Toggle::NightLight, true),
            vec![vec!["hyprctl", "hyprsunset", "temperature", "4500"]]
        );
        assert_eq!(
            commands(Toggle::NightLight, false),
            vec![vec!["hyprctl", "hyprsunset", "identity"]]
        );
        let on = &commands(Toggle::StayAwake, true)[0];
        assert_eq!(on[0], "systemd-run");
        assert!(on.contains(&"--what=idle".to_string()));
        assert!(on.contains(&format!("--unit={STAY_AWAKE_UNIT}")));
        assert_eq!(
            commands(Toggle::StayAwake, false),
            vec![vec!["systemctl", "--user", "stop", "--", STAY_AWAKE_UNIT]]
        );
        assert!(commands(Toggle::Silence, true).is_empty());
        assert!(must_succeed(Toggle::NightLight, true) && !must_succeed(Toggle::StayAwake, true));
        assert!(
            !must_succeed(Toggle::NightLight, false),
            "off without hyprsunset is off"
        );
        // e2e D2: never "log in" advice to someone logged in when the program is missing.
        let missing = unavailable(Toggle::NightLight, false);
        assert!(
            missing.contains("not installed") && !missing.contains("log in"),
            "{missing}"
        );
        assert!(unavailable(Toggle::NightLight, true).contains("desktop session"));
        // Not running (ended, or installed after login): the compositor starts it warm.
        assert_eq!(
            start_command(Toggle::NightLight, true),
            Some(vec![
                "hyprctl".into(),
                "dispatch".into(),
                "exec".into(),
                "hyprsunset -t 4500".into()
            ])
        );
        assert_eq!(start_command(Toggle::NightLight, false), None);
        assert_eq!(start_command(Toggle::StayAwake, true), None);
    }

    #[test]
    fn night_light_is_hyprsunsets_temperature() {
        assert_eq!(night_light_from("4500\n"), Some(true));
        assert_eq!(night_light_from("6500"), Some(false));
        assert_eq!(night_light_from("error"), None, "not running");
    }

    #[test]
    fn a_flag_follows_the_change() {
        let d = std::env::temp_dir().join(format!("wol-toggle-flag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (c, s) = (d.join("c"), d.join("s"));
        let f = state_file(Toggle::NightLight, &c, &s);
        set_flag(Toggle::NightLight, &c, &s, true).unwrap();
        assert!(is_on(Toggle::NightLight, &c, &s, |p| p.exists()) && f.exists());
        set_flag(Toggle::NightLight, &c, &s, false).unwrap();
        assert!(!f.exists());
        set_flag(Toggle::NightLight, &c, &s, false).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_state_flips() {
        assert!(target(false, None));
        assert!(!target(true, None));
        assert!(target(true, Some(true)));
        assert!(!target(false, Some(false)));
    }
}
