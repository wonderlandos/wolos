//! Clipboard history and session actions: pure data rules.

pub const MAX: usize = 25;

/// Newest first, no duplicates, no blank entries, at most `MAX`. Returns true when the list changed.
pub fn push(history: &mut Vec<String>, text: &str) -> bool {
    if text.trim().is_empty() || history.first().is_some_and(|f| f == text) {
        return false;
    }
    history.retain(|h| h != text);
    history.insert(0, text.to_string());
    history.truncate(MAX);
    true
}

/// One line, at most 60 chars.
pub fn preview(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 60 {
        line.chars().take(59).chain(['…']).collect()
    } else {
        line
    }
}

/// Session menu action -> argv. Sleep, restart and shut down go through `wol power` (wol-admind:
/// any account at the computer, no password; sleep locks first); Hyprland exit for log out.
pub fn session_command(action: &str) -> Option<&'static [&'static str]> {
    Some(match action {
        "lock" => &["wol-lock"],
        "sleep" => &["wol", "power", "sleep"],
        "restart" => &["wol", "power", "restart"],
        "shutdown" => &["wol", "power", "shutdown"],
        "logout" => &["hyprctl", "dispatch", "exit"],
        _ => return None,
    })
}

/// Seconds the Restart / Shut Down / Log Out sheet waits before it acts by itself (macOS).
pub const CONFIRM_SECS: u32 = 60;

/// Actions that end every open app: asked first, with a countdown.
pub fn needs_confirm(action: &str) -> bool {
    matches!(action, "restart" | "shutdown" | "logout")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rules() {
        let mut h = Vec::new();
        assert!(push(&mut h, "a"));
        assert!(!push(&mut h, "a"));
        assert!(!push(&mut h, "  "));
        push(&mut h, "b");
        push(&mut h, "a");
        assert_eq!(h, ["a", "b"]);
        for i in 0..40 {
            push(&mut h, &i.to_string());
        }
        assert_eq!(h.len(), MAX);
    }

    #[test]
    fn previews() {
        assert_eq!(preview("a\n  b"), "a b");
        assert_eq!(preview(&"x".repeat(80)).chars().count(), 60);
    }

    #[test]
    fn session_actions() {
        for (a, argv, confirm) in [
            ("lock", &["wol-lock"][..], false),
            ("sleep", &["wol", "power", "sleep"][..], false),
            ("restart", &["wol", "power", "restart"][..], true),
            ("shutdown", &["wol", "power", "shutdown"][..], true),
            ("logout", &["hyprctl", "dispatch", "exit"][..], true),
        ] {
            assert_eq!(session_command(a), Some(argv), "{a}");
            assert_eq!(needs_confirm(a), confirm, "{a}");
            assert_eq!(
                crate::copy::session_confirm(a, CONFIRM_SECS).is_some(),
                confirm
            );
        }
        assert_eq!(session_command("dance"), None);
        let (q, line, button) = crate::copy::session_confirm("shutdown", 60).unwrap();
        assert_eq!(q, "Are you sure you want to shut down your computer now?");
        assert_eq!(
            line,
            "If you do nothing, your computer will shut down automatically in 60 seconds."
        );
        assert_eq!(button, "Shut Down");
        assert!(
            crate::copy::session_confirm("logout", 1)
                .unwrap()
                .1
                .ends_with("in 1 second.")
        );
        for a in ["shutdown", "restart", "logout"] {
            let (q, line, _) = crate::copy::session_confirm(a, 60).unwrap();
            assert!(q.starts_with("Are you sure") && q.ends_with('?'));
            assert!(crate::copy::lint(&line).is_empty(), "{line}");
        }
    }
}
