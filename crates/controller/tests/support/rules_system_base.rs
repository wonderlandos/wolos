// SPDX-License-Identifier: GPL-2.0-or-later
//! Baselines and helpers of `rules_system.rs`: the growth-only lists of known violations, the
//! repo readers, and the parsers for scripts, unit files, sysctl and compositor config.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

// ---------------------------------------------------------------- baselines (growth-only)

/// RS-001: scripts whose shebang is not exactly `#!/bin/sh`.
pub(crate) const B_SHEBANG: &[&str] = &[];
/// RS-004: scripts without `set -eu` (or `set -u` plus a comment naming `set -e` and why not).
pub(crate) const B_SET_EU: &[&str] = &[
    "desktop/overlay/etc/NetworkManager/dispatcher.d/50-wolos-online",
    "desktop/overlay/usr/bin/wol-greeter-session",
    "desktop/overlay/usr/bin/wol-session",
    "desktop/overlay/usr/bin/wol-volume",
    "desktop/overlay/usr/bin/wol-zoom",
    "desktop/overlay/usr/share/wolos/widgets/game-friends/friends.sh",
    "files/usr/bin/wolos-update-slot",
    "files/usr/lib/systemd/system-sleep/wolos-power",
    "files/usr/lib/wolos/installer-session",
    "files/usr/lib/wolos/wol-install",
    "files/usr/share/wolos/boot/41_wolos",
];
/// RS-005: scripts that use `eval`.
pub(crate) const B_EVAL: &[&str] = &[
    "desktop/overlay/usr/bin/wol-session",
    "files/usr/lib/wolos/verify-gpu.sh",
];
/// RS-007: scripts with an `rm -rf` whose variable path has no `${var:?}` guard.
pub(crate) const B_RM_RF: &[&str] = &[
    "files/usr/lib/wolos/boot-setup",
    "files/usr/lib/wolos/clam-update",
    "files/usr/lib/wolos/install-run",
    "files/usr/lib/wolos/xguard-update",
];
/// RS-008: scripts hiding stderr of a disk/key/boot/db write.
pub(crate) const B_QUIET_WRITES: &[&str] = &[
    "files/usr/lib/wolos/boot-keys",
    "files/usr/lib/wolos/boot-setup",
];
/// RS-003: scripts with a short header.
pub(crate) const B_HEADER: &[&str] = &["desktop/overlay/usr/bin/wol-session"];
/// RS-006: scripts that run `sh -c` on a variable.
pub(crate) const B_SH_C: &[&str] = &["files/usr/lib/wolos/verify-gpu.sh"];
/// RS-018: scripts naming /home/ (not in a comment).
pub(crate) const B_HOME: &[&str] = &["files/usr/lib/wolos/install-run"];
/// RS-022: scripts calling `which`.
pub(crate) const B_WHICH: &[&str] = &["files/usr/lib/wolos/install-run"];
/// RS-011: unquoted expansions per script (heuristic count; the number may only fall).
pub(crate) const B_UNQUOTED: &[(&str, usize)] = &[
    ("desktop/hyprland/build-plugins.sh", 5),
    ("desktop/overlay/usr/bin/wol-greeter-session", 1),
    ("desktop/overlay/usr/bin/wol-session", 6),
    ("files/usr/bin/wolos-update-slot", 1),
    ("files/usr/lib/wolos/autoinstall", 2),
    ("files/usr/lib/wolos/boot-keys", 1),
    ("files/usr/lib/wolos/boot-setup", 3),
    ("files/usr/lib/wolos/clam-update", 4),
    ("files/usr/lib/wolos/esp-sync", 4),
    ("files/usr/lib/wolos/install-check", 1),
    ("files/usr/lib/wolos/install-run", 11),
    ("files/usr/lib/wolos/modules-install", 6),
    ("files/usr/lib/wolos/update-check", 2),
    ("files/usr/lib/wolos/verify-gpu.sh", 1),
];
/// RS-014: scripts using bashisms under `#!/bin/sh`.
pub(crate) const B_BASHISM: &[&str] = &[];
/// RS-015: scripts using backtick substitution.
pub(crate) const B_BACKTICK: &[&str] = &[];
/// RS-016: scripts naming a fixed /tmp path.
pub(crate) const B_TMP: &[&str] = &[];
/// RS-020: scripts with a curl call lacking `-f` or a time limit.
pub(crate) const B_CURL: &[&str] = &[
    "files/usr/lib/wolos/update-check",
    "files/usr/lib/wolos/xguard-update",
];
/// RS-021: scripts using `read` without `-r`.
pub(crate) const B_READ_R: &[&str] = &[];
/// RS-024: scripts in /usr/lib/wolos nothing references.
pub(crate) const B_DEAD_SCRIPT: &[&str] = &["files/usr/lib/wolos/modules-enable-ai-claude"];
/// RS-027: scripts that mktemp without a trap.
pub(crate) const B_MKTEMP_TRAP: &[&str] = &[
    "files/usr/lib/wolos/install-run",
    "files/usr/lib/wolos/modules-install",
];
/// RS-031: root services without ProtectSystem=.
pub(crate) const B_PROTECT_SYSTEM: &[&str] = &[
    "files/usr/lib/systemd/system/wolos-boot-ok.service",
    "files/usr/lib/systemd/system/wolos-firstboot.service",
    "files/usr/lib/systemd/system/wolos-hw.service",
    "files/usr/lib/systemd/system/wolos-mok-enroll.service",
    "files/usr/lib/systemd/system/wolos-power.service",
    "files/usr/lib/systemd/system/wolos-shm-limit.service",
    "files/usr/lib/systemd/system/wolos-tpm-bind.service",
    "files/usr/lib/systemd/system/wolos-tpm-reseal.service",
    "files/usr/lib/systemd/system/wolos-verity-check.service",
    "files/usr/lib/systemd/system/wolos-xguard.service",
];
/// RS-036: long-running services without a Restart= policy.
pub(crate) const B_RESTART: &[&str] = &["files/usr/lib/systemd/system/wolos-clam.service"];
/// RS-038: units with After=network-online.target and no Wants= for it.
pub(crate) const B_NET_WANTS: &[&str] = &[
    "files/usr/lib/systemd/system/wolos-clam-update.service",
    "files/usr/lib/systemd/system/wolos-report.service",
    "files/usr/lib/systemd/system/wolos-update-check.service",
    "files/usr/lib/systemd/system/wolos-xguard-update.service",
    "files/usr/lib/systemd/user/wolos-app-update.service",
];
/// RS-043: units whose Exec* line starts a shell.
pub(crate) const B_UNIT_SHELL: &[&str] = &[];
/// RS-050: hardened root units without the `# Hardening` explanation block.
pub(crate) const B_HARDENING_COMMENT: &[&str] = &[];
/// RS-060: sysctl keys docs/modules/security.md does not mention.
pub(crate) const B_SYSCTL_DOC: &[&str] = &[
    "fs.file-max",
    "fs.protected_regular",
    "kernel.nmi_watchdog",
    "kernel.printk",
    "kernel.sched_autogroup_enabled",
    "kernel.split_lock_mitigate",
    "kernel.watchdog_thresh",
    "net.core.netdev_max_backlog",
    "vm.dirty_background_bytes",
    "vm.dirty_bytes",
    "vm.dirty_writeback_centisecs",
    "vm.page-cluster",
    "vm.swappiness",
    "vm.vfs_cache_pressure",
];
/// RS-067: sysctl keys with no comment on or just above the line.
pub(crate) const B_SYSCTL_COMMENT: &[&str] = &[];
/// RS-080: binds the shortcut sheet has no name for yet (added while the sheet was being extended).
pub(crate) const B_BIND_ROWS: &[&str] = &[];
/// RS-082: colour literals in hyprland.conf (rgb/rgba/#hex). Themes own every colour (hypr/wolos/theme.conf);
/// the one left is `misc:background_color = rgb(000000)`, the start-up black (ADR 0012), not a theme colour.
pub(crate) const B_HYPR_COLOURS: usize = 1;
/// RS-089: compositor confs not starting with a comment.
pub(crate) const B_HYPR_HEADER: &[&str] = &["desktop/overlay/usr/share/wolos/hypr/hypridle.conf"];
/// RS-091: duplicate key combinations in hyprland.conf.
pub(crate) const B_HYPR_DUP: &[&str] = &[];
/// RS-101: .desktop files whose absolute Icon is not in the repo.
pub(crate) const B_ICON: &[&str] = &["desktop/overlay/usr/share/wolos/applications/steam.desktop"];
/// RS-120: files under files/ no test names.
pub(crate) const B_UNTESTED: &[&str] = &[
    "files/etc/dkms/framework.conf.d/wolos-sign.conf",
    "files/etc/skel/.curlrc",
    "files/etc/skel/.wgetrc",
    "files/usr/bin/wolos-update-slot",
    "files/usr/lib/systemd/system/wolos-clam-update.service",
    "files/usr/lib/systemd/system/wolos-clam.service",
    "files/usr/lib/systemd/system/wolos-clam.socket",
    "files/usr/lib/systemd/system/wolos-xguard-sweep.timer",
    "files/usr/lib/systemd/system/wolos-xguard-update.timer",
    "files/usr/lib/udev/rules.d/60-wolos-ioscheduler.rules",
    "files/usr/lib/udev/rules.d/70-wolos-usb-writeback.rules",
    "files/usr/lib/wolos/install-boot",
    "files/usr/lib/wolos/modules-enable-ai-claude",
    "files/usr/lib/wolos/modules-install",
    "files/usr/share/libalpm/hooks/wolos-steam-entry.hook",
    "files/usr/share/plymouth/themes/wolos/wolos.plymouth",
    "files/usr/share/plymouth/themes/wolos/wolos.script",
    "files/usr/share/wolos/agents/skills/wolos/SKILL.md",
    "files/usr/share/wolos/boot/41_wolos",
    "files/usr/share/wolos/modules/ai-sessions.json",
    "files/usr/share/wolos/modules/creative.json",
];

