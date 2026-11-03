// SPDX-License-Identifier: BSD-2-Clause
//! Timing for the benches that measure call times (`Bench::time`); included by those that do, so a
//! bench that only records values compiles no unused code.
use crate::w3_support::Bench;
use std::time::Instant;

impl Bench {
    /// Times `f` over `samples` batches of `iters` calls; reports ns per call (p50, p95).
    pub fn time(
        &mut self,
        name: &str,
        samples: usize,
        iters: usize,
        mut f: impl FnMut(),
    ) -> (f64, f64) {
        // Warm up.
        for _ in 0..iters.min(64) {
            f();
        }
        let mut v = Vec::with_capacity(samples);
        for _ in 0..samples {
            let t = Instant::now();
            for _ in 0..iters {
                f();
            }
            v.push(t.elapsed().as_nanos() as f64 / iters as f64);
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = v[v.len() / 2];
        let p95 = v[(v.len() * 95 / 100).min(v.len() - 1)];
        println!(
            "bench {}.{name} p50={p50:.0} p95={p95:.0} n={samples}x{iters}",
            self.group
        );
        self.limit(name, p50);
        (p50, p95)
    }
}
