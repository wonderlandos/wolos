//! WolOS package mirrors (ADR 0019): repo order `[wolos]` -> `[wolos-cachy]` -> Arch, the CPU gate
//! for `[wolos-cachy]` (x86-64-v3 builds), and latency ranking of `/etc/pacman.d/wolos-mirrorlist`.
//! Pure: the caller reads files and probes servers.

/// Mirrorlist of `[wolos]` and `[wolos-cachy]` (package `wolos-mirrorlist`).
pub const MIRRORLIST: &str = "/etc/pacman.d/wolos-mirrorlist";
pub const PACMAN_CONF: &str = "/etc/pacman.conf";
/// The optimized repo, on only when the CPU runs its builds.
pub const CACHY_REPO: &str = "wolos-cachy";
/// The x86-64 level the `[wolos-cachy]` builds need.
pub const CACHY_LEVEL: u8 = 3;
/// Kernel built for x86-64-v3 (same boot files as linux-wolos, which it replaces).
pub const KERNEL_V3: &str = "linux-wolos-v3";

const V2: &[&str] = &[
    "cx16", "lahf_lm", "popcnt", "pni", "sse4_1", "sse4_2", "ssse3",
];
const V3: &[&str] = &[
    "avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "abm", "movbe", "xsave",
];
const V4: &[&str] = &["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"];

/// x86-64 micro-architecture level (1-4) from `/proc/cpuinfo` (the first `flags` line);
/// 0 when there is none (not x86-64).
pub fn x86_64_level(cpuinfo: &str) -> u8 {
    let Some(flags) = cpuinfo.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "flags").then_some(v)
    }) else {
        return 0;
    };
    let have: Vec<&str> = flags.split_whitespace().collect();
    let all = |want: &[&str]| want.iter().all(|f| have.contains(f));
    match (all(V2), all(V3), all(V4)) {
        (true, true, true) => 4,
        (true, true, false) => 3,
        (true, false, _) => 2,
        _ => 1,
    }
}

/// `[wolos-cachy]` on this CPU.
pub fn cachy_supported(cpuinfo: &str) -> bool {
    x86_64_level(cpuinfo) >= CACHY_LEVEL
}

/// Whether `[wolos-cachy]` is enabled in `pacman.conf`.
pub fn cachy_enabled(conf: &str) -> bool {
    conf.lines().any(|l| l.trim() == format!("[{CACHY_REPO}]"))
}

