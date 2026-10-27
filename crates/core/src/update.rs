//! System update plan with boot environments: new BE, upgrade inside it, activate on success.
//! Pure: produces argv lists; the caller runs them in order and stops on the first failure.

pub mod check;

/// Pending package names from `pkg version -U -vRL=`; None when output unknown.
pub fn parse_pending(output: Option<&str>) -> Option<Vec<String>> {
    Some(
        output?
            .lines()
            .filter(|l| l.contains("needs updating"))
            .filter_map(|l| {
                let pkg = l.split_whitespace().next()?;
                Some(pkg.rsplit_once('-').map_or(pkg, |(n, _)| n).to_owned())
            })
            .collect(),
    )
}

pub const PENDING_ARGV: [&str; 4] = ["pkg", "version", "-U", "-vRL="];

/// "2 package updates: a, b" / "+N more".
pub fn summary(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    let mut s = format!("{} package updates: {}", names.len(), shown.join(", "));
    if names.len() > max {
        s.push_str(&format!(", +{} more", names.len() - max));
    }
    s
}

/// Valid boot environment name: `wol-YYYYMMDD-HHMM` style, `[A-Za-z0-9._-]`, max 64.
pub fn is_valid_be(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// Steps: create BE, mount it, upgrade packages (and optionally base) inside it, unmount, activate (next boot).
pub fn plan(be: &str, mountpoint: &str, with_base: bool) -> Option<Vec<Vec<String>>> {
    if !is_valid_be(be) || !mountpoint.starts_with('/') {
        return None;
    }
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
    let mut steps = vec![
        v(&["bectl", "create", be]),
        v(&["bectl", "mount", be, mountpoint]),
        v(&["pkg", "-r", mountpoint, "upgrade", "-y"]),
    ];
    if with_base {
        steps.push(v(&["freebsd-update", "-b", mountpoint, "fetch", "install"]));
    }
    steps.push(v(&["bectl", "umount", be]));
    steps.push(v(&["bectl", "activate", be]));
    Some(steps)
}

/// Rollback after a failed step: drop the half-upgraded BE.
pub fn rollback(be: &str) -> Vec<Vec<String>> {
    vec![
        vec!["bectl".into(), "umount".into(), "-f".into(), be.into()],
        vec!["bectl".into(), "destroy".into(), "-F".into(), be.into()],
    ]
}

/// Restore points: ZFS boot environments (`bectl`). WolOS names its own `wol-*`; only those are
/// ever pruned, and never the one running now or the one booting next.
pub mod be {
    use serde::{Deserialize, Serialize};

    pub const LIST_ARGV: [&str; 3] = ["bectl", "list", "-H"];
    /// WolOS restore points kept; older `wol-*` boot environments are destroyed.
    pub const KEEP: usize = 5;
    pub const PREFIX: &str = "wol-";

    /// Why a restore point was made. Each kind is pruned on its own: installing an app never
    /// removes the point taken before the last system update.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Update,
        Install,
    }

    impl Kind {
        pub const fn prefix(self) -> &'static str {
            match self {
                Kind::Update => "wol-before-update-",
                Kind::Install => "wol-before-install-",
            }
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BootEnv {
        pub name: String,
        /// Running now (`N`).
        pub active: bool,
        /// Boots next (`R`, or `T` once).
        pub next_boot: bool,
        pub mountpoint: Option<String>,
        pub space: String,
        /// `YYYY-MM-DD HH:MM` as bectl prints it (sorts as text).
        pub created: String,
    }

    /// `bectl list -H`: tab-separated name, active flags, mountpoint, space, created.
    pub fn parse(text: &str) -> Vec<BootEnv> {
        text.lines()
            .filter_map(|l| {
                let f: Vec<&str> = if l.contains('\t') {
                    l.split('\t').map(str::trim).collect()
                } else {
                    l.split_whitespace().collect()
                };
                if f.len() < 5 || !super::is_valid_be(f[0]) {
                    return None;
                }
                let flags = f[1];
                Some(BootEnv {
                    name: f[0].into(),
                    active: flags.contains('N'),
                    next_boot: flags.contains('R') || flags.contains('T'),
                    mountpoint: (f[2] != "-").then(|| f[2].into()),
                    space: f[3].into(),
                    created: f[4..].join(" "),
                })
            })
            .collect()
    }

    /// `wol update rollback` without a name: a pending switch to another BE is cancelled (boot the
    /// running one again); otherwise the newest BE that is not running.
    pub fn rollback_target(bes: &[BootEnv]) -> Option<&BootEnv> {
        let running = bes.iter().find(|b| b.active);
        if bes.iter().any(|b| b.next_boot && !b.active) {
            return running;
        }
        bes.iter()
            .filter(|b| !b.active)
            .max_by(|a, b| a.created.cmp(&b.created).then(a.name.cmp(&b.name)))
    }

    /// Names to destroy so at most `keep` `wol-*` BEs remain; oldest first. Never the running or
    /// next-boot BE, never one WolOS did not create.
    pub fn prune(bes: &[BootEnv], keep: usize) -> Vec<String> {
        prune_prefix(bes, PREFIX, keep)
    }

    /// [`prune`] limited to names starting with `prefix` (one [`Kind`] of restore point).
    pub fn prune_prefix(bes: &[BootEnv], prefix: &str, keep: usize) -> Vec<String> {
        let mut ours: Vec<&BootEnv> = bes.iter().filter(|b| b.name.starts_with(prefix)).collect();
        ours.sort_by(|a, b| a.created.cmp(&b.created).then(a.name.cmp(&b.name)));
        let extra = ours.len().saturating_sub(keep);
        ours.into_iter()
            .take(extra)
            .filter(|b| !b.active && !b.next_boot)
            .map(|b| b.name.clone())
            .collect()
    }

    pub fn activate_argv(name: &str) -> Vec<String> {
        vec!["bectl".into(), "activate".into(), name.into()]
    }

    /// `-o` also destroys the origin snapshot.
    pub fn destroy_argv(name: &str) -> Vec<String> {
        vec!["bectl".into(), "destroy".into(), "-o".into(), name.into()]
    }

    /// Boot environments on `host`; None without bectl (not root on ZFS) or when it fails.
    pub fn read(host: &dyn crate::system::Host) -> Option<Vec<BootEnv>> {
        host.run(&LIST_ARGV).map(|t| parse(&t))
    }

    /// `wol-before-update-<unix secs>` or `wol-before-install-<unix secs>`.
    pub fn restore_point_name(kind: Kind, now: u64) -> String {
        format!("{}{now}", kind.prefix())
    }

    /// Steps wol-admind runs before changing packages in place: create a restore point, then prune
    /// old ones. `list` = `bectl list -H` output; None = no boot environments (not root on ZFS):
    /// no steps, the update runs without a restore point.
    pub fn restore_point(
        list: Option<&str>,
        kind: Kind,
        now: u64,
    ) -> Option<(Vec<String>, Vec<Vec<String>>)> {
        let bes = parse(list?);
        let name = restore_point_name(kind, now);
        let create = vec!["bectl".into(), "create".into(), name];
        // The new BE is the newest: keep KEEP - 1 of the existing ones.
        let destroy = prune_prefix(&bes, kind.prefix(), KEEP.saturating_sub(1))
            .iter()
            .map(|n| destroy_argv(n))
            .collect();
        Some((create, destroy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_table() {
        let cases = [
            (None, None),
            (Some(""), Some("")),
            (
                Some(
                    "curl-8.1 <   needs updating (remote has 8.2)\nfoo-bar-1.0_2,1 <   needs updating\n",
                ),
                Some("curl,foo-bar"),
            ),
            (
                Some("pkg-1.20   ?   orphaned: ports-mgmt/pkg\nfoo-1 >   succeeds index\n"),
                Some(""),
            ),
        ];
        for (out, want) in cases {
            assert_eq!(parse_pending(out).map(|l| l.join(",")).as_deref(), want);
        }
    }

    #[test]
    fn summary_table() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(summary(&n(&["a", "b"]), 5), "2 package updates: a, b");
        assert_eq!(
            summary(&n(&["a", "b", "c", "d", "e", "f", "g"]), 5),
            "7 package updates: a, b, c, d, e, +2 more"
        );
    }

    #[test]
    fn be_plan() {
        let p = plan("wol-20261026", "/mnt/upd", false).unwrap();
        let s: Vec<String> = p.iter().map(|a| a.join(" ")).collect();
        assert_eq!(
            s,
            [
                "bectl create wol-20261026",
                "bectl mount wol-20261026 /mnt/upd",
                "pkg -r /mnt/upd upgrade -y",
                "bectl umount wol-20261026",
                "bectl activate wol-20261026"
            ]
        );
        assert_eq!(plan("x", "/m", true).unwrap().len(), 6);
        for bad in ["", "-x", "a b", "a;b", "../x"] {
            assert!(plan(bad, "/m", false).is_none(), "{bad}");
        }
        assert!(plan("x", "rel", false).is_none());
        assert_eq!(rollback("x")[1].join(" "), "bectl destroy -F x");
    }

    const BECTL: &str = "default\tNR\t/\t21.5G\t2026-01-01 10:00\n\
wol-update-1761500000\t-\t-\t1.1G\t2026-10-26 18:13\n\
wol-before-update-1761000000\t-\t-\t300M\t2026-10-20 09:00\n\
bad name\t-\t-\t1M\t2026-10-20 09:00\n";

    #[test]
    fn bectl_list_fixture() {
        let b = be::parse(BECTL);
        assert_eq!(b.len(), 3, "invalid names skipped");
        assert_eq!(
            (
                b[0].name.as_str(),
                b[0].active,
                b[0].next_boot,
                b[0].mountpoint.as_deref()
            ),
            ("default", true, true, Some("/"))
        );
        assert_eq!(b[1].created, "2026-10-26 18:13");
        assert_eq!(
            (b[1].active, b[1].next_boot, b[1].mountpoint.clone()),
            (false, false, None)
        );
        // Space-aligned output (no -H) still parses; T = boots next once.
        let s = be::parse("wol-x   T   -   1M   2026-10-27 08:00\n");
        assert!(s[0].next_boot && s[0].created == "2026-10-27 08:00");
    }

    #[test]
    fn read_from_host() {
        let h = crate::system::fake::FakeHost::new(true);
        assert_eq!(be::read(&h), None, "no bectl: not root on ZFS");
        h.command("bectl list -H", BECTL);
        assert_eq!(be::read(&h).unwrap().len(), 3);
    }

    #[test]
    fn rollback_target_table() {
        let b = be::parse(BECTL);
        assert_eq!(
            be::rollback_target(&b).unwrap().name,
            "wol-update-1761500000"
        );
        // Update applied, not rebooted yet: rollback = keep booting the running BE.
        let pending = be::parse(
            "default\tN\t/\t1G\t2026-01-01 10:00\nwol-update-2\tR\t-\t1G\t2026-10-26 18:13\n",
        );
        assert_eq!(be::rollback_target(&pending).unwrap().name, "default");
        assert!(
            be::rollback_target(&be::parse("default\tNR\t/\t1G\t2026-01-01 10:00\n")).is_none()
        );
    }

    #[test]
    fn prune_never_destroys_running_next_or_foreign() {
        let mut text = String::from("default\t-\t-\t1G\t2025-01-01 10:00\n");
        for i in 0..8 {
            let flags = match i {
                0 => "N",
                1 => "R",
                _ => "-",
            };
            text.push_str(&format!(
                "wol-{i}\t{flags}\t-\t1G\t2026-10-0{} 10:00\n",
                i + 1
            ));
        }
        let b = be::parse(&text);
        assert_eq!(be::prune(&b, 3), ["wol-2", "wol-3", "wol-4"]);
        assert!(
            be::prune(&b, 0)
                .iter()
                .all(|n| n != "wol-0" && n != "wol-1" && n != "default")
        );
        assert!(be::prune(&b, 20).is_empty());
        assert_eq!(
            be::destroy_argv("wol-2").join(" "),
            "bectl destroy -o wol-2"
        );
    }

    #[test]
    fn restore_point_before_in_place_updates() {
        assert_eq!(
            be::restore_point(None, be::Kind::Update, 7),
            None,
            "no ZFS: no restore point"
        );
        let (create, destroy) =
            be::restore_point(Some(BECTL), be::Kind::Update, 1761600000).unwrap();
        assert_eq!(
            create.join(" "),
            "bectl create wol-before-update-1761600000"
        );
        assert!(destroy.is_empty());
        assert!(is_valid_be(&be::restore_point_name(
            be::Kind::Install,
            u64::MAX
        )));
        let many: String = (0..6)
            .map(|i| {
                format!(
                    "wol-before-update-{i}\t-\t-\t1G\t2026-10-0{} 10:00\n",
                    i + 1
                )
            })
            .collect::<String>()
            + "default\tNR\t/\t1G\t2025-01-01 10:00\n";
        let (_, destroy) = be::restore_point(Some(&many), be::Kind::Update, 1).unwrap();
        let d: Vec<String> = destroy.iter().map(|a| a.join(" ")).collect();
        assert_eq!(
            d,
            [
                "bectl destroy -o wol-before-update-0",
                "bectl destroy -o wol-before-update-1"
            ],
            "KEEP incl. the new one"
        );
        // Installing an app prunes only install points: the pre-update ones stay.
        let (create, destroy) = be::restore_point(Some(&many), be::Kind::Install, 2).unwrap();
        assert_eq!(create[2], "wol-before-install-2");
        assert!(destroy.is_empty(), "{destroy:?}");
    }
}
