//! GPU tuning contract (ADR 0007): what the GPU can do (`GpuCaps`) and what it is doing
//! (`GpuSample`). Plain data; backends read text the driver already exposes. Missing counters are
//! `None`, never faked.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FreqRange {
    pub min: u32,
    pub max: u32,
    pub cur: u32,
    pub boost: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuCaps {
    /// Kernel driver (`gpu_select::GpuInfo::driver`): i915, xe, amdgpu, ...
    pub driver: String,
    /// Lossless render compression in the user-space driver (Intel CCS, AMD DCC).
    pub render_compression: bool,
    /// Display FBC active; None = unknown.
    pub display_compression: Option<bool>,
    /// Scanout ARGB8888 format modifiers; empty = not read (needs KMS IN_FORMATS).
    pub modifiers: Vec<u64>,
    pub freq_mhz: Option<FreqRange>,
    /// min/max frequency writable (`GpuBackend::set_freq`).
    pub freq_control: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuSample {
    pub t_ns: u64,
    pub busy_pct: Option<f32>,
    pub freq_mhz: Option<u32>,
    /// Cumulative graphics energy (RAPL PP1), wraps; consumers diff two samples.
    pub gpu_energy_uj: Option<u64>,
    pub mem_bytes: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TuneError {
    Unsupported,
}

pub trait GpuBackend {
    fn caps(&self) -> GpuCaps;
    fn sample(&self) -> GpuSample;
    fn set_freq(&self, min_mhz: u32, max_mhz: u32) -> Result<(), TuneError>;
}

/// "Key: 300 MHz" / "Key: 300MHz" -> 300.
fn mhz(text: &str, key: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let v = l.strip_prefix(key)?.trim().trim_end_matches("MHz").trim();
        v.parse().ok()
    })
}

/// i915 `gt0/frequency` (`intel_gt_pm_frequency_dump`). cur = actual (CAGF), else requested.
pub fn parse_i915_freq(text: &str) -> Option<FreqRange> {
    Some(FreqRange {
        min: mhz(text, "Min freq:")?,
        max: mhz(text, "Max freq:")?,
        cur: mhz(text, "Actual freq:").or_else(|| mhz(text, "Current freq:"))?,
        boost: mhz(text, "Boost freq:"),
    })
}

/// i915 `i915_fbc_status`: "FBC enabled" / "FBC disabled: <reason>".
pub fn parse_i915_fbc(text: &str) -> Option<bool> {
    let first = text.lines().next()?;
    if first.starts_with("FBC enabled") {
        Some(true)
    } else if first.starts_with("FBC disabled") {
        Some(false)
    } else {
        None
    }
}

/// `cpucontrol -m 0x639 /dev/cpuctl0` -> "MSR 0x639: 0x<hi> 0x<lo>".
pub fn parse_cpucontrol(text: &str) -> Option<u64> {
    let mut w = text.split_whitespace().rev();
    let lo = u64::from_str_radix(w.next()?.trim_start_matches("0x"), 16).ok()?;
    let hi = u64::from_str_radix(w.next()?.trim_start_matches("0x"), 16).ok()?;
    Some(hi << 32 | lo)
}

/// RAPL counter (low 32 bits) in µJ; energy unit = 1/2^ESU J, ESU = MSR 0x606 bits 12:8.
pub fn rapl_uj(units_msr: u64, counter: u64) -> u64 {
    let esu = (units_msr >> 8) & 0x1f;
    (((counter & 0xffff_ffff) as u128 * 1_000_000) >> esu) as u64
}

/// i915 (and xe display) backend over lindebugfs + RAPL MSRs. `msr` reads one MSR (cpuctl).
pub struct I915 {
    pub debugfs: PathBuf,
    pub msr: fn(u32) -> Option<u64>,
}

impl I915 {
    fn read(&self, names: &[&str]) -> Option<String> {
        names
            .iter()
            .find_map(|n| std::fs::read_to_string(self.debugfs.join(n)).ok())
    }
    fn freq(&self) -> Option<FreqRange> {
        parse_i915_freq(&self.read(&["gt0/frequency", "gt/frequency", "i915_frequency_info"])?)
    }
}

impl GpuBackend for I915 {
    fn caps(&self) -> GpuCaps {
        GpuCaps {
            driver: "i915".into(),
            render_compression: true, // iris/anv use CCS on Gen12 by default
            display_compression: self
                .read(&["i915_fbc_status"])
                .and_then(|t| parse_i915_fbc(&t)),
            modifiers: Vec::new(),
            freq_mhz: self.freq(),
            freq_control: false, // deferred (ADR 0007)
        }
    }
    fn sample(&self) -> GpuSample {
        let units = (self.msr)(0x606);
        GpuSample {
            t_ns: now_ns(),
            busy_pct: None, // no i915 PMU on FreeBSD
            freq_mhz: self.freq().map(|f| f.cur),
            gpu_energy_uj: units.zip((self.msr)(0x639)).map(|(u, c)| rapl_uj(u, c)),
            mem_bytes: None,
        }
    }
    fn set_freq(&self, _: u32, _: u32) -> Result<(), TuneError> {
        Err(TuneError::Unsupported)
    }
}

/// Any other driver: nothing known.
pub struct Unknown(pub String);

