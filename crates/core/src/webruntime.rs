// SPDX-License-Identifier: BSD-2-Clause
//! Web-app runtime profiles (ADR 0042, track 1: apps that bundle their own web engine; the rows
//! are the flag rows of the per-app policy in `webpolicy`, docs/modules/webapps.md). One
//! profile per user, Balanced (default), Light or Full, decides the switches the session writes for
//! each role: pacman desktop apps on the shared engine (`electron-flags.conf`), the code editor
//! (`code-flags.conf`) and the browser (`atlas-flags.conf` plus its system policy file).
//!
//! Invariant: a switch or policy reaches a file only from a row in [`ROWS`] whose status is
//! [`Status::Baseline`] or [`Status::Measured`]; every [`Status::Candidate`] row stays in the table,
//! tested, but is never emitted until the web-app benchmark (tools/bench/webapps) has a number for
//! it. Why the file shape is so strict: the engine keeps only the last occurrence of a switch, so
//! list switches are merged into one; and the two readers split lines differently (one line = one
//! argument, or the whole file word-split), so every emitted line is a single word.
//!
//! Caches: the engine keeps its compiled-code cache and snapshot data in the app's own profile
//! directory by default; no row may move or disable them ([`FORBIDDEN`]).
use std::path::{Path, PathBuf};

/// How hard the runtime trades speed for memory and wakeups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Profile {
    /// Caps that never bite a normal session, throttling of what is hidden, GPU where it helps.
    Balanced,
    /// Smallest footprint: tighter heap caps, no GPU rasterisation (it costs GPU memory).
    Light,
    /// No caps and no extra throttling: the engine's own defaults plus the GPU.
    Full,
}

impl Profile {
    /// Every profile, default first.
    pub const ALL: [Profile; 3] = [Profile::Balanced, Profile::Light, Profile::Full];

    /// The word the profile file holds.
    pub fn id(self) -> &'static str {
        match self {
            Profile::Balanced => "balanced",
            Profile::Light => "light",
            Profile::Full => "full",
        }
    }

    /// The profile a word names (spaces around it ignored).
    pub fn parse(s: &str) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| p.id() == s.trim())
    }
}

/// Which app the flags are for; each role is one flag file the launchers read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Desktop apps from pacman on the shared engine.
    App,
    /// The code editor (its own launcher, its own file).
    Editor,
    /// The default browser.
    Browser,
}

impl Role {
    /// Every role.
    pub const ALL: [Role; 3] = [Role::App, Role::Editor, Role::Browser];

    /// File name under `$XDG_CONFIG_HOME` (fixed by each launcher).
    pub fn file(self) -> &'static str {
        match self {
            Role::App => "electron-flags.conf",
            Role::Editor => "code-flags.conf",
            Role::Browser => "atlas-flags.conf",
        }
    }
}

/// Whether the GPU may rasterise: a render node, not a VM adapter the compositor rejects, and
/// `wol hw compute` reporting a backend other than `cpu` (wol-session decides, see link.sh).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    Soft,
    Hw,
}

impl Gpu {
    /// Both tiers.
    pub const ALL: [Gpu; 2] = [Gpu::Soft, Gpu::Hw];

    /// The tier's word in the shipped file names (`<profile>-<soft|gpu>`).
    pub fn id(self) -> &'static str {
        match self {
            Gpu::Soft => "soft",
            Gpu::Hw => "gpu",
        }
    }
}

/// A measured gain from the web-app benchmark: what, before, after, and where it is written down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gain {
    pub metric: &'static str,
    pub before: &'static str,
    pub after: &'static str,
    /// Repo path (and anchor) of the benchmark record.
    pub source: &'static str,
}

/// Whether a row ships, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Shipped before this table for correctness, not speed (the reason says which audit).
    Baseline,
    /// Shipped: the benchmark measured it.
    Measured(Gain),
    /// Not shipped: waiting for a number.
    Candidate,
}