/// `pacman.conf` with the `[wolos-cachy]` section on (uncommented) or off (`#`-commented).
/// Idempotent; other sections stay as they are.
pub fn gate(conf: &str, on: bool) -> String {
    let head = format!("[{CACHY_REPO}]");
    let mut inside = false;
    let mut out = String::with_capacity(conf.len() + 8);
    for l in conf.lines() {
        let bare = l.trim_start_matches('#').trim();
        if bare.starts_with('[') {
            inside = bare == head;
        } else if bare.is_empty() {
            inside = false;
        }
        // Inside the section: a header or an option line (`Key = value`), not a prose comment.
        let option = bare.starts_with('[') || bare.split_once('=').is_some();
        if inside && option {
            out.push_str(if on { "" } else { "#" });
            out.push_str(bare);
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    out
}

/// The system kernel package for this CPU: `linux-wolos-v3` where the CPU runs it and a repo
/// offers it, else `linux-wolos`.
pub fn kernel_package(cpuinfo: &str, available: bool) -> &'static str {
    if available && cachy_supported(cpuinfo) {
        KERNEL_V3
    } else {
        crate::update::check::SYSTEM_KERNEL
    }
}

/// `Server = ` lines of a mirrorlist, in order (commented ones skipped).
pub fn servers(mirrorlist: &str) -> Vec<String> {
    mirrorlist
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Server"))
        .filter_map(|r| r.trim_start().strip_prefix('='))
        .map(|s| s.trim().to_owned())
        .collect()
}

/// URL that answers for a server: its `[wolos]` database.
pub fn probe_url(server: &str) -> String {
    format!(
        "{}/wolos.db",
        server
            .replace("$repo", "wolos")
            .replace("$arch", "x86_64")
            .trim_end_matches('/')
    )
}

/// Probe every URL, in parallel (divide and conquer with `shared::par::join`); results in order.
pub fn probe_all(
    urls: &[String],
    probe: &(dyn Fn(&str) -> Option<u64> + Sync),
) -> Vec<Option<u64>> {
    match urls {
        [] => vec![],
        [u] => vec![probe(u)],
        _ => {
            let (a, b) = urls.split_at(urls.len() / 2);
            let (mut l, r) = shared::par::join(|| probe_all(a, probe), || probe_all(b, probe));
            l.extend(r);
            l
        }
    }
}

/// The mirrorlist with its `Server` lines reordered fastest first (`latency_ms` per server in
/// file order; unreachable ones last, ties keep the file order). Comments stay on top.
pub fn rank(mirrorlist: &str, latency_ms: &[Option<u64>]) -> String {
    let mut out = String::new();
    let mut lines: Vec<(Option<u64>, usize, &str)> = Vec::new();
    for l in mirrorlist.lines() {
        if servers(l).is_empty() {
            out.push_str(l);
            out.push('\n');
        } else {
            let i = lines.len();
            lines.push((latency_ms.get(i).copied().flatten(), i, l));
        }
    }
    lines.sort_by_key(|(ms, i, _)| (ms.is_none(), ms.unwrap_or(0), *i));
    for (_, _, l) in lines {
        out.push_str(l);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const V3_CPU: &str = "processor\t: 0\nflags\t\t: fpu cx16 lahf_lm popcnt pni sse4_1 sse4_2 \
ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave\nprocessor\t: 1\nflags\t\t: fpu\n";

    #[test]
    fn levels_from_cpuinfo() {
        assert_eq!(x86_64_level(V3_CPU), 3);
        let v4 = V3_CPU.replacen(
            "xsave",
            "xsave avx512f avx512bw avx512cd avx512dq avx512vl",
            1,
        );
        assert_eq!(x86_64_level(&v4), 4);
        // No movbe (some early AVX2 parts): v2 only.
        assert_eq!(x86_64_level(&V3_CPU.replacen(" movbe", "", 1)), 2);
        assert_eq!(x86_64_level("flags\t: fpu sse2\n"), 1);
        assert_eq!(x86_64_level("Features\t: fp asimd\n"), 0);
        assert!(cachy_supported(V3_CPU));
        assert!(!cachy_supported(&V3_CPU.replacen(" avx2", "", 1)));
        assert_eq!(kernel_package(V3_CPU, true), "linux-wolos-v3");
        assert_eq!(kernel_package(V3_CPU, false), "linux-wolos");
        assert_eq!(kernel_package("flags: sse2\n", true), "linux-wolos");
    }

    const CONF: &str = "[options]\nSigLevel = Required\n\n[wolos]\nInclude = /etc/pacman.d/wolos-mirrorlist\n\n\
# Optimized builds: on only on x86-64-v3 CPUs (`wol mirror gate`).\n#[wolos-cachy]\n#Include = /etc/pacman.d/wolos-mirrorlist\n\n\
[core]\nServer = https://archive.archlinux.org/repos/2026/10/20/$repo/os/$arch\n";

    #[test]
    fn gate_toggles_only_the_cachy_section() {
        assert!(!cachy_enabled(CONF));
        let on = gate(CONF, true);
        assert!(cachy_enabled(&on));
        assert!(
            on.contains("\n[wolos-cachy]\nInclude = /etc/pacman.d/wolos-mirrorlist\n\n[core]\n")
        );
        assert!(on.contains("# Optimized builds"), "prose comment kept");
        assert_eq!(gate(&on, true), on, "idempotent");
        assert_eq!(gate(&on, false), CONF);
        assert_eq!(gate(CONF, false), CONF);
        let order: Vec<&str> = on.lines().filter(|l| l.starts_with('[')).collect();
        assert_eq!(order, ["[options]", "[wolos]", "[wolos-cachy]", "[core]"]);
    }

    const LIST: &str = "# WolOS mirrors\nServer = https://a.example/$repo-2026-10-20\n\
#Server = https://off.example/x\nServer = https://b.example/2026-10-20/$repo/$arch\n\
Server=https://c.example/$repo\n";

    #[test]
    fn servers_probe_and_rank() {
        let s = servers(LIST);
        assert_eq!(s.len(), 3);
        assert_eq!(
            probe_url(&s[1]),
            "https://b.example/2026-10-20/wolos/x86_64/wolos.db"
        );
        assert_eq!(
            probe_url(&s[0]),
            "https://a.example/wolos-2026-10-20/wolos.db"
        );
        let urls: Vec<String> = (0..7).map(|i| format!("u{i}")).collect();
        let got = probe_all(&urls, &|u: &str| {
            u[1..].parse::<u64>().ok().filter(|n| n % 3 != 0)
        });
        assert_eq!(got, [None, Some(1), Some(2), None, Some(4), Some(5), None]);
        assert!(probe_all(&[], &|_: &str| Some(1)).is_empty());
        let r = rank(LIST, &[Some(90), None, Some(12)]);
        assert_eq!(
            r,
            "# WolOS mirrors\n#Server = https://off.example/x\nServer=https://c.example/$repo\n\
Server = https://a.example/$repo-2026-10-20\nServer = https://b.example/2026-10-20/$repo/$arch\n"
        );
        assert_eq!(rank(LIST, &[]).lines().count(), LIST.lines().count());
    }
}
