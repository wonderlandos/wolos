//! Intel RAPL package power limits (MSR 0x610 PKG_POWER_LIMIT, units in MSR 0x606).
//! Pure decode/encode; I/O is `gpu_tune::cpuctl_msr` + `cpucontrol -m 0x610=<v>` in the binary.
//! The effective limit is the lower of this MSR and the MCHBAR mirror (firmware-owned, not touched).

pub const MSR_RAPL_POWER_UNIT: u32 = 0x606;
pub const MSR_PKG_POWER_LIMIT: u32 = 0x610;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub watts: f64,
    pub enabled: bool,
    pub clamp: bool,
    /// Time window in seconds (PL1 tau; PL2 short window).
    pub window_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PkgLimits {
    pub pl1: Limit,
    pub pl2: Limit,
    /// Bit 63: firmware locked the MSR until reset; writes are ignored.
    pub locked: bool,
}

fn power_unit(units: u64) -> f64 {
    1.0 / f64::from(1u32 << (units & 0xf))
}

fn time_unit(units: u64) -> f64 {
    1.0 / f64::from(1u32 << ((units >> 16) & 0xf))
}

fn limit(bits: u64, units: u64) -> Limit {
    let y = (bits >> 17) & 0x1f;
    let z = (bits >> 22) & 0x3;
    Limit {
        watts: (bits & 0x7fff) as f64 * power_unit(units),
        enabled: bits & (1 << 15) != 0,
        clamp: bits & (1 << 16) != 0,
        window_s: f64::from(1u32 << y) * (1.0 + z as f64 / 4.0) * time_unit(units),
    }
}

pub fn decode(units: u64, msr: u64) -> PkgLimits {
    PkgLimits {
        pl1: limit(msr & 0xffff_ffff, units),
        pl2: limit(msr >> 32, units),
        locked: msr >> 63 != 0,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PowerError {
    Locked,
    /// Outside 1 W ..= the 15-bit field.
    OutOfRange,
}

/// New MSR value with PL1/PL2 watts replaced (and enabled); windows, clamp and lock untouched.
pub fn with_watts(
    units: u64,
    msr: u64,
    pl1: Option<f64>,
    pl2: Option<f64>,
) -> Result<u64, PowerError> {
    if msr >> 63 != 0 {
        return Err(PowerError::Locked);
    }
    let field = |w: f64| -> Result<u64, PowerError> {
        let v = (w / power_unit(units)).round();
        if !(1.0..=f64::from(0x7fffu32)).contains(&v) || w < 1.0 {
            return Err(PowerError::OutOfRange);
        }
        Ok(v as u64)
    };
    let mut out = msr;
    if let Some(w) = pl1 {
        out = (out & !0x7fff) | field(w)? | 1 << 15;
    }
    if let Some(w) = pl2 {
        out = (out & !(0x7fff << 32)) | field(w)? << 32 | 1 << 47;
    }
    Ok(out)
}

/// `set [--pl1 W] [--pl2 W]` arguments; None = usage error (nothing to set or bad number).
pub fn parse_set(args: &[&str]) -> Option<(Option<f64>, Option<f64>)> {
    let (mut pl1, mut pl2) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v: f64 = it.next()?.parse().ok()?;
        match *a {
            "--pl1" => pl1 = Some(v),
            "--pl2" => pl2 = Some(v),
            _ => return None,
        }
    }
    (pl1.is_some() || pl2.is_some()).then_some((pl1, pl2))
}

impl PkgLimits {
    pub fn to_json(&self) -> String {
        let l = |l: &Limit| {
            format!(
                r#"{{"watts":{:.3},"enabled":{},"clamp":{},"window_s":{:.4}}}"#,
                l.watts, l.enabled, l.clamp, l.window_s
            )
        };
        format!(
            r#"{{"pl1":{},"pl2":{},"locked":{}}}"#,
            l(&self.pl1),
            l(&self.pl2),
            self.locked
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ADL-P typical: units 0x000a0e03 (1/8 W, 2^-14 J, 2^-10 s).
    const UNITS: u64 = 0x000a_0e03;
    // PL1 28 W enabled + clamp, Y=10 Z=0 -> 1 s; PL2 64 W enabled, Y=1 Z=1 -> 2.4 ms.
    const MSR: u64 = (0x0042_8200 | 1 << 15) << 32 | 0x0014_80e0 | 1 << 15 | 1 << 16;

    #[test]
    fn decode_table() {
        let d = decode(UNITS, MSR);
        assert_eq!(d.pl1.watts, 0xe0 as f64 / 8.0); // 28 W
        assert!(d.pl1.enabled && d.pl1.clamp && !d.locked);
        assert_eq!(d.pl1.window_s, 1024.0 / 1024.0); // Y=10, Z=0
        assert_eq!(d.pl2.watts, 0x200 as f64 / 8.0); // 64 W
        assert!(d.pl2.enabled);
        assert_eq!(d.pl2.window_s, 2.0 * 1.25 / 1024.0); // Y=1, Z=1
        assert!(decode(UNITS, MSR | 1 << 63).locked);
    }

    #[test]
    fn set_watts_round_trips_and_keeps_other_bits() {
        let v = with_watts(UNITS, MSR, Some(15.0), None).unwrap();
        let d = decode(UNITS, v);
        assert_eq!((d.pl1.watts, d.pl2.watts), (15.0, 64.0));
        assert_eq!(
            v & !0x7fff,
            MSR & !0x7fff,
            "only the PL1 power field changed"
        );
        let v = with_watts(UNITS, MSR, None, Some(40.0)).unwrap();
        assert_eq!(decode(UNITS, v).pl2.watts, 40.0);
        assert_eq!(v & 0xffff_ffff, MSR & 0xffff_ffff);
    }

    #[test]
    fn set_args_table() {
        for (args, want) in [
            (&["--pl1", "15"][..], Some((Some(15.0), None))),
            (
                &["--pl2", "40", "--pl1", "20.5"][..],
                Some((Some(20.5), Some(40.0))),
            ),
            (&[][..], None),
            (&["--pl1"][..], None),
            (&["--pl3", "1"][..], None),
            (&["--pl1", "x"][..], None),
        ] {
            assert_eq!(parse_set(args), want, "{args:?}");
        }
    }

    #[test]
    fn refuses_locked_and_out_of_range() {
        for (msr, pl1, want) in [
            (MSR | 1 << 63, Some(15.0), PowerError::Locked),
            (MSR, Some(0.0), PowerError::OutOfRange),
            (MSR, Some(0.5), PowerError::OutOfRange),
            (MSR, Some(5000.0), PowerError::OutOfRange),
        ] {
            assert_eq!(with_watts(UNITS, msr, pl1, None), Err(want), "{pl1:?}");
        }
    }
}
