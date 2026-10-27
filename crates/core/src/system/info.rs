//! "About this computer": /etc/os-release + sysctl (FreeBSD) or /proc (Linux).
use super::{Host, pairs};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SystemInfo {
    pub name: String,
    /// `kern.ident`: the kernel product (WolKernel; GENERIC on a stock kernel).
    pub kernel: String,
    /// `kern.ostype kern.osrelease` (FreeBSD 15.1-RELEASE-p6): the base the kernel reports.
    pub base: String,
    pub processor: String,
    pub memory: String,
}

pub const SYSCTL_KEYS: [&str; 6] = [
    "kern.ident",
    "kern.ostype",
    "kern.osrelease",
    "hw.model",
    "hw.ncpu",
    "hw.physmem",
];

impl SystemInfo {
    pub fn unknown() -> Self {
        Self {
            name: "WolOS".into(),
            kernel: "Unknown".into(),
            base: "Unknown".into(),
            processor: "Unknown".into(),
            memory: "Unknown".into(),
        }
    }

    pub fn from(os_release: &str, sysctl: &HashMap<&str, String>) -> Self {
        let os: HashMap<&str, &str> = os_release
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k, v.trim_matches('"')))
            .collect();
        let get = |k: &str| {
            sysctl
                .get(k)
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| "Unknown".into())
        };
        let memory = sysctl
            .get("hw.physmem")
            .and_then(|v| v.parse::<i64>().ok())
            .map_or_else(
                || "Unknown".into(),
                |b| format!("{} GB", (b + (1 << 29)) >> 30),
            );
        let model = get("hw.model");
        let processor = if model == "Unknown" {
            model
        } else {
            format!("{model} ({} cores)", get("hw.ncpu"))
        };
        let base = match (get("kern.ostype"), get("kern.osrelease")) {
            (t, r) if t != "Unknown" && r != "Unknown" => format!("{t} {r}"),
            _ => "Unknown".into(),
        };
        Self {
            name: os
                .get("PRETTY_NAME")
                .or_else(|| os.get("NAME"))
                .map_or("WolOS", |v| v)
                .into(),
            kernel: get("kern.ident"),
            base,
            processor,
            memory,
        }
    }

    pub fn read(h: &dyn Host) -> Self {
        let os = h
            .read_text("/etc/os-release")
            .or_else(|| h.read_text("/usr/local/etc/os-release"))
            .unwrap_or_default();
        let mut m = HashMap::new();
        if h.is_freebsd() {
            for k in SYSCTL_KEYS {
                let v = if k == "hw.ncpu" || k == "hw.physmem" {
                    h.sysctl_i64(k).map(|v| v.to_string())
                } else {
                    h.sysctl_string(k)
                };
                m.insert(k, v.unwrap_or_default());
            }
        } else {
            let cpu = h.read_text("/proc/cpuinfo").unwrap_or_default();
            m.insert(
                "hw.model",
                pairs(&cpu, ':')
                    .get("model name")
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            );
            m.insert(
                "hw.ncpu",
                std::thread::available_parallelism()
                    .map_or(1, |n| n.get())
                    .to_string(),
            );
            let mem = super::stats::MemoryStats::parse_meminfo(
                &h.read_text("/proc/meminfo").unwrap_or_default(),
            )
            .total;
            m.insert(
                "hw.physmem",
                if mem > 0 {
                    mem.to_string()
                } else {
                    String::new()
                },
            );
            let proc = |k: &str| {
                h.read_text(&format!("/proc/sys/kernel/{k}"))
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };
            m.insert("kern.ident", proc("osrelease"));
            m.insert("kern.ostype", proc("ostype"));
            m.insert("kern.osrelease", proc("osrelease"));
        }
        Self::from(&os, &m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_os_release_and_sysctl() {
        let m: HashMap<&str, String> = [
            ("kern.ident", "GENERIC".to_string()),
            ("hw.model", "Intel i7".into()),
            ("hw.ncpu", "8".into()),
            ("hw.physmem", (16i64 << 30).to_string()),
        ]
        .into();
        let i = SystemInfo::from("NAME=FreeBSD\nPRETTY_NAME=\"WolOS 0.10\"\n", &m);
        assert_eq!(
            i,
            SystemInfo {
                name: "WolOS 0.10".into(),
                kernel: "GENERIC".into(),
                base: "Unknown".into(),
                processor: "Intel i7 (8 cores)".into(),
                memory: "16 GB".into()
            }
        );
        let u = SystemInfo::from("", &HashMap::new());
        assert_eq!(u, SystemInfo::unknown());
    }

    #[test]
    fn freebsd_reads_sysctl() {
        let h = super::super::fake::FakeHost::new(true);
        // WolKernel: kern.ident names the product; ostype stays FreeBSD (pkg ABI, ports, Linuxulator).
        h.string("kern.ident", "WolKernel")
            .string("kern.ostype", "FreeBSD")
            .string("kern.osrelease", "15.1-RELEASE-p6")
            .string("hw.model", "Ryzen")
            .int("hw.ncpu", 4)
            .int("hw.physmem", 8 << 30);
        let i = SystemInfo::read(&h);
        assert_eq!(
            (
                i.kernel.as_str(),
                i.base.as_str(),
                i.processor.as_str(),
                i.memory.as_str()
            ),
            (
                "WolKernel",
                "FreeBSD 15.1-RELEASE-p6",
                "Ryzen (4 cores)",
                "8 GB"
            )
        );
    }
}
