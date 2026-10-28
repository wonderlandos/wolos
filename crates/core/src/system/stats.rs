//! Stats (cpu, mem, load, uptime), disks, temperatures, GPUs. The 1 Hz sample never forks:
//! /proc/stat, /proc/meminfo, /proc/uptime.
use super::{Host, Load, pairs};
use std::sync::Arc;

/// Cumulative CPU ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: i64,
    pub total: i64,
}

impl CpuTimes {
    /// /proc/stat aggregate "cpu" line: user nice system idle iowait irq softirq steal. Allocation-free.
    pub fn parse_proc_stat(text: &str) -> Option<Self> {
        let line = text.lines().find(|l| l.starts_with("cpu "))?;
        let (mut total, mut idle) = (0i64, 0i64);
        for (i, f) in line.split_ascii_whitespace().skip(1).take(8).enumerate() {
            let v: i64 = f.parse().ok()?;
            total += v;
            if i == 3 || i == 4 {
                idle += v;
            }
        }
        Some(Self {
            busy: total - idle,
            total,
        })
    }

    pub fn usage_percent(prev: Self, cur: Self) -> f64 {
        let dt = cur.total - prev.total;
        if dt <= 0 {
            0.0
        } else {
            (1000.0 * (cur.busy - prev.busy) as f64 / dt as f64).round() / 10.0
        }
    }
}

/// Bytes. Used excludes reclaimable cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct MemoryStats {
    pub total: i64,
    pub used: i64,
    pub swap_total: i64,
    pub swap_used: i64,
}

impl MemoryStats {
    pub fn parse_meminfo(text: &str) -> Self {
        let kv = pairs(text, ':');
        let kb = |k: &str| {
            kv.get(k)
                .and_then(|v| v.split(' ').next()?.parse::<i64>().ok())
                .map_or(0, |n| n * 1024)
        };
        let total = kb("MemTotal");
        let avail = if kv.contains_key("MemAvailable") {
            kb("MemAvailable")
        } else {
            kb("MemFree")
        };
        Self {
            total,
            used: total - avail,
            swap_total: kb("SwapTotal"),
            swap_used: kb("SwapTotal") - kb("SwapFree"),
        }
    }
}