/// What one row sets in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    /// `--name` or `--name=value`.
    Switch(&'static str, Option<&'static str>),
    /// One entry of the merged `--enable-features=`.
    Feature(&'static str),
    /// One entry of the merged `--disable-features=`.
    NoFeature(&'static str),
    /// One `--js-flags=` value (one per role and profile: the line stays one word).
    JsFlag(&'static str),
    /// A browser policy (system-wide, so one value for every profile).
    Policy(&'static str, PolicyValue),
}

/// The value of a browser policy row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyValue {
    Bool(bool),
    Int(i64),
}

/// One row of [`ROWS`]: a knob, the roles and profiles it is for, and why it is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub knob: Knob,
    pub roles: &'static [Role],
    pub profiles: &'static [Profile],
    /// Only with [`Gpu::Hw`].
    pub needs_gpu: bool,
    pub reason: &'static str,
    pub status: Status,
}

impl Row {
    /// Whether the row reaches a file (Baseline or Measured, never Candidate).
    pub fn shipped(&self) -> bool {
        !matches!(self.status, Status::Candidate)
    }

    fn applies(&self, p: Profile, r: Role, g: Gpu) -> bool {
        self.profiles.contains(&p) && self.roles.contains(&r) && (!self.needs_gpu || g == Gpu::Hw)
    }
}

use Profile::{Balanced, Full, Light};
use Role::{App, Browser, Editor};
const ALL_P: &[Profile] = &Profile::ALL;
const APPS: &[Role] = &[App, Editor];
const EVERY: &[Role] = &Role::ALL;

/// Every knob WolOS considers, shipped or not. Order is the order in the files.
pub const ROWS: &[Row] = &[
    Row {
        id: "ozone-hint",
        knob: Knob::Switch("--ozone-platform-hint", Some("auto")),
        roles: APPS,
        profiles: ALL_P,
        needs_gpu: false,
        reason: "Wayland when the session has it, X11 otherwise (audit X1-16); the browser launcher sets it itself",
        status: Status::Baseline,
    },
    // Heap caps per role. A cap below what an app really needs ends in an out-of-memory crash of
    // the renderer, so each value waits for the benchmark's peak heap of the heaviest app per role.
    Row {
        id: "heap-app-light",
        knob: Knob::JsFlag("--max-old-space-size=512"),
        roles: &[App],
        profiles: &[Light],
        needs_gpu: false,
        reason: "a leaking chat or music app stops at its cap instead of taking the machine; value to be sized from the measured peak heap",
        status: Status::Candidate,
    },
    Row {
        id: "heap-app-balanced",
        knob: Knob::JsFlag("--max-old-space-size=1024"),
        roles: &[App],
        profiles: &[Balanced],
        needs_gpu: false,
        reason: "same leak stop with headroom for large chat workspaces",
        status: Status::Candidate,
    },
    Row {
        id: "heap-editor-light",
        knob: Knob::JsFlag("--max-old-space-size=2048"),
        roles: &[Editor],
        profiles: &[Light],
        needs_gpu: false,
        reason: "the editor window stays responsive with large workspaces; language servers run outside this heap",
        status: Status::Candidate,
    },
    Row {
        id: "heap-editor-balanced",
        knob: Knob::JsFlag("--max-old-space-size=4096"),
        roles: &[Editor],
        profiles: &[Balanced],
        needs_gpu: false,
        reason: "the engine's own default ceiling made explicit for the editor",
        status: Status::Candidate,
    },
    // Background throttling and timer alignment: hidden windows wake the CPU less often.
    Row {
        id: "intensive-wakeup-throttling",
        knob: Knob::Feature("IntensiveWakeUpThrottling"),
        roles: APPS,
        profiles: &[Balanced, Light],
        needs_gpu: false,
        reason: "timers of pages hidden for 5 minutes run once a minute: fewer idle wakeups",
        status: Status::Candidate,
    },
    Row {
        id: "align-wakeups",
        knob: Knob::Feature("AlignWakeUps"),
        roles: APPS,
        profiles: &[Light],
        needs_gpu: false,
        reason: "delayed tasks of all processes wake together on a coarse tick: the CPU sleeps longer",
        status: Status::Candidate,
    },
    Row {
        id: "browser-wakeup-throttling-policy",
        knob: Knob::Policy("IntensiveWakeUpThrottlingEnabled", PolicyValue::Bool(true)),
        roles: &[Browser],
        profiles: ALL_P,
        needs_gpu: false,
        reason: "the same throttling for background tabs, enforced so a site cannot opt out",
        status: Status::Candidate,
    },
    // Tab discard: background tabs give their memory back and reload when shown.
    Row {
        id: "browser-tab-discard-policy",
        knob: Knob::Policy("MemorySaverModeSavings", PolicyValue::Int(1)),
        roles: &[Browser],
        profiles: ALL_P,
        needs_gpu: false,
        reason: "discard background tabs at the balanced setting; check the name against the shipped browser's policy list before promotion",
        status: Status::Candidate,
    },
    // GPU rasterisation and zero-copy upload where the GPU is real.
    Row {
        id: "gpu-rasterization",
        knob: Knob::Switch("--enable-gpu-rasterization", None),
        roles: EVERY,
        profiles: &[Balanced, Full],
        needs_gpu: true,
        reason: "page tiles are drawn on the GPU: less CPU per scrolled frame",
        status: Status::Candidate,
    },
    Row {
        id: "zero-copy",
        knob: Knob::Switch("--enable-zero-copy", None),
        roles: EVERY,
        profiles: &[Balanced, Full],
        needs_gpu: true,
        reason: "raster output goes straight to GPU memory, no staging copy",
        status: Status::Candidate,
    },
];

/// Switches no row may emit: they move or disable the compiled-code and shader caches that live
/// in the app's profile directory, or turn background throttling off.
pub const FORBIDDEN: &[&str] = &[
    "--disk-cache-dir",
    "--disk-cache-size",
    "--disable-gpu-shader-disk-cache",
    "--user-data-dir",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
];

/// Shipped rows for one profile, role and GPU tier.
pub fn rows(p: Profile, r: Role, g: Gpu) -> impl Iterator<Item = &'static Row> {
    ROWS.iter()
        .filter(move |row| row.shipped() && row.applies(p, r, g))
}

