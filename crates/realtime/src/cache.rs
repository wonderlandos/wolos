//! When to refresh a window's fallback snapshot (used if a client dies without a clean close).
//! Captures cost a full window render (CPU on Pixman), so take them when content has settled.

/// Content counts as settled after this long without commits.
pub const SETTLE_MS: u64 = 500;
/// Always-changing content (video, games) still gets a snapshot this often.
pub const MAX_STALE_MS: u64 = 2000;

pub fn should_refresh(
    last_capture_ms: Option<u64>,
    last_commit_ms: u64,
    now_ms: u64,
    changed: bool,
) -> bool {
    let Some(last) = last_capture_ms else {
        return true;
    };
    changed
        && (now_ms.saturating_sub(last_commit_ms) >= SETTLE_MS
            || now_ms.saturating_sub(last) >= MAX_STALE_MS)
}

/// How long the frame clock may sleep: full rate while something moves, else a slow idle tick
/// (commits and input wake it immediately through a ping).
pub fn next_frame_ms(animating: bool) -> u64 {
    if animating { 16 } else { 250 }
}

/// Buffer age of the headless render target. It is one persistent texture, so it always holds
/// the previous frame: age 1 redraws only damage. Age 0 ("unknown") would repaint every tick and
/// report a change each time, waking screen-copy clients (wayvnc) at the frame rate.
pub const HEADLESS_TARGET_AGE: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_target_keeps_the_previous_frame() {
        assert_eq!(HEADLESS_TARGET_AGE, 1);
    }

    #[test]
    fn first_capture_always_happens() {
        assert!(should_refresh(None, 0, 0, false));
    }

    #[test]
    fn unchanged_window_is_never_recaptured() {
        assert!(!should_refresh(Some(0), 0, 10_000, false));
    }

    #[test]
    fn busy_window_waits_until_it_settles() {
        assert!(!should_refresh(Some(1000), 1400, 1500, true)); // committed 100 ms ago
        assert!(should_refresh(Some(1000), 1400, 1900, true)); // quiet for 500 ms
    }

    #[test]
    fn always_changing_window_still_refreshes_every_two_seconds() {
        assert!(!should_refresh(Some(1000), 2985, 2990, true)); // 1990 ms since capture, still busy
        assert!(should_refresh(Some(1000), 3000, 3000, true));
    }

    #[test]
    fn frame_clock_idles_slowly_and_animates_at_full_rate() {
        assert_eq!(next_frame_ms(true), 16);
        assert!(next_frame_ms(false) >= 200);
    }
}
