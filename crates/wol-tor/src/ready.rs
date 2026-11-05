// SPDX-License-Identifier: GPL-2.0-or-later
//! The "Keep Tor ready" switch (ADR 0053 D4). Default **off**.
//!
//! The cache refresh runs tor on a timer so a Tor tab starts warm. That is a periodic Tor
//! bootstrap from this machine, and unless bridges are configured it connects to the Tor network
//! directly and tells the local network "this machine uses Tor". So nothing is installed, enabled
//! or run until the person turns the switch on (`wol-tor keep-ready on`, Settings), and
//! `cache-run` refuses to run while it is off.
use crate::units::cache_unit_files;
use std::path::Path;

/// The switch's file, relative to `$XDG_CONFIG_HOME`.
pub const FILE: &str = "wol-tor/keep-ready";
/// File name of the cache service unit.
pub const SERVICE_NAME: &str = "wol-tor-cache.service";
/// File name of the cache timer unit.
pub const TIMER_NAME: &str = "wol-tor-cache.timer";

/// Set to `on` by the cache service; `cache-run` refuses to run without it.
pub const UNIT_ENV: &str = "WOL_TOR_KEEP_READY";

/// Reads the switch: on only when a line is exactly `on`; a missing, empty or unreadable file,
/// and anything else, is off.
pub fn parse(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        == Some("on")
}

/// The text of the switch's file.
pub fn render(on: bool) -> String {
    format!(
        "# Keep Tor ready (wol-tor, ADR 0053 D4). `on` fetches Tor's directory documents every few hours;\n{}\n",
        if on { "on" } else { "off" }
    )
}

/// The unit files that must exist for the switch: none when it is off.
pub fn unit_files(on: bool, wol_tor: &Path) -> Vec<(&'static str, String)> {
    if !on {
        return Vec::new();
    }
    let (service, timer) = cache_unit_files(wol_tor);
    vec![(SERVICE_NAME, service), (TIMER_NAME, timer)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_off_unless_it_says_on() {
        assert!(!parse(""));
        assert!(!parse("# on\n"));
        assert!(!parse("yes\n"));
        assert!(!parse("off\non\n"), "the first setting line decides");
        assert!(parse("on\n"));
        assert!(parse(&render(true)));
        assert!(!parse(&render(false)));
    }

    #[test]
    fn off_writes_no_unit_and_on_writes_the_pair() {
        let me = Path::new("/usr/lib/wol-tor");
        assert!(
            unit_files(parse(""), me).is_empty(),
            "default: nothing installed"
        );
        assert!(unit_files(false, me).is_empty());
        let on = unit_files(true, me);
        assert_eq!(
            on.iter().map(|u| u.0).collect::<Vec<_>>(),
            [SERVICE_NAME, TIMER_NAME]
        );
        assert!(on[0].1.contains("cache-run"));
    }
}