/// The argument lines of every shipped row, one word each: plain switches in table order, then
/// the merged lists. The files use [`flags_with`] and the role's policy.
pub fn flags(p: Profile, r: Role, g: Gpu) -> Vec<String> {
    flags_with(p, r, g, &|_| true)
}

/// Shipped rows that `select` lets through (`webpolicy::allows` with the app's policy).
pub fn flags_with(p: Profile, r: Role, g: Gpu, select: &dyn Fn(&Row) -> bool) -> Vec<String> {
    let (mut out, mut on, mut off, mut js) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for row in rows(p, r, g).filter(|row| select(row)) {
        match row.knob {
            Knob::Switch(n, None) => out.push(n.to_owned()),
            Knob::Switch(n, Some(v)) => out.push(format!("{n}={v}")),
            Knob::Feature(f) => on.push(f),
            Knob::NoFeature(f) => off.push(f),
            Knob::JsFlag(f) => js.push(f),
            Knob::Policy(..) => {}
        }
    }
    if !on.is_empty() {
        out.push(format!("--enable-features={}", on.join(",")));
    }
    if !off.is_empty() {
        out.push(format!("--disable-features={}", off.join(",")));
    }
    if !js.is_empty() {
        out.push(format!("--js-flags={}", js.join(" ")));
    }
    out
}