pub fn parse_proc_uptime(text: &str) -> Option<f64> {
    text.split(' ').next()?.trim().parse().ok()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct SystemStats {
    pub cpu_percent: f64,
    pub memory: MemoryStats,
    pub load: Option<Load>,
    pub uptime_secs: Option<f64>,
}

/// Keeps the previous CPU sample so each read reports usage since the last one.
pub struct StatsSource {
    h: Arc<dyn Host>,
    prev: Option<CpuTimes>,
    buf: [u8; 256],
}

impl StatsSource {
    pub fn new(h: Arc<dyn Host>) -> Self {
        Self {
            h,
            prev: None,
            buf: [0; 256],
        }
    }

    pub fn read(&mut self) -> SystemStats {
        let h = &*self.h;
        let n = h.read_head("/proc/stat", &mut self.buf).unwrap_or(0);
        let cpu = std::str::from_utf8(&self.buf[..n])
            .ok()
            .and_then(CpuTimes::parse_proc_stat);
        let pct = cpu.map_or(0.0, |c| {
            CpuTimes::usage_percent(self.prev.unwrap_or_default(), c)
        });
        self.prev = cpu.or(self.prev);
        SystemStats {
            cpu_percent: pct,
            memory: MemoryStats::parse_meminfo(&h.read_text("/proc/meminfo").unwrap_or_default()),
            load: h.load_average(),
            uptime_secs: h
                .read_text("/proc/uptime")
                .and_then(|t| parse_proc_uptime(&t)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DiskUsage {
    pub mount: String,
    pub device: String,
    pub total: i64,
    pub used: i64,
    pub available: i64,
}

const PSEUDO: [&str; 13] = [
    "devfs",
    "tmpfs",
    "procfs",
    "linprocfs",
    "linsysfs",
    "fdescfs",
    "devtmpfs",
    "run",
    "efivarfs",
    "none",
    "overlay",
    "shm",
    "nullfs",
];

impl DiskUsage {
    pub fn parse_df(text: &str) -> Vec<Self> {
        text.lines()
            .skip(1)
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() < 6 || PSEUDO.contains(&f[0]) {
                    return None;
                }
                let n = |i: usize| f[i].parse::<i64>().ok().map(|v| v * 1024);
                let total = n(1).filter(|t| *t > 0)?;
                Some(Self {
                    mount: f[5..].join(" "),
                    device: f[0].into(),
                    total,
                    used: n(2)?,
                    available: n(3)?,
                })
            })
            .collect()
    }

    pub fn read(h: &dyn Host) -> Vec<Self> {
        h.run(&["df", "-kP"])
            .map(|t| Self::parse_df(&t))
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Temperature {
    pub name: String,
    pub celsius: f64,
}

impl Temperature {
    pub fn from_millidegrees(text: &str) -> Option<f64> {
        text.trim().parse::<i64>().ok().map(|m| m as f64 / 1000.0)
    }
    pub fn read(h: &dyn Host) -> Vec<Self> {
        let mut v = Vec::new();
        for z in h.list_dir("/sys/class/thermal") {
            let base = z.rsplit('/').next().unwrap_or("").to_owned();
            if !base.starts_with("thermal_zone") {
                continue;
            }
            if let Some(c) = h
                .read_text(&format!("{z}/temp"))
                .and_then(|t| Self::from_millidegrees(&t))
            {
                let name = h
                    .read_text(&format!("{z}/type"))
                    .map_or(base, |t| t.trim().to_owned());
                v.push(Self { name, celsius: c });
            }
        }
        // hwmon: coretemp / k10temp / amdgpu / nvme sensors, one entry per input.
        for d in h.list_dir("/sys/class/hwmon") {
            let Some(chip) = h.read_text(&format!("{d}/name")) else {
                continue;
            };
            for f in h.list_dir(&d) {
                let Some(input) = f
                    .rsplit('/')
                    .next()
                    .filter(|n| n.starts_with("temp") && n.ends_with("_input"))
                else {
                    continue;
                };
                let Some(c) = h.read_text(&f).and_then(|t| Self::from_millidegrees(&t)) else {
                    continue;
                };
                let label = h
                    .read_text(&format!("{d}/{}_label", input.trim_end_matches("_input")))
                    .map(|l| l.trim().to_owned());
                let name = match label {
                    Some(l) => format!("{} {l}", chip.trim()),
                    None => chip.trim().to_owned(),
                };
                v.push(Self { name, celsius: c });
            }
        }
        v
    }
}

pub mod gpu {
    use super::Host;

    /// `lspci -mm`: quoted fields slot "class" "vendor" "device".
    pub fn parse_lspci(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('"').collect();
                (f.len() > 6
                    && (f[1].contains("VGA") || f[1].contains("3D") || f[1].contains("Display")))
                .then(|| format!("{} {}", f[3], f[5]))
            })
            .collect()
    }

    pub fn read(h: &dyn Host) -> Vec<String> {
        h.run(&["lspci", "-mm"])
            .map(|t| parse_lspci(&t))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeHost;
    use super::*;

    #[test]
    fn cpu_parsers_and_usage() {
        assert_eq!(
            CpuTimes::parse_proc_stat("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4\n"),
            Some(CpuTimes {
                busy: 150,
                total: 1000
            })
        );
        for (b0, t0, b1, t1, pct) in [(100, 1000, 150, 1100, 50.0), (100, 1000, 100, 1000, 0.0)] {
            assert_eq!(
                CpuTimes::usage_percent(
                    CpuTimes {
                        busy: b0,
                        total: t0
                    },
                    CpuTimes {
                        busy: b1,
                        total: t1
                    }
                ),
                pct
            );
        }
    }

    #[test]
    fn memory_parsers() {
        let m = MemoryStats::parse_meminfo(
            "MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 400 kB\nSwapTotal: 200 kB\nSwapFree: 150 kB\n",
        );
        assert_eq!(
            m,
            MemoryStats {
                total: 1000 * 1024,
                used: 600 * 1024,
                swap_total: 200 * 1024,
                swap_used: 50 * 1024
            }
        );
    }

    #[test]
    fn uptime_parsers() {
        assert_eq!(parse_proc_uptime("3600.50 7000.00\n"), Some(3600.5));
    }

    #[test]
    fn df_keeps_real_filesystems() {
        let d = DiskUsage::parse_df(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\nzroot/ROOT/default 100 40 60 40% /\ndevfs 1 1 0 100% /dev\ntmpfs 50 0 50 0% /tmp\n/dev/sda1 200 50 150 25% /media/My Disk\n",
        );
        assert_eq!(
            d.iter().map(|x| x.mount.as_str()).collect::<Vec<_>>(),
            ["/", "/media/My Disk"]
        );
        assert_eq!(
            d[0],
            DiskUsage {
                mount: "/".into(),
                device: "zroot/ROOT/default".into(),
                total: 102400,
                used: 40960,
                available: 61440
            }
        );
    }

    #[test]
    fn temperatures_and_gpus() {
        assert_eq!(Temperature::from_millidegrees("45500\n"), Some(45.5));
        let lspci = "00:00.0 \"Host bridge\" \"Intel Corporation\" \"Device 4621\" -r02 \"Lenovo\" \"Device 3b3a\"\n00:02.0 \"VGA compatible controller\" \"Intel Corporation\" \"Alder Lake-UP3 GT2 [Iris Xe Graphics]\" -r0c -p00 \"Lenovo\" \"Device 3b3a\"\n";
        assert_eq!(
            gpu::parse_lspci(lspci),
            ["Intel Corporation Alder Lake-UP3 GT2 [Iris Xe Graphics]"]
        );
        let l = FakeHost::linux();
        l.file("/sys/class/thermal/thermal_zone0/temp", "40000")
            .file("/sys/class/thermal/thermal_zone0/type", "acpitz\n")
            .file("/sys/class/hwmon/hwmon2/name", "coretemp\n")
            .file("/sys/class/hwmon/hwmon2/temp1_input", "52000\n")
            .file("/sys/class/hwmon/hwmon2/temp1_label", "Package id 0\n")
            .file("/sys/class/hwmon/hwmon3/name", "nvme\n")
            .file("/sys/class/hwmon/hwmon3/temp1_input", "38850\n");
        assert_eq!(
            Temperature::read(&l),
            vec![
                Temperature {
                    name: "acpitz".into(),
                    celsius: 40.0
                },
                Temperature {
                    name: "coretemp Package id 0".into(),
                    celsius: 52.0
                },
                Temperature {
                    name: "nvme".into(),
                    celsius: 38.85
                }
            ]
        );
    }

    #[test]
    fn linux_stats_without_forking() {
        let h = Arc::new(FakeHost::linux());
        h.file("/proc/stat", "cpu 100 0 0 900 0 0 0 0\n")
            .file("/proc/meminfo", "MemTotal: 10 kB\nMemAvailable: 5 kB\n")
            .file("/proc/uptime", "60.0 1.0\n");
        h.set_load(Load {
            one: 1.0,
            five: 2.0,
            fifteen: 3.0,
        });
        let s = StatsSource::new(h.clone()).read();
        assert_eq!(
            (s.cpu_percent, s.memory.used, s.uptime_secs),
            (10.0, 5 * 1024, Some(60.0))
        );
        assert_eq!(
            s.load,
            Some(Load {
                one: 1.0,
                five: 2.0,
                fifteen: 3.0
            })
        );
        assert!(h.ran().is_empty());
    }

    #[test]
    fn real_proc_reads() {
        let h: Arc<dyn Host> = Arc::new(super::super::PosixHost);
        let s = StatsSource::new(h.clone()).read();
        assert!(s.memory.total > 0 && s.uptime_secs.is_some_and(|u| u > 0.0));
        assert!(s.load.is_some());
    }
}