impl GpuBackend for Unknown {
    fn caps(&self) -> GpuCaps {
        GpuCaps {
            driver: self.0.clone(),
            ..GpuCaps::default()
        }
    }
    fn sample(&self) -> GpuSample {
        GpuSample {
            t_ns: now_ns(),
            ..GpuSample::default()
        }
    }
    fn set_freq(&self, _: u32, _: u32) -> Result<(), TuneError> {
        Err(TuneError::Unsupported)
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

/// `cpucontrol -m <msr> /dev/cpuctl0` (needs root + cpuctl).
pub fn cpuctl_msr(msr: u32) -> Option<u64> {
    let o = std::process::Command::new("cpucontrol")
        .args(["-m", &format!("{msr:#x}"), "/dev/cpuctl0"])
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| parse_cpucontrol(&String::from_utf8_lossy(&o.stdout)))
        .flatten()
}

/// /dev/dri/renderD<128+n> -> /sys/kernel/debug/dri/<n> (lindebugfs mounted there).
pub fn debugfs_dir(render_node: &str) -> Option<PathBuf> {
    let n: u32 = render_node.rsplit("renderD").next()?.parse().ok()?;
    Some(PathBuf::from(format!(
        "/sys/kernel/debug/dri/{}",
        n.checked_sub(128)?
    )))
}

/// Backend for a driver; debugfs dir = /sys/kernel/debug/dri/<n>.
pub fn backend(driver: &str, debugfs: PathBuf) -> Box<dyn GpuBackend> {
    match driver {
        "i915" | "xe" => Box::new(I915 {
            debugfs,
            msr: cpuctl_msr,
        }),
        d => Box::new(Unknown(d.into())),
    }
}

impl GpuCaps {
    pub fn to_json(&self) -> String {
        let f = self.freq_mhz.map_or("null".into(), |f| {
            format!(
                r#"{{"min":{},"max":{},"cur":{},"boost":{}}}"#,
                f.min,
                f.max,
                f.cur,
                f.boost.map_or("null".into(), |b| b.to_string())
            )
        });
        format!(
            r#"{{"driver":"{}","render_compression":{},"display_compression":{},"modifiers":{:?},"freq_mhz":{},"freq_control":{}}}"#,
            self.driver,
            self.render_compression,
            self.display_compression
                .map_or("null".into(), |b| b.to_string()),
            self.modifiers,
            f,
            self.freq_control
        )
    }
}

impl GpuSample {
    pub fn to_json(&self) -> String {
        fn o<T: ToString>(v: Option<T>) -> String {
            v.map_or("null".into(), |v| v.to_string())
        }
        format!(
            r#"{{"t_ns":{},"busy_pct":{},"freq_mhz":{},"gpu_energy_uj":{},"mem_bytes":{}}}"#,
            self.t_ns,
            o(self.busy_pct),
            o(self.freq_mhz),
            o(self.gpu_energy_uj),
            o(self.mem_bytes)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i915")
    }

    #[test]
    fn i915_fixture_to_caps_and_sample() {
        fn msr(n: u32) -> Option<u64> {
            match n {
                0x606 => Some(0x000a_0e03),           // ESU 14 -> 61.035 µJ per count
                0x639 => Some(0x0000_0001_0000_4000), // high bits ignored
                _ => None,
            }
        }
        let b = I915 {
            debugfs: fixtures(),
            msr,
        };
        let c = b.caps();
        assert_eq!(
            c.freq_mhz,
            Some(FreqRange {
                min: 100,
                max: 1400,
                cur: 350,
                boost: Some(1400)
            })
        );
        assert_eq!(c.display_compression, Some(true));
        assert!(c.render_compression && !c.freq_control);
        let s = b.sample();
        assert_eq!(s.freq_mhz, Some(350));
        assert_eq!(s.gpu_energy_uj, Some(1_000_000)); // 0x4000 counts * 2^-14 J
        assert_eq!((s.busy_pct, s.mem_bytes), (None, None));
        assert_eq!(b.set_freq(300, 900), Err(TuneError::Unsupported));
    }

    #[test]
    fn parsers_table() {
        let off = std::fs::read_to_string(fixtures().join("i915_fbc_status.off")).unwrap();
        for (text, want) in [
            ("FBC enabled\nCompressing: no\n", Some(true)),
            (off.as_str(), Some(false)),
            ("", None),
            ("garbage", None),
        ] {
            assert_eq!(parse_i915_fbc(text), want, "{text:?}");
        }
        for (text, want) in [
            ("MSR 0x639: 0x00000002 0x0012abcd", Some(0x2_0012_abcd)),
            ("cpucontrol: error", None),
            ("", None),
        ] {
            assert_eq!(parse_cpucontrol(text), want, "{text:?}");
        }
        assert_eq!(parse_i915_freq("Min freq: 100 MHz\n"), None);
        assert_eq!(
            parse_i915_freq("Current freq: 300 MHz\nMin freq: 100 MHz\nMax freq: 900 MHz\n")
                .map(|f| (f.cur, f.boost)),
            Some((300, None))
        );
    }

    #[test]
    fn render_node_to_debugfs() {
        for (node, want) in [
            ("/dev/dri/renderD128", Some("/sys/kernel/debug/dri/0")),
            ("/dev/dri/renderD129", Some("/sys/kernel/debug/dri/1")),
            ("/dev/dri/card0", None),
            ("/dev/dri/renderD12", None),
        ] {
            assert_eq!(debugfs_dir(node), want.map(PathBuf::from), "{node}");
        }
    }

    #[test]
    fn unknown_vendor_is_all_none() {
        let b = backend("nvidia-drm", PathBuf::from("/nonexistent"));
        let c = b.caps();
        assert_eq!(
            (
                c.driver.as_str(),
                c.freq_mhz,
                c.display_compression,
                c.render_compression
            ),
            ("nvidia-drm", None, None, false)
        );
        let s = b.sample();
        assert_eq!(
            (s.busy_pct, s.freq_mhz, s.gpu_energy_uj, s.mem_bytes),
            (None, None, None, None)
        );
        assert!(c.to_json().contains(r#""freq_mhz":null"#));
    }
}
