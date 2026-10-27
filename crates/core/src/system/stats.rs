//! Stats (cpu, mem, load, uptime), disks, temperatures, GPUs. The 1 Hz sample never forks on Linux;
//! on FreeBSD only `swapinfo -k`, cached 30 s.
use super::{Cached, Host, Load, pairs};
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

    /// FreeBSD kern.cp_time: long[5] user nice sys intr idle.
    pub fn from_cp_time(raw: &[u8]) -> Option<Self> {
        if raw.len() < 40 {
            return None;
        }
        let t: Vec<i64> = (0..5)
            .map(|i| i64::from_ne_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap_or_default()))
            .collect();
        let total: i64 = t.iter().sum();
        Some(Self {
            busy: total - t[4],
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

    /// `swapinfo -k`: device lines (1K blocks), "Total" ignored.
    pub fn parse_swapinfo(text: &str) -> (i64, i64) {
        let (mut t, mut u) = (0, 0);
        for l in text.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 3 || f[0] == "Total" {
                continue;
            }
            if let (Ok(a), Ok(b)) = (f[1].parse::<i64>(), f[2].parse::<i64>()) {
                t += a * 1024;
                u += b * 1024;
            }
        }
        (t, u)
    }
}

pub fn parse_proc_uptime(text: &str) -> Option<f64> {
    text.split(' ').next()?.trim().parse().ok()
}

/// kern.boottime (struct timeval) -> seconds since boot.
pub fn uptime_from_boottime(tv: &[u8], now: f64) -> Option<f64> {
    (tv.len() >= 8)
        .then(|| now.floor() - i64::from_ne_bytes(tv[..8].try_into().unwrap_or_default()) as f64)
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
    swap: Cached<(i64, i64)>,
    buf: [u8; 256],
}

impl StatsSource {
    pub fn new(h: Arc<dyn Host>) -> Self {
        let h2 = h.clone();
        let swap = Cached::new(h.clone(), 30.0, (0, 0), move || {
            h2.run(&["swapinfo", "-k"])
                .map(|t| MemoryStats::parse_swapinfo(&t))
        });
        Self {
            h,
            prev: None,
            swap,
            buf: [0; 256],
        }
    }

    pub fn read(&mut self) -> SystemStats {
        let h = &*self.h;
        let cpu = if h.is_freebsd() {
            let mut b = [0u8; 40];
            h.sysctl_into("kern.cp_time", &mut b)
                .and_then(|n| CpuTimes::from_cp_time(&b[..n]))
        } else {
            let n = h.read_head("/proc/stat", &mut self.buf).unwrap_or(0);
            std::str::from_utf8(&self.buf[..n])
                .ok()
                .and_then(CpuTimes::parse_proc_stat)
        };
        let pct = cpu.map_or(0.0, |c| {
            CpuTimes::usage_percent(self.prev.unwrap_or_default(), c)
        });
        self.prev = cpu.or(self.prev);
        let uptime = if h.is_freebsd() {
            h.sysctl_bytes("kern.boottime")
                .and_then(|tv| uptime_from_boottime(&tv, h.now()))
        } else {
            h.read_text("/proc/uptime")
                .and_then(|t| parse_proc_uptime(&t))
        };
        SystemStats {
            cpu_percent: pct,
            memory: self.memory(),
            load: h.load_average(),
            uptime_secs: uptime,
        }
    }

