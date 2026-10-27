//! What the user reads when something did not work: the one place errors become sentences.
//! HIG §3: inline, one or two short sentences, what happened + what to do. Surfaces show these,
//! never an error's own text (`Display`, io errors, exit codes). [`lint`] is the rule;
//! `tests/copy_lint.rs` runs it on every sentence here and on the copy of other modules.
use crate::system::ActionError;
use wol_core::journal::UndoError;

/// Fragments a user must never read (case-insensitive, at a word start), and what to do instead.
pub const BANNED: [(&str, &str); 37] = [
    ("failed", "say what happened and what to do"),
    ("error", "say what happened and what to do"),
    ("errno", "raw OS error"),
    ("os error", "raw io::Error text"),
    ("exit status", "process exit status"),
    ("exit code", "process exit code"),
    ("(exit", "process exit code"),
    ("panicked", "stack text"),
    ("backtrace", "stack text"),
    ("unwrap", "stack text"),
    ("permission denied", "raw EACCES text"),
    ("no such file", "raw ENOENT text"),
    ("invalid", "say what is wrong with it"),
    ("authentication", "say password"),
    ("unavailable", "say what is missing and what to do"),
    ("stderr", "plumbing"),
    ("stdout", "plumbing"),
    ("stdin", "plumbing"),
    ("daemon", "plumbing"),
    ("socket", "plumbing"),
    ("json", "file format"),
    ("wol-", "program name"),
    ("wpa_", "program name"),
    ("wlan", "interface name"),
    ("ssid", "say network name"),
    ("ifconfig", "program name"),
    ("sysctl", "program name"),
    ("hyprctl", "program name"),
    ("hyprland", "say the desktop"),
    ("nmcli", "program name"),
    ("bectl", "program name"),
    ("pkg", "say app or update"),
    ("sudo", "terminal"),
    ("root", "say administrator"),
    ("boot environment", "say restore point"),
    ("some(", "debug text"),
    ("err(", "debug text"),
];

/// Longest sentence pair a sheet or status line shows whole.
pub const MAX_LEN: usize = 140;

/// Everything wrong with `s` as an error sentence (empty = fine).
pub fn lint(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = s.to_lowercase();
    for (pat, why) in BANNED {
        let at_word = lower.match_indices(pat).any(|(i, _)| {
            !lower[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
        });
        if at_word {
            out.push(format!("“{pat}”: {why}"));
        }
    }
    for t in s.split_whitespace() {
        let t = t.trim_start_matches(['“', '(', '"']);
        if t.starts_with(['/', '~', '$', '`', '-']) || t.contains("::") {
            out.push(format!("“{t}”: path, flag or code"));
        }
        if [".conf", ".json", ".log", ".sock", ".rs", ".so"]
            .iter()
            .any(|x| t.trim_end_matches(['.', ',', '”']).ends_with(x))
        {
            out.push(format!("“{t}”: file name"));
        }
    }
    if !s.starts_with(|c: char| c.is_uppercase() || c == '“') {
        out.push("starts with a capital letter".into());
    }
    if !s.ends_with('.') {
        out.push("ends with a full stop".into());
    }
    if s.contains('!') {
        out.push("no exclamation marks".into());
    }
    if s.chars().count() > MAX_LEN {
        out.push(format!("at most {MAX_LEN} characters"));
    }
    if s.matches(". ").count() > 1 {
        out.push("at most two sentences".into());
    }
    out
}

/// Every [`ActionError`], for the lint. [`action`] matches exhaustively, so a new variant needs a
/// sentence before it compiles; list it here too.
pub const ACTION_ERRORS: [ActionError; 7] = [
    ActionError::Unavailable,
    ActionError::NeedsAuth,
    ActionError::Denied,
    ActionError::NotAdmin,
    ActionError::Invalid,
    ActionError::Failed,
    ActionError::WrongPassword,
];

/// Any change that did not happen.
pub fn action(e: ActionError) -> &'static str {
    match e {
        ActionError::Unavailable => "This isn't available on this computer.",
        ActionError::NeedsAuth => "Enter an administrator's password to make this change.",
        ActionError::Denied => "Incorrect password. Try again.",
        ActionError::NotAdmin => "Only an administrator can make this change.",
        ActionError::Invalid | ActionError::Failed => "The change did not work. Try again.",
        ActionError::WrongPassword => {
            "The network didn't accept that password. Check it and try again."
        }
    }
}

/// Joining the Wi-Fi network `name`. Denied = the administrator sheet was cancelled.
pub fn wifi_join(e: ActionError, name: &str) -> String {
    match e {
        ActionError::WrongPassword => {
            format!("“{name}” didn't accept that password. Check it and try again.")
        }
        ActionError::Invalid => {
            "Wi-Fi passwords have 8 to 63 characters. Check it and try again.".into()
        }
        ActionError::Denied | ActionError::NeedsAuth => {
            format!("Not joined. Joining “{name}” needs an administrator's password.")
        }
        ActionError::NotAdmin => format!("Only an administrator can join “{name}”."),
        ActionError::Unavailable => "Wi-Fi isn't available on this computer.".into(),
        ActionError::Failed => format!("Couldn't join “{name}”. Move closer to it and try again."),
    }
}

/// Progress while joining the Wi-Fi network `name` (Control Center's Wi-Fi page).
pub fn wifi_joining(name: &str) -> String {
    format!("Joining “{name}”…")
}

