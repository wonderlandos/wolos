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

/// Whether a failed command should stop the toggle. Turning Stay Awake on when it already is
/// fails (the unit exists) and turning it off when it is not fails too: the state is right
/// either way, so only Night Light failures (no hyprsunset) stop the change.
pub fn must_succeed(t: Toggle) -> bool {
    t == Toggle::NightLight
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
        assert!(must_succeed(Toggle::NightLight) && !must_succeed(Toggle::StayAwake));
    }

    #[test]
    fn no_state_flips() {
        assert!(target(false, None));
        assert!(!target(true, None));
        assert!(target(true, Some(true)));
        assert!(!target(false, Some(false)));
    }
}
