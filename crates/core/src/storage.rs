//! ZFS-aware disk usage for Settings > General > Storage and About (df fallback without a pool).
use crate::system::Host;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Category {
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Storage {
    /// Pool name, or the mount point without ZFS.
    pub name: String,
    pub total: u64,
    pub used: u64,
    /// Largest first, empty ones dropped.
    pub categories: Vec<Category>,
    pub encrypted: Option<bool>,
    /// Bytes held only by snapshots.
    pub snapshots: u64,
    pub boot_environments: u32,
    pub zfs: bool,
}

fn fields3(line: &str) -> Option<(String, u64, u64)> {
    let f: Vec<&str> = line.split('\t').collect();
    (f.len() == 3 && !f[0].is_empty()).then(|| {
        Some((
            f[0].to_string(),
            f[1].trim().parse().ok()?,
            f[2].trim().parse().ok()?,
        ))
    })?
}

/// `zpool list -Hp -o name,size,alloc`: (name, size, allocated) of the first pool.
pub fn parse_zpool_list(text: &str) -> Option<(String, u64, u64)> {
    fields3(text.lines().find(|l| !l.trim().is_empty())?)
}

/// `zfs list -Hp -o name,usedbydataset,usedbysnapshots -r <pool>`: (dataset, bytes, snapshot bytes).
pub fn parse_zfs_list(text: &str) -> Vec<(String, u64, u64)> {
    text.lines().filter_map(fields3).collect()
}

/// Category of a dataset path relative to its pool ("" = the pool's root dataset).
fn category(rel: &str) -> &'static str {
    let under = |p: &str| rel == p || rel.starts_with(&format!("{p}/"));
    if under("ROOT") {
        "System"
    } else if under("home") || under("usr/home") {
        "Home folders"
    } else if under("var/log") {
        "Logs"
    } else if under("var/tmp") || under("tmp") {
        "Temporary files"
    } else if under("usr/ports") || under("usr/src") {
        "Developer"
    } else if under("var") {
        "System data"
    } else {
        "Other"
    }
}

/// Dataset bytes summed per category, largest first, empty ones dropped.
pub fn categorize(pool: &str, rows: &[(String, u64, u64)]) -> Vec<Category> {
    let mut out: Vec<Category> = Vec::new();
    for (name, bytes, _) in rows {
        let Some(rel) = name.strip_prefix(pool) else {
            continue;
        };
        if !rel.is_empty() && !rel.starts_with('/') {
            continue;
        }
        let c = category(rel.trim_start_matches('/'));
        match out.iter_mut().find(|x| x.name == c) {
            Some(x) => x.bytes += bytes,
            None => out.push(Category {
                name: c.into(),
                bytes: *bytes,
            }),
        }
    }
    out.retain(|c| c.bytes > 0);
    out.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    out
}

/// `zfs get -H -o value encryption <pool>`.
pub fn parse_encryption(text: &str) -> Option<bool> {
    match text.trim() {
        "" | "-" => None,
        "off" => Some(false),
        _ => Some(true),
    }
}

/// `bectl list -H`: one line per boot environment.
pub fn parse_bectl(text: &str) -> u32 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

/// FreeBSD: the first ZFS pool; otherwise (no pool, Linux) the root file system from df.
pub fn read(h: &dyn Host) -> Option<Storage> {
    if h.is_freebsd()
        && let Some((pool, total, used)) = h
            .run(&["zpool", "list", "-Hp", "-o", "name,size,alloc"])
            .and_then(|t| parse_zpool_list(&t))
    {
        let rows = h
            .run(&[
                "zfs",
                "list",
                "-Hp",
                "-o",
                "name,usedbydataset,usedbysnapshots",
                "-r",
                &pool,
            ])
            .map(|t| parse_zfs_list(&t))
            .unwrap_or_default();
        return Some(Storage {
            categories: categorize(&pool, &rows),
            snapshots: rows.iter().map(|r| r.2).sum(),
            encrypted: h
                .run(&["zfs", "get", "-H", "-o", "value", "encryption", &pool])
                .and_then(|t| parse_encryption(&t)),
            boot_environments: h
                .run(&["bectl", "list", "-H"])
                .map_or(0, |t| parse_bectl(&t)),
            name: pool,
            total,
            used,
            zfs: true,
        });
    }
    let root = crate::system::stats::DiskUsage::read(h)
        .into_iter()
        .find(|d| d.mount == "/")?;
    Some(Storage {
        name: root.mount,
        total: root.total.max(0) as u64,
        used: root.used.max(0) as u64,
        ..Default::default()
    })
}