    fn memory(&self) -> MemoryStats {
        let h = &*self.h;
        if !h.is_freebsd() {
            return MemoryStats::parse_meminfo(&h.read_text("/proc/meminfo").unwrap_or_default());
        }
        let i = |n: &str| h.sysctl_i64(n).unwrap_or(0);
        let page = i("hw.pagesize");
        let total = i("vm.stats.vm.v_page_count") * page;
        let free = (i("vm.stats.vm.v_free_count") + i("vm.stats.vm.v_inactive_count")) * page;
        let (st, su) = self.swap.get();
        MemoryStats {
            total,
            used: total - free,
            swap_total: st,
            swap_used: su,
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
    pub fn from_deci_kelvin(dk: i64) -> f64 {
        (dk - 2731) as f64 / 10.0
    }

    pub fn read(h: &dyn Host) -> Vec<Self> {
        let mut v = Vec::new();
        if h.is_freebsd() {
            let mut i = 0;
            while let Some(dk) = h.sysctl_i64(&format!("dev.cpu.{i}.temperature")) {
                v.push(Self {
                    name: format!("cpu{i}"),
                    celsius: Self::from_deci_kelvin(dk),
                });
                i += 1;
            }
            return v;
        }
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
        v
    }
}

pub mod gpu {
    use super::Host;

    /// `pciconf -lv`: display-class devices as "vendor device".
    pub fn parse_pciconf(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let (mut vendor, mut device, mut display) = (String::new(), None::<String>, false);
        let flush = |out: &mut Vec<String>, v: &str, d: &Option<String>, disp: bool| {
            if let (true, Some(d)) = (disp, d) {
                out.push(format!("{v} {d}").trim().to_owned());
            }
        };
        for l in text.lines().filter(|l| !l.is_empty()) {
            if !l.starts_with(char::is_whitespace) {
                flush(&mut out, &vendor, &device, display);
                vendor.clear();
                device = None;
                display = l.contains("class=0x03");
                continue;
            }
            let Some((k, v)) = l.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('\'').to_owned();
            match k.trim() {
                "vendor" => vendor = v,
                "device" => device = Some(v),
                _ => {}
            }
        }
        flush(&mut out, &vendor, &device, display);
        out
    }

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
        if h.is_freebsd() {
            h.run(&["pciconf", "-lv"]).map(|t| parse_pciconf(&t))
        } else {
            h.run(&["lspci", "-mm"]).map(|t| parse_lspci(&t))
        }
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeHost;
    use super::*;

    fn longs(v: &[i64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_ne_bytes()).collect()
    }

    #[test]
    fn cpu_parsers_and_usage() {
        assert_eq!(
            CpuTimes::parse_proc_stat("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4\n"),
            Some(CpuTimes {
                busy: 150,
                total: 1000
            })
        );
        assert_eq!(
            CpuTimes::from_cp_time(&longs(&[10, 5, 20, 5, 60])),
            Some(CpuTimes {
                busy: 40,
                total: 100
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
        let s = MemoryStats::parse_swapinfo(
            "Device          1K-blocks     Used    Avail Capacity\n/dev/ada0p3       2097152     1024  2096128     0%\n/dev/md0 1024 0 1024 0%\nTotal 2098176 1024 2097152 0%\n",
        );
        assert_eq!(s, (2098176 * 1024, 1024 * 1024));
    }

    #[test]
    fn uptime_parsers() {
        assert_eq!(parse_proc_uptime("3600.50 7000.00\n"), Some(3600.5));
        assert_eq!(
            uptime_from_boottime(&longs(&[1000, 0]), 1500.0),
            Some(500.0)
        );
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
        assert_eq!(Temperature::from_deci_kelvin(3231), 50.0);
        let pciconf = "hostb0@pci0:0:0:0: class=0x060000 rev=0x02 hdr=0x00 vendor=0x8086 device=0x4621\n    vendor     = 'Intel Corporation'\n    device     = 'Host bridge'\n    class      = bridge\nvgapci0@pci0:0:2:0: class=0x030000 rev=0x0c hdr=0x00 vendor=0x8086 device=0x46a8\n    vendor     = 'Intel Corporation'\n    device     = 'Alder Lake-UP3 GT2 [Iris Xe Graphics]'\n    class      = display\n    subclass   = VGA\n";
        assert_eq!(
            gpu::parse_pciconf(pciconf),
            ["Intel Corporation Alder Lake-UP3 GT2 [Iris Xe Graphics]"]
        );
        let lspci = "00:00.0 \"Host bridge\" \"Intel Corporation\" \"Device 4621\" -r02 \"Lenovo\" \"Device 3b3a\"\n00:02.0 \"VGA compatible controller\" \"Intel Corporation\" \"Alder Lake-UP3 GT2 [Iris Xe Graphics]\" -r0c -p00 \"Lenovo\" \"Device 3b3a\"\n";
        assert_eq!(
            gpu::parse_lspci(lspci),
            ["Intel Corporation Alder Lake-UP3 GT2 [Iris Xe Graphics]"]
        );
        let h = FakeHost::new(true);
        h.int("dev.cpu.0.temperature", 3231)
            .int("dev.cpu.1.temperature", 3331);
        assert_eq!(Temperature::read(&h).len(), 2);
        let l = FakeHost::new(false);
        l.file("/sys/class/thermal/thermal_zone0/temp", "40000")
            .file("/sys/class/thermal/thermal_zone0/type", "acpitz\n");
        assert_eq!(
            Temperature::read(&l),
            vec![Temperature {
                name: "acpitz".into(),
                celsius: 40.0
            }]
        );
    }

    #[test]
    fn linux_stats_without_forking() {
        let h = Arc::new(FakeHost::new(false));
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
    fn freebsd_stats_sysctl_and_delta() {
        let h = Arc::new(FakeHost::new(true));
        h.int("hw.pagesize", 4096)
            .int("vm.stats.vm.v_page_count", 100)
            .int("vm.stats.vm.v_free_count", 20)
            .int("vm.stats.vm.v_inactive_count", 30);
        h.raw("kern.boottime", longs(&[999_000, 0])).command(
            "swapinfo -k",
            "Device 1K-blocks Used Avail Capacity\n/dev/ada0p3 100 10 90 10%\n",
        );
        h.raw("kern.cp_time", longs(&[0, 0, 0, 0, 100]));
        let mut src = StatsSource::new(h.clone());
        src.read();
        h.raw("kern.cp_time", longs(&[25, 0, 25, 0, 150]));
        let s = src.read();
        assert_eq!(s.cpu_percent, 50.0);
        assert_eq!(
            s.memory,
            MemoryStats {
                total: 400 * 1024,
                used: 200 * 1024,
                swap_total: 100 * 1024,
                swap_used: 10 * 1024
            }
        );
        assert_eq!(s.uptime_secs, Some(1000.0));
    }
}
