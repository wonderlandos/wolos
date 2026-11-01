// SPDX-License-Identifier: GPL-2.0-or-later
//! The Arch Linux image (ADR 0016): mkarchiso profile `wolos-iso/profile`, pinned to one Arch Linux Archive
//! date, `[wolos]` before the Arch repos, linux-wolos only, the `[wolos]` repo build and the release
//! manifest. Scripts run against fakes; nothing here needs root or the network.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn every_hyprland_session_runs_at_scale_one() {
    // The installer, setup and login sessions use the desktop's monitor rule: an automatic scale
    // made the logical screen smaller than the welcome card in a VM.
    let want = "monitor = , preferred, auto, 1";
    for c in ["hyprland", "installer", "setup", "login"] {
        let s = read(&format!("desktop/overlay/usr/share/wolos/hypr/{c}.conf"));
        assert!(s.lines().any(|l| l == want), "{c}.conf lacks {want}");
    }
}
