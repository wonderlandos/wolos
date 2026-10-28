//! "About this computer": /etc/os-release, /proc/cpuinfo, /proc/meminfo, /proc/sys/kernel.
use super::{Host, pairs};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SystemInfo {
    pub name: String,
    /// Kernel product: WolKernel (`linux-wolos`, release ends in `-wolos`) or Linux on a stock kernel.
    pub kernel: String,
    /// `ostype osrelease` (Linux 6.17.1-wolos): what the kernel reports.
    pub base: String,
    pub processor: String,
    pub memory: String,
}

/// Raw Linux facts behind [`SystemInfo`]; empty strings / 0 = unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts<'a> {
    pub os_release: &'a str,
    pub cpuinfo: &'a str,
    pub meminfo: &'a str,
    pub ostype: &'a str,
    pub osrelease: &'a str,
    /// 0 = count `processor` lines in cpuinfo.
    pub ncpu: usize,
}

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

    pub fn from(f: &Facts) -> Self {
        let os: std::collections::HashMap<&str, &str> = f
            .os_release
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k, v.trim_matches('"')))
            .collect();
        let known = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
        let mem = super::stats::MemoryStats::parse_meminfo(f.meminfo).total;
        let model = pairs(f.cpuinfo, ':')
            .get("model name")
            .and_then(|m| known(m));
        let ncpu = match f.ncpu {
            0 => f
                .cpuinfo
                .lines()
                .filter(|l| l.starts_with("processor"))
                .count(),
            n => n,
        };
        let (t, r) = (known(f.ostype), known(f.osrelease));
        Self {
            name: os
                .get("PRETTY_NAME")
                .or_else(|| os.get("NAME"))
                .map_or("WolOS", |v| v)
                .into(),
            kernel: match &r {
                Some(r) if r.ends_with("-wolos") => "WolKernel".into(),
                Some(_) => t.clone().unwrap_or_else(|| "Linux".into()),
                None => "Unknown".into(),
            },
            base: match (&t, &r) {
                (Some(t), Some(r)) => format!("{t} {r}"),
                _ => "Unknown".into(),
            },
            processor: model.map_or_else(|| "Unknown".into(), |m| format!("{m} ({ncpu} cores)")),
            memory: if mem > 0 {
                format!("{} GB", (mem + (1 << 29)) >> 30)
            } else {
                "Unknown".into()
            },
        }
    }

    pub fn read(h: &dyn Host) -> Self {
        let r = |p: &str| h.read_text(p).unwrap_or_default();
        let os = h
            .read_text("/etc/os-release")
            .or_else(|| h.read_text("/usr/lib/os-release"))
            .unwrap_or_default();
        let (cpuinfo, meminfo) = (r("/proc/cpuinfo"), r("/proc/meminfo"));
        let (ostype, osrelease) = (
            r("/proc/sys/kernel/ostype"),
            r("/proc/sys/kernel/osrelease"),
        );
        Self::from(&Facts {
            os_release: &os,
            cpuinfo: &cpuinfo,
            meminfo: &meminfo,
            ostype: &ostype,
            osrelease: &osrelease,
            ncpu: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: 12th Gen Intel(R) Core(TM) i7-1255U\n\nprocessor\t: 1\nmodel name\t: 12th Gen Intel(R) Core(TM) i7-1255U\n";

    #[test]
    fn from_linux_facts() {
        let i = SystemInfo::from(&Facts {
            os_release: "NAME=\"WolOS\"\nPRETTY_NAME=\"WolOS 0.16\"\nID_LIKE=arch\n",
            cpuinfo: CPUINFO,
            meminfo: "MemTotal:       16303412 kB\nMemAvailable: 1 kB\n",
            ostype: "Linux\n",
            osrelease: "6.17.1-wolos\n",
            ncpu: 0,
        });
        assert_eq!(
            i,
            SystemInfo {
                name: "WolOS 0.16".into(),
                kernel: "WolKernel".into(),
                base: "Linux 6.17.1-wolos".into(),
                processor: "12th Gen Intel(R) Core(TM) i7-1255U (2 cores)".into(),
                memory: "16 GB".into()
            }
        );
        assert_eq!(SystemInfo::from(&Facts::default()), SystemInfo::unknown());
        let stock = SystemInfo::from(&Facts {
            ostype: "Linux",
            osrelease: "6.17.1-arch1-1",
            ..Default::default()
        });
        assert_eq!(stock.kernel, "Linux");
    }

    #[test]
    fn reads_proc_through_host() {
        let h = super::super::fake::FakeHost::linux();
        h.file("/usr/lib/os-release", "PRETTY_NAME=\"Arch Linux\"\n")
            .file("/proc/cpuinfo", CPUINFO)
            .file("/proc/meminfo", "MemTotal: 8388608 kB\n")
            .file("/proc/sys/kernel/ostype", "Linux\n")
            .file("/proc/sys/kernel/osrelease", "6.17.1-wolos\n");
        let i = SystemInfo::read(&h);
        assert_eq!(
            (i.name.as_str(), i.kernel.as_str(), i.memory.as_str()),
            ("Arch Linux", "WolKernel", "8 GB")
        );
    }

    #[test]
    fn real_host_reports_linux() {
        let i = SystemInfo::read(&super::super::PosixHost);
        assert!(i.base.starts_with("Linux "), "{i:?}");
        assert_ne!(i.memory, "Unknown");
        assert_ne!(i.processor, "Unknown");
    }
}
