// SPDX-License-Identifier: BSD-2-Clause
//! Startup: the user's systemd services that start with the session. The app reads the unit
//! files with the user manager's `ListUnitFiles` (D-Bus) and switches them with
//! `EnableUnitFiles`/`DisableUnitFiles`; this module only decides what is listed and how.
//! Static, masked, generated and alias units are not a choice a person makes, so they are not
//! listed; services the system ships can be seen but only the person's own can be switched.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// `syncthing.service`.
    pub name: String,
    pub path: String,
    pub enabled: bool,
    /// In the person's own unit folder (`~/.config/systemd/user`): theirs to switch.
    pub mine: bool,
}

/// (path, state) pairs as `ListUnitFiles` returns them, sorted by name.
pub fn rows(files: &[(String, String)], config_home: &str) -> Vec<Unit> {
    let own = format!("{}/systemd/user/", config_home.trim_end_matches('/'));
    let mut v: Vec<Unit> = files
        .iter()
        .filter_map(|(path, state)| {
            let name = path.rsplit('/').next()?;
            let enabled = match state.as_str() {
                "enabled" => true,
                "disabled" => false,
                _ => return None,
            };
            (name.ends_with(".service") && !name.contains('@')).then(|| Unit {
                name: name.to_owned(),
                path: path.clone(),
                enabled,
                mine: path.starts_with(&own),
            })
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

/// A unit the person may switch: a plain service name (no folder, no template) whose file is
/// in their own unit folder `<config_home>/systemd/user/`. Checked again right before the
/// switch, whatever the page showed.
pub fn own_unit(config_home: &std::path::Path, name: &str) -> bool {
    let plain =
        name.ends_with(".service") && !name.contains(['/', '@', '\\']) && !name.starts_with('.');
    plain
        && std::fs::symlink_metadata(config_home.join("systemd/user").join(name))
            .is_ok_and(|m| m.is_file())
}

/// `Description=` of the `[Unit]` section.
pub fn description(unit: &str) -> Option<String> {
    let mut in_unit = false;
    for l in unit.lines().map(str::trim) {
        if l.starts_with('[') {
            in_unit = l == "[Unit]";
        } else if in_unit && let Some(d) = l.strip_prefix("Description=") {
            let d = d.trim();
            if !d.is_empty() {
                return Some(d.to_owned());
            }
        }
    }
    None
}