/// Installing the app `name` from the App Store.
pub fn install(e: ActionError, name: &str) -> String {
    match e {
        ActionError::Denied | ActionError::NeedsAuth => {
            format!("{name} wasn't installed. Installing apps needs an administrator's password.")
        }
        ActionError::NotAdmin => {
            format!("Only an administrator can install {name}. Ask one to sign in.")
        }
        ActionError::Unavailable => {
            format!("{name} can't be installed right now. Restart the computer and try again.")
        }
        ActionError::Invalid | ActionError::Failed | ActionError::WrongPassword => {
            format!("{name} couldn't be installed. Check that you're online, then try again.")
        }
    }
}

/// Checking for updates found no answer.
pub const UPDATE_CHECK: &str =
    "Couldn't check for updates. Check that you're online, then try again.";

/// Installing updates.
pub fn update(e: ActionError) -> &'static str {
    match e {
        ActionError::Invalid | ActionError::Failed | ActionError::WrongPassword => {
            "The update didn't finish. Check that you're online, then try again."
        }
        e => action(e),
    }
}

/// Undoing a settings change (Settings > Restore, the Undo button).
pub fn undo(e: &UndoError) -> &'static str {
    match e {
        UndoError::Nothing => "There's nothing to undo.",
        UndoError::NotFound(_) => "That change is no longer in the history.",
        UndoError::Changed(_) => "This was changed again later. Undo the newer change first.",
        UndoError::Refused(_) => "This change can't be undone.",
        UndoError::Io(_) => "The change couldn't be undone. Try again.",
    }
}

/// Every `UndoError` shape, for the lint.
pub fn undo_errors() -> [UndoError; 5] {
    [
        UndoError::Nothing,
        UndoError::NotFound(12),
        UndoError::Changed("wolos/theme".into()),
        UndoError::Refused("helpers cannot change their own permissions"),
        UndoError::Io("Permission denied (os error 13)".into()),
    ]
}

// ---- Desktop widgets (the card's sentence; the developer's detail sits behind Show Details) ----

/// A widget's data source did not give it data.
pub fn widget_source(e: &wol_core::widgets::SourceError, name: &str) -> String {
    use wol_core::widgets::SourceError as E;
    match e {
        E::NotFound(_) => format!("{name} is missing a file it needs. Reinstall it, or remove it."),
        E::Timeout(_) => format!("{name} is taking too long to update. It tries again soon."),
        E::Failed { .. } | E::TooLarge | E::BadJson(_) => {
            format!("{name} couldn't get its data. It tries again soon.")
        }
        E::NeedsConsent(host) => format!("Allow {name} to read data from {host}."),
        E::Offline(_) => format!("{name} can't reach the internet. It updates when you're online."),
    }
}

/// A widget package whose description has mistakes (each listed under Show Details).
pub fn widget_problems(name: &str) -> String {
    format!("{name} can't be shown. Its developer can fix it with the details.")
}

/// A widget on the desktop whose package is gone.
pub const WIDGET_MISSING: &str = "This widget isn't installed. Reinstall it, or remove it.";
/// A widget kind from a newer WolOS.
pub const WIDGET_NEWER: &str = "This widget needs a newer version of WolOS.";
/// The Weather city search got no answer.
pub const CITY_SEARCH: &str =
    "Couldn't search for cities. Check that you're online, then try again.";
/// Weather with nothing cached and no network.
pub const WEATHER_OFFLINE: &str = "No forecast right now. It updates when you're back online.";

// ---- Battery, disk and session power (notifications and the confirm sheet) ----

/// Battery notification at 20, 10 and 5 %: summary, body.
pub fn battery_low(percent: u8) -> (String, String) {
    (
        "Low Battery".into(),
        format!("Your battery has {percent}% left. Connect to power, or turn on Low Power Mode."),
    )
}

/// The battery is about to run out: the computer locks and sleeps.
pub const BATTERY_SLEEP: (&str, &str) = (
    "Battery Empty",
    "Your computer is going to sleep now. Connect to power to keep working.",
);

/// Under 5 % free on the startup disk: summary, body (`free` is "4.2 GB").
pub fn disk_full(free: &str) -> (String, String) {
    (
        "Your Disk Is Almost Full".into(),
        format!("Only {free} left. Empty the Trash or remove files you don't need."),
    )
}

/// The daily update check found something (wol_core::update::check).
pub fn updates_ready(count: usize, security: bool) -> (String, String) {
    let (n, it) = if count == 1 {
        ("1 update".to_string(), "it")
    } else {
        (format!("{count} updates"), "them")
    };
    let fixes = if security {
        ", including security fixes"
    } else {
        ""
    };
    (
        "Updates Available".into(),
        format!("WolOS has {n} ready{fixes}. Open Software Update to install {it}."),
    )
}

/// Restart, Shut Down and Log Out ask first (macOS): question, countdown line, button.
pub fn session_confirm(action: &str, secs: u32) -> Option<(&'static str, String, &'static str)> {
    let (q, what, button) = match action {
        "shutdown" => (
            "Are you sure you want to shut down your computer now?",
            "your computer will shut down",
            "Shut Down",
        ),
        "restart" => (
            "Are you sure you want to restart your computer now?",
            "your computer will restart",
            "Restart",
        ),
        "logout" => (
            "Are you sure you want to log out now?",
            "you will be logged out",
            "Log Out",
        ),
        _ => return None,
    };
    let unit = if secs == 1 { "second" } else { "seconds" };
    Some((
        q,
        format!("If you do nothing, {what} automatically in {secs} {unit}."),
        button,
    ))
}
