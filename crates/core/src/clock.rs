// SPDX-License-Identifier: BSD-2-Clause
//! Local time-of-day arithmetic shared by the timers that wake at a set hour (the light/dark
//! switch, Focus "until tomorrow"). Pure: the caller reads the local clock.

/// Seconds in a day.
pub const DAY: u32 = 86_400;

/// Seconds from local `h:m:s` to the next of `targets` (seconds after midnight, each below
/// [`DAY`]) that is strictly later; when none is later today, the earliest one tomorrow. An exact
/// hit counts as the next occurrence, so a timer set at the target waits a full cycle. No targets:
/// 0.
pub fn secs_until(h: u8, m: u8, s: u8, targets: &[u32]) -> u32 {
    let now = u32::from(h) * 3600 + u32::from(m) * 60 + u32::from(s);
    let today = targets.iter().copied().filter(|t| *t > now).min();
    let tomorrow = targets.iter().copied().min().map(|t| t + DAY);
    today.or(tomorrow).map_or(0, |t| t - now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_target_today_or_the_first_one_tomorrow() {
        let (seven, nineteen) = (7 * 3600, 19 * 3600);
        assert_eq!(secs_until(6, 59, 30, &[seven, nineteen]), 30);
        assert_eq!(
            secs_until(12, 0, 0, &[nineteen, seven]),
            7 * 3600,
            "order does not matter"
        );
        assert_eq!(secs_until(23, 0, 0, &[seven, nineteen]), 8 * 3600);
        assert_eq!(
            secs_until(7, 0, 0, &[seven]),
            DAY,
            "an exact hit waits a whole day"
        );
        assert_eq!(secs_until(14, 5, 0, &[seven]), (16 * 60 + 55) * 60);
        assert_eq!(secs_until(0, 0, 0, &[]), 0);
    }
}