/// Decimal units like macOS: "512 MB", "41.2 GB", "1.5 TB".
pub fn human(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = "bytes";
    for u in ["KB", "MB", "GB", "TB", "PB"] {
        if v < 1000.0 {
            break;
        }
        v /= 1000.0;
        unit = u;
    }
    let n = if v < 100.0 {
        let s = format!("{v:.1}");
        s.strip_suffix(".0").map_or(s.clone(), str::to_string)
    } else {
        format!("{v:.0}")
    };
    format!("{n} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const ZPOOL: &str = "zroot\t497142202368\t131208486912\n";
    const ZFS: &str = "zroot\t98304\t0\n\
zroot/ROOT\t98304\t0\n\
zroot/ROOT/default\t21474836480\t1073741824\n\
zroot/ROOT/15.1-update\t1073741824\t0\n\
zroot/home\t98304\t0\n\
zroot/home/alice\t53687091200\t2147483648\n\
zroot/usr\t98304\t0\n\
zroot/usr/home\t1048576\t0\n\
zroot/usr/ports\t3221225472\t0\n\
zroot/usr/src\t1073741824\t0\n\
zroot/var\t98304\t0\n\
zroot/var/log\t104857600\t0\n\
zroot/var/tmp\t52428800\t0\n\
zroot/var/crash\t0\t0\n\
zroot/var/mail\t1048576\t0\n\
zroot/tmp\t10485760\t0\n\
zroot/vm\t2147483648\t0\n";

    fn names(c: &[Category]) -> Vec<(&str, u64)> {
        c.iter().map(|c| (c.name.as_str(), c.bytes)).collect()
    }

    #[test]
    fn zpool_and_zfs_rows() {
        assert_eq!(
            parse_zpool_list(ZPOOL),
            Some(("zroot".into(), 497142202368, 131208486912))
        );
        assert_eq!(
            parse_zpool_list("zroot\t1\t2\nbackup\t3\t4\n").map(|p| p.0),
            Some("zroot".into())
        );
        assert_eq!(parse_zpool_list(""), None);
        assert_eq!(parse_zpool_list("zroot\tx\t2\n"), None);
        let rows = parse_zfs_list(ZFS);
        assert_eq!(rows.len(), 17);
        assert_eq!(
            rows[2],
            ("zroot/ROOT/default".into(), 21474836480, 1073741824)
        );
        assert_eq!(parse_zfs_list("junk\nzroot\t1\n"), vec![]);
    }

    #[test]
    fn categories_by_dataset() {
        let c = categorize("zroot", &parse_zfs_list(ZFS));
        assert_eq!(
            names(&c),
            [
                ("Home folders", 53687091200 + 98304 + 1048576),
                ("System", 21474836480 + 1073741824 + 98304),
                ("Developer", 3221225472 + 1073741824),
                ("Other", 2147483648 + 98304 + 98304),
                ("Logs", 104857600),
                ("Temporary files", 52428800 + 10485760),
                ("System data", 98304 + 1048576),
            ]
        );
        assert!(categorize("zroot", &[]).is_empty());
    }

    #[test]
    fn encryption_and_boot_environments() {
        assert_eq!(parse_encryption("off\n"), Some(false));
        assert_eq!(parse_encryption("aes-256-gcm\n"), Some(true));
        assert_eq!(parse_encryption("-\n"), None);
        assert_eq!(parse_encryption(""), None);
        assert_eq!(
            parse_bectl(
                "default\tNR\t/\t21.5G\t2026-01-01 10:00\n15.1-update\t-\t-\t1G\t2026-02-01 10:00\n"
            ),
            2
        );
        assert_eq!(parse_bectl(""), 0);
    }

    #[test]
    fn reads_zfs_pool() {
        let h = FakeHost::new(true);
        h.command("zpool list -Hp -o name,size,alloc", ZPOOL)
            .command(
                "zfs list -Hp -o name,usedbydataset,usedbysnapshots -r zroot",
                ZFS,
            )
            .command("zfs get -H -o value encryption zroot", "off\n")
            .command("bectl list -H", "default\tNR\t/\t21.5G\t2026-01-01 10:00\n");
        let s = read(&h).unwrap();
        assert_eq!(
            (s.name.as_str(), s.total, s.used, s.zfs),
            ("zroot", 497142202368, 131208486912, true)
        );
        assert_eq!(s.encrypted, Some(false));
        assert_eq!(s.snapshots, 1073741824 + 2147483648);
        assert_eq!(s.boot_environments, 1);
        assert_eq!(s.categories[0].name, "Home folders");
    }

    #[test]
    fn falls_back_to_df() {
        let df = "Filesystem 1024-blocks Used Avail Capacity Mounted on\n/dev/ada0p2 100000000 40000000 60000000 40% /\ndevfs 1 1 0 100% /dev\n";
        for bsd in [true, false] {
            let h = FakeHost::new(bsd);
            h.command("df -kP", df);
            let s = read(&h).unwrap();
            assert_eq!(
                (s.name.as_str(), s.total, s.used, s.zfs),
                ("/", 102400000000, 40960000000, false)
            );
            assert!(s.categories.is_empty());
            assert_eq!(s.encrypted, None);
        }
        assert_eq!(read(&FakeHost::new(true)), None);
    }

    #[test]
    fn human_sizes() {
        for (b, want) in [
            (0, "0 bytes"),
            (999, "999 bytes"),
            (512_000_000, "512 MB"),
            (41_200_000_000, "41.2 GB"),
            (9_990_000, "10 MB"),
            (9_940_000, "9.9 MB"),
            (1_500_000_000_000, "1.5 TB"),
            (123_400_000_000, "123 GB"),
            (2_000, "2 KB"),
        ] {
            assert_eq!(human(b), want, "{b}");
        }
    }
}