// ---------------------------------------------------------------- helpers

pub(crate) fn root() -> PathBuf {
    super::siblings::root()
}

pub(crate) fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if n == "target" || n == ".git" || n == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Repo-relative, '/'-separated paths of every file under `top`, sorted.
pub(crate) fn files_under(top: &str) -> Vec<String> {
    let r = root();
    let mut v = Vec::new();
    walk(&r.join(top), &mut v);
    let mut s: Vec<String> = v
        .iter()
        .filter_map(|p| p.strip_prefix(&r).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    s.sort();
    s
}

pub(crate) fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_default()
}

pub(crate) fn exists(rel: &str) -> bool {
    root().join(rel).exists()
}

pub(crate) fn ratchet(rule: &str, found: BTreeSet<String>, baseline: &[&str]) {
    let base: BTreeSet<&str> = baseline.iter().copied().collect();
    let new: Vec<&String> = found
        .iter()
        .filter(|f| !base.contains(f.as_str()))
        .collect();
    assert!(
        new.is_empty(),
        "{rule}: new violations (fix them; baselines only shrink):\n{}",
        new.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    for b in base {
        if !found.contains(b) {
            eprintln!("{rule}: baseline entry {b} is compliant now; remove it");
        }
    }
}

pub(crate) fn ratchet_count(
    rule: &str,
    found: BTreeMap<String, usize>,
    baseline: &[(&str, usize)],
) {
    let mut bad = Vec::new();
    for (f, n) in &found {
        let allowed = baseline.iter().find(|(b, _)| b == f).map_or(0, |(_, n)| *n);
        if *n > allowed {
            bad.push(format!("(\"{f}\", {n}),  // allowed {allowed}"));
        } else if *n < allowed {
            eprintln!("{rule}: {f} is down to {n}; lower the baseline");
        }
    }
    // A script whose count fell to 0 (or that is gone) has no entry in `found`.
    for (f, _) in baseline.iter().filter(|(f, _)| !found.contains_key(*f)) {
        eprintln!("{rule}: {f} is down to 0; drop it from the baseline");
    }
    assert!(bad.is_empty(), "{rule}: counts grew:\n{}", bad.join("\n"));
}

/// Comment text of a shell/conf line: whole-line `#` comments only.
pub(crate) fn is_comment(l: &str) -> bool {
    l.trim_start().starts_with('#')
}

/// Non-blank, non-comment lines of a shell script outside here-document bodies: (number, text).
pub(crate) fn code_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut end: Option<String> = None;
    for (i, l) in text.lines().enumerate() {
        if let Some(e) = &end {
            if l.trim() == e {
                end = None;
            }
            continue;
        }
        if l.trim().is_empty() || is_comment(l) {
            continue;
        }
        if let Some(p) = l.find("<<") {
            let w = l[p + 2..].trim_start_matches('-').trim_start();
            let w: String = w
                .trim_start_matches(['\'', '"'])
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !w.is_empty() && !l[p..].starts_with("<<<") {
                end = Some(w);
            }
        }
        out.push((i + 1, l.to_string()));
    }
    out
}

