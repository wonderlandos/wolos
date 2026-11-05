// SPDX-License-Identifier: BSD-2-Clause
//! Temperatures: the system API's reader (`system::stats::Temperature::read`: thermal zones,
//! then one entry per hwmon input) on the sampler's root.
use super::Root;
pub use crate::system::stats::Temperature;

/// The temperature sensors under `root`.
pub fn read(root: &Root) -> Vec<Temperature> {
    Temperature::read(root)
        .into_iter()
        // A sensor that is not wired reports 0 or a huge negative; nothing real is outside this.
        .filter(|t| t.celsius > -40.0 && t.celsius < 150.0 && t.celsius != 0.0)
        .collect()
}

/// The hottest sensor, if there is any.
pub fn hottest(t: &[Temperature]) -> Option<&Temperature> {
    t.iter().max_by(|a, b| a.celsius.total_cmp(&b.celsius))
}
