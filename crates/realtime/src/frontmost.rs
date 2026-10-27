//! Frontmost window for the shell's menu bar. Written to `$XDG_RUNTIME_DIR/wol-active` on change,
//! one line in `wlrctl toplevel list state:active` format (`<app_id>: <title>`), empty when no
//! window is active. The shell stats the file instead of forking wlrctl every second.

pub const FILE: &str = "wol-active";

/// State-file line for the active window, or "" for none.
pub fn line(active: Option<(&str, &str)>) -> String {
    match active {
        Some(("", title)) => format!("<unknown>: {title}"), // as wlrctl prints it
        Some((app_id, title)) => format!("{app_id}: {title}"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_wlrctl_active_line() {
        for (input, want) in [
            (Some(("wol-files", "Home")), "wol-files: Home"),
            (Some(("", "xterm")), "<unknown>: xterm"),
            (None, ""),
        ] {
            assert_eq!(line(input), want);
        }
    }
}