/// The flag file text for `$XDG_CONFIG_HOME/<role file>`, with the role's policy
/// (`webpolicy::role_id`) deciding which shipped rows apply.
pub fn flags_file(p: Profile, r: Role, g: Gpu, policy: &crate::webpolicy::Policy) -> String {
    let mut s = format!(
        "# WolOS web-app runtime, profile {} ({} GPU), written from core/src/webruntime.rs.\n\
         # wol-session links it when you have no file of your own; replace the link to make it yours.\n",
        p.id(),
        if g == Gpu::Hw { "with" } else { "no" }
    );
    for l in flags_with(p, r, g, &|row| crate::webpolicy::allows(policy, row)) {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// Browser policies from shipped rows that `select` lets through (system-wide, merged into the
/// browser's policy file; the selection is the browser's own policy, `webpolicy::browser_policies`).
pub fn browser_policies(
    select: &dyn Fn(&Row) -> bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for row in ROWS.iter().filter(|r| r.shipped() && select(r)) {
        if let Knob::Policy(k, v) = row.knob {
            let v = match v {
                PolicyValue::Bool(b) => serde_json::Value::from(b),
                PolicyValue::Int(i) => serde_json::Value::from(i),
            };
            m.insert(k.to_owned(), v);
        }
    }
    m
}

/// Rows waiting for a number (what the benchmark should measure next).
pub fn candidates() -> impl Iterator<Item = &'static Row> {
    ROWS.iter().filter(|r| !r.shipped())
}

/// Shipped directory of one profile and tier, under `/usr/share/wolos`.
pub fn dir(p: Profile, g: Gpu) -> String {
    format!("webruntime/{}-{}", p.id(), g.id())
}

/// The user's profile, `$XDG_CONFIG_HOME/wolos/web-runtime`: one word. wol-session reads it.
pub const PROFILE_FILE: &str = "wolos/web-runtime";
/// The profile when the file is missing or says something else.
pub const DEFAULT: Profile = Profile::Balanced;

/// [`PROFILE_FILE`] under `config_home`.
pub fn profile_path(config_home: &Path) -> PathBuf {
    config_home.join(PROFILE_FILE)
}

/// Missing, unreadable or unknown: Balanced.
pub fn load_profile(config_home: &Path) -> Profile {
    std::fs::read_to_string(profile_path(config_home))
        .ok()
        .and_then(|s| Profile::parse(&s))
        .unwrap_or(DEFAULT)
}

/// Takes effect at the next session start (wol-session relinks the files).
pub fn save_profile(config_home: &Path, p: Profile) -> std::io::Result<()> {
    let f = profile_path(config_home);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    shared::atomic::atomic_write(&f, 0o644, format!("{}\n", p.id()).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../desktop/overlay/usr/share/wolos")
            .join(rel)
    }

    fn all() -> impl Iterator<Item = (Profile, Role, Gpu)> {
        Profile::ALL.into_iter().flat_map(|p| {
            Role::ALL
                .into_iter()
                .flat_map(move |r| Gpu::ALL.into_iter().map(move |g| (p, r, g)))
        })
    }

    #[test]
    fn every_row_has_an_id_a_reason_and_a_target() {
        let mut ids: Vec<_> = ROWS.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ROWS.len(), "row ids are unique");
        for r in ROWS {
            assert!(r.reason.len() >= 20, "{}: say why", r.id);
            assert!(!r.roles.is_empty() && !r.profiles.is_empty(), "{}", r.id);
            if let Status::Measured(g) = r.status {
                assert!(!g.metric.is_empty() && !g.before.is_empty() && !g.after.is_empty());
                assert!(
                    g.source.starts_with("docs/bench/"),
                    "{}: {}",
                    r.id,
                    g.source
                );
            }
            if let Knob::Policy(..) = r.knob {
                assert_eq!(r.roles, &[Browser], "{}: policies are the browser's", r.id);
                assert_eq!(r.profiles, ALL_P, "{}: system-wide, one value", r.id);
            }
        }
    }

    #[test]
    fn candidates_are_never_emitted() {
        for (p, r, g) in all() {
            let lines = flags(p, r, g);
            for c in candidates() {
                let hit = match c.knob {
                    Knob::Switch(n, _) => lines
                        .iter()
                        .any(|l| l == n || l.starts_with(&format!("{n}="))),
                    Knob::Feature(f) | Knob::NoFeature(f) | Knob::JsFlag(f) => {
                        lines.iter().any(|l| l.contains(f))
                    }
                    Knob::Policy(..) => false,
                };
                assert!(!hit, "candidate {} emitted for {p:?}/{r:?}/{g:?}", c.id);
            }
        }
        for c in candidates() {
            if let Knob::Policy(k, _) = c.knob {
                assert!(!browser_policies(&|_| true).contains_key(k), "{}", c.id);
            }
        }
    }

    #[test]
    fn lines_are_one_word_and_each_switch_once() {
        for (p, r, g) in all() {
            let lines = flags(p, r, g);
            let mut names: Vec<_> = lines
                .iter()
                .map(|l| l.split('=').next().unwrap().to_owned())
                .collect();
            for l in &lines {
                assert!(
                    l.starts_with("--") && !l.contains(char::is_whitespace),
                    "{l}"
                );
                assert!(!FORBIDDEN.contains(&l.split('=').next().unwrap()), "{l}");
            }
            names.sort();
            let n = names.len();
            names.dedup();
            assert_eq!(
                names.len(),
                n,
                "the engine keeps only the last of a switch: {lines:?}"
            );
            if r == Browser {
                assert!(
                    !lines.iter().any(|l| l.starts_with("--enable-features")),
                    "would replace the launcher's video-decode features"
                );
            }
        }
        // The same holds with every candidate promoted: the table is safe to ship row by row.
        for p in Profile::ALL {
            for r in Role::ALL {
                let js = ROWS
                    .iter()
                    .filter(|row| row.applies(p, r, Gpu::Hw) && matches!(row.knob, Knob::JsFlag(_)))
                    .count();
                assert!(js <= 1, "{p:?}/{r:?}: one js flag keeps the line one word");
                let feat = ROWS.iter().any(|row| {
                    row.applies(p, Browser, Gpu::Hw) && matches!(row.knob, Knob::Feature(_))
                });
                assert!(!feat, "{p:?}: no feature rows for the browser");
            }
        }
        for row in ROWS {
            if let Knob::Switch(n, _) = row.knob {
                assert!(!FORBIDDEN.contains(&n), "{}", row.id);
            }
        }
    }

    #[test]
    fn profiles_order_light_balanced_full() {
        // Full never throttles and never caps; Light never rasterises on the GPU.
        for row in ROWS {
            let throttles = matches!(row.knob, Knob::Feature(_) | Knob::JsFlag(_));
            if throttles {
                assert!(!row.profiles.contains(&Full), "{}", row.id);
            }
            if row.needs_gpu {
                assert!(!row.profiles.contains(&Light), "{}", row.id);
            }
        }
        // Today (no number yet) every profile ships the baseline only.
        for (p, r, g) in all() {
            let want: Vec<String> = if r == Browser {
                vec![]
            } else {
                vec!["--ozone-platform-hint=auto".into()]
            };
            if ROWS
                .iter()
                .all(|row| !matches!(row.status, Status::Measured(_)))
            {
                assert_eq!(flags(p, r, g), want, "{p:?}/{r:?}/{g:?}");
            }
        }
    }

    #[test]
    fn profile_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("wol-webruntime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_profile(&dir), Balanced);
        save_profile(&dir, Light).unwrap();
        assert_eq!(load_profile(&dir), Light);
        assert_eq!(
            std::fs::read_to_string(profile_path(&dir)).unwrap(),
            "light\n"
        );
        std::fs::write(profile_path(&dir), "turbo\n").unwrap();
        assert_eq!(load_profile(&dir), Balanced);
        std::fs::remove_dir_all(&dir).ok();
        for p in Profile::ALL {
            assert_eq!(Profile::parse(p.id()), Some(p));
        }
    }

    /// The image ships every profile x tier x role file (and the legacy default files, Balanced
    /// without GPU) exactly as generated; `WOL_REGEN=1` rewrites them.
    #[test]
    fn image_ships_the_generated_files() {
        let pol = |r: Role| {
            let d =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../files/usr/share/wolos/webpolicy");
            crate::webpolicy::load(&d, crate::webpolicy::role_id(r))
                .expect("role policy")
                .expect("valid")
        };
        let mut files: Vec<(PathBuf, String)> = all()
            .map(|(p, r, g)| {
                (
                    overlay(&format!("{}/{}", dir(p, g), r.file())),
                    flags_file(p, r, g, &pol(r)),
                )
            })
            .collect();
        for r in [App, Editor] {
            files.push((
                overlay(r.file()),
                flags_file(DEFAULT, r, Gpu::Soft, &pol(r)),
            ));
        }
        for (p, text) in files {
            if std::env::var_os("WOL_REGEN").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, &text).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(&p).ok(),
                Some(text),
                "{}",
                p.display()
            );
        }
    }
}
