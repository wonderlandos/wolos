// SPDX-License-Identifier: BSD-2-Clause
//! Lower bounds for the benches that have one (`Bench::at_least`, the atlas hit rate); included
//! only by those.
use crate::w3_support::Bench;

impl Bench {
    /// Records a number that must be at least `min`.
    pub fn at_least(&mut self, name: &str, v: f64, min: f64) {
        println!("bench {}.{name} value={v} min={min}", self.group);
        if v < min {
            self.failures
                .push(format!("{}.{name} = {v} is below {min}", self.group));
        }
    }
}
