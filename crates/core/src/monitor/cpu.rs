// SPDX-License-Identifier: BSD-2-Clause
//! CPU load per core from `/proc/stat`: the `cpu` total line and one `cpuN` line per core, the
//! same fields and busy/idle split as [`CpuTimes::parse_proc_stat`].
pub use crate::system::stats::CpuTimes;

/// One `cpu…` line: user nice system idle iowait irq softirq steal (idle = idle + iowait).
/// Counters are unsigned and summed saturating; a corrupt line never panics or wraps.
fn line(l: &str) -> Option<CpuTimes> {
    let (mut total, mut idle) = (0u64, 0u64);
    for (i, f) in l.split_ascii_whitespace().skip(1).take(8).enumerate() {
        let v: u64 = f.parse().ok()?;
        total = total.saturating_add(v);
        if i == 3 || i == 4 {
            idle = idle.saturating_add(v);
        }
    }
    let i = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    Some(CpuTimes {
        busy: i(total - idle),
        total: i(total),
    })
}

/// The `cpu` total line, then `cpu0`, `cpu1`, ... in file order. Empty when unreadable.
pub fn parse(stat: &str) -> Vec<CpuTimes> {
    stat.lines()
        .filter(|l| l.starts_with("cpu"))
        .map_while(line)
        .collect()
}

/// Percent busy per entry of two [`parse`] readings (one decimal). A core that came or went
/// between them (hotplug) makes every entry 0 for that one sample.
pub fn usage(prev: &[CpuTimes], cur: &[CpuTimes]) -> Vec<f64> {
    if prev.len() != cur.len() {
        return vec![0.0; cur.len()];
    }
    prev.iter()
        .zip(cur)
        .map(|(p, c)| CpuTimes::usage_percent(*p, *c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_line_ends_the_list() {
        let v = parse("cpu  1 2 3 4\ncpu0 1 x 3 4\ncpu1 1 1 1 1\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], CpuTimes { busy: 6, total: 10 });
        assert!(parse("").is_empty());
    }
}