/// Shell scripts shipped in files/ and desktop/ (first line a shell shebang).
pub(crate) fn scripts() -> &'static Vec<String> {
    static S: OnceLock<Vec<String>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v = Vec::new();
        for f in files_under("files")
            .into_iter()
            .chain(files_under("desktop"))
        {
            let t = read(&f);
            let first = t.lines().next().unwrap_or("");
            if first.starts_with("#!")
                && first.contains("sh")
                && !first.contains("python")
                && !first.contains("lua")
            {
                v.push(f);
            }
        }
        v
    })
}

pub(crate) fn on_scripts(f: impl Fn(&str, &str) -> bool) -> BTreeSet<String> {
    scripts()
        .iter()
        .filter(|s| f(s, &read(s)))
        .cloned()
        .collect()
}

/// Rule over code lines: the script is a violator when any code line satisfies `bad`.
pub(crate) fn any_code(bad: impl Fn(&str) -> bool) -> BTreeSet<String> {
    on_scripts(|_, t| code_lines(t).iter().any(|(_, l)| bad(l)))
}

pub(crate) fn word_in(l: &str, w: &str) -> bool {
    let b = l.as_bytes();
    let mut from = 0;
    while let Some(p) = l[from..].find(w) {
        let s = from + p;
        let e = s + w.len();
        let before =
            s == 0 || !(b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_' || b[s - 1] == b'-');
        let after = e >= b.len() || !(b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'-');
        if before && after {
            return true;
        }
        from = e;
    }
    false
}

// ----- unit-file parsing

pub(crate) const SYS_UNITS: &str = "files/usr/lib/systemd/system";
pub(crate) const USER_UNITS: &str = "files/usr/lib/systemd/user";

pub(crate) fn unit_files(dir: &str, ext: &[&str]) -> Vec<String> {
    let r = root().join(dir);
    let mut v: Vec<String> = std::fs::read_dir(&r)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| ext.iter().any(|x| n.ends_with(x)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.into_iter().map(|n| format!("{dir}/{n}")).collect()
}

pub(crate) fn all_units() -> Vec<String> {
    let e = [".service", ".timer", ".socket"];
    let mut v = unit_files(SYS_UNITS, &e);
    v.extend(unit_files(USER_UNITS, &e));
    v
}

pub(crate) fn system_services() -> Vec<String> {
    unit_files(SYS_UNITS, &[".service"])
}

/// (section, key, value) of an ini-style file, comments dropped.
pub(crate) fn ini(rel: &str) -> Vec<(String, String, String)> {
    let mut sec = String::new();
    let mut out = Vec::new();
    for l in read(rel).lines().map(str::trim) {
        if l.starts_with('#') || l.starts_with(';') || l.is_empty() {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            sec = l[1..l.len() - 1].to_string();
        } else if let Some((k, v)) = l.split_once('=') {
            out.push((sec.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

pub(crate) fn vals(rel: &str, sec: &str, key: &str) -> Vec<String> {
    ini(rel)
        .into_iter()
        .filter(|(s, k, _)| s == sec && k == key)
        .map(|t| t.2)
        .collect()
}

pub(crate) fn has(rel: &str, sec: &str, key: &str) -> bool {
    !vals(rel, sec, key).is_empty()
}

pub(crate) fn base(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

pub(crate) fn is_root_service(u: &str) -> bool {
    !has(u, "Service", "User")
}

/// Executable names the repo builds or ships: crate bins, files/usr/bin, overlay/usr/bin.
pub(crate) fn known_bins() -> &'static BTreeSet<String> {
    static S: OnceLock<BTreeSet<String>> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = BTreeSet::new();
        for f in files_under("crates") {
            if let Some(n) = f.strip_suffix(".rs")
                && n.contains("/src/bin/")
            {
                s.insert(base(n).to_string());
            }
            if f.ends_with("/Cargo.toml") {
                let mut in_bin = false;
                for l in read(&f).lines().map(str::trim) {
                    if l.starts_with('[') {
                        in_bin = l == "[[bin]]";
                    } else if in_bin
                        && l.starts_with("name")
                        && let Some(v) = l.split('"').nth(1)
                    {
                        s.insert(v.to_string());
                    }
                }
            }
        }
        for f in files_under("files/usr/bin")
            .into_iter()
            .chain(files_under("desktop/overlay/usr/bin"))
        {
            s.insert(base(&f).to_string());
        }
        s
    })
}

/// Programs of packages the image installs (not built here); each is used by a shipped file.
pub(crate) const EXTERNAL: &[&str] = &[
    "mount",
    "systemctl",
    "sh",
    "foot",
    "hyprctl",
    "playerctl",
    "lxqt-policykit-agent",
    "gnome-keyring-daemon",
    "dbus-update-activation-environment",
    "systemd-detect-virt",
    "xdg-desktop-portal-hyprland",
    "rm",
    "touch",
    "flatpak",
];

/// `/usr/bin/x`, `/usr/lib/wolos/x` or a bare name resolves to something shipped or packaged.
pub(crate) fn command_exists(cmd: &str) -> bool {
    if let Some(n) = cmd.strip_prefix("/usr/lib/wolos/") {
        return exists(&format!("files/usr/lib/wolos/{n}"));
    }
    let n = cmd.rsplit('/').next().unwrap_or(cmd);
    known_bins().contains(n) || EXTERNAL.contains(&n)
}

// ----- sysctl

pub(crate) fn sysctl_pairs() -> Vec<(String, String, String)> {
    let mut v = Vec::new();
    for f in files_under("files/usr/lib/sysctl.d") {
        for l in read(&f).lines().map(str::trim) {
            if l.starts_with('#') || l.starts_with(';') || l.is_empty() {
                continue;
            }
            if let Some((k, val)) = l.split_once('=') {
                v.push((f.clone(), k.trim().to_string(), val.trim().to_string()));
            }
        }
    }
    v
}

// ----- hyprland

pub(crate) const HYPR_DIR: &str = "desktop/overlay/usr/share/wolos/hypr";

pub(crate) fn hypr_confs() -> Vec<String> {
    files_under(HYPR_DIR)
        .into_iter()
        .filter(|f| f.ends_with(".conf"))
        .collect()
}

pub(crate) fn hypr_code(rel: &str) -> Vec<(usize, String)> {
    read(rel)
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !is_comment(l))
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

pub(crate) fn strip_trailing_comment(l: &str) -> &str {
    l.split(" #").next().unwrap_or(l).trim_end()
}
