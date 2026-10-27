//! HIG §3 Errors: what the user reads when something did not work is a friendly sentence from
//! `shell::copy`, never raw error text (errno, exit codes, paths, "failed", program names).
use shell::copy::{self, ACTION_ERRORS};
use std::path::Path;

fn assert_clean(what: &str, s: &str) {
    let p = copy::lint(s);
    assert!(p.is_empty(), "{what}: “{s}”: {p:?}");
}

/// Raw strings the lint must catch (each was, or could be, on screen).
#[test]
fn banned_patterns_are_caught() {
    for raw in [
        "wolos/theme: Permission denied (os error 13)",
        "pkg install failed (exit 3)",
        "Unavailable on this computer",
        "Administrator password required",
        "Authentication unavailable",
        "wpa_supplicant: wlan0 not found.",
        "thread 'main' panicked at src/ui/store.rs:3:5.",
        "Could not check for updates. Make sure pkg is installed and you are online.",
        "Helpers are turned off (or `wol helpers on`).",
        "Saved to /home/alice/Pictures.",
        "Search failed.",
        "The data source failed (exit 3): no token",
        "No WolOS session is showing (wol-shell is not running).",
        "Updates install into a new boot environment.",
        "Done!",
        "the change was refused (invalid request).",
        "Could not read ~/.config/wolos/shell.json.",
        "Error: Some(\"x\").",
    ] {
        assert!(!copy::lint(raw).is_empty(), "lint let through: “{raw}”");
    }
}

/// No false alarms on ordinary sentences.
#[test]
fn friendly_sentences_pass() {
    for ok in [
        "Incorrect password. Try again.",
        "“Home” didn't accept that password. Check it and try again.",
        "Wi-Fi passwords have 8 to 63 characters. Check it and try again.",
        "Firefox couldn't be installed. Check that you're online, then try again.",
        "Your Downloads folder is tidy.",
        "Restore points keep your last five updates.",
    ] {
        assert_clean("good sentence", ok);
    }
}

#[test]
fn every_copy_sentence_passes() {
    for e in ACTION_ERRORS {
        assert_clean(&format!("action {e:?}"), copy::action(e));
        assert_eq!(
            e.message(),
            copy::action(e),
            "ActionError::message is the copy"
        );
        assert_clean(&format!("wifi {e:?}"), &copy::wifi_join(e, "Home"));
        assert_clean(&format!("install {e:?}"), &copy::install(e, "Firefox"));
        assert_clean(&format!("update {e:?}"), copy::update(e));
    }
    assert_clean("update check", copy::UPDATE_CHECK);
    // Progress, not an outcome: an ellipsis, no full stop (HIG §3).
    assert_eq!(copy::wifi_joining("Home"), "Joining “Home”…");
    for e in copy::undo_errors() {
        assert_clean(&format!("undo {e:?}"), copy::undo(&e));
    }
}

/// Setup Assistant problems under the account fields.
#[test]
fn setup_account_problems_pass() {
    let mut d = shell::setup::Draft::default();
    let mut seen = Vec::new();
    let mut note = |d: &shell::setup::Draft| {
        if let Some((_, s)) = d.account_problem() {
            seen.push(s);
        }
    };
    note(&d);
    d.set_full_name("Alice Liddell");
    d.set_name("Alice!");
    note(&d);
    d.set_name("alice");
    note(&d);
    d.password = "curiouser".into();
    d.verify = "other".into();
    note(&d);
    d.verify = "curiouser".into();
    d.hint = "curiouser".into();
    note(&d);
    d.hint = "a:b".into();
    note(&d);
    d.hint.clear();
    d.set_full_name("Alice: Liddell");
    d.set_name("alice");
    note(&d);
    assert!(seen.len() >= 6, "{seen:?}");
    for s in seen {
        assert_clean("setup", s);
    }
}

/// Surfaces never format an error's own text into a label: `{e}`, `e.to_string()` and the like
/// only appear in log lines (eprintln!/println! within a line of them), and `copy.rs` is the one
/// place that maps errors.
#[test]
fn no_raw_error_text_on_screen() {
    const RAW: [&str; 7] = [
        "{e}",
        "{err}",
        "{e:?}",
        "{err:?}",
        "e.to_string()",
        "err.to_string()",
        "{error}",
    ];
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("copy.rs") {
                files.push(p);
            }
        }
    }
    let mut bad = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        let lines: Vec<&str> = code.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            // `name.to_string()` is not an error: the match starts at a word boundary.
            let raw = RAW.iter().any(|r| {
                l.match_indices(r).any(|(at, _)| {
                    !l[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            });
            if !raw || l.trim_start().starts_with("//") {
                continue;
            }
            let logged = lines[i.saturating_sub(2)..(i + 2).min(lines.len())]
                .iter()
                .any(|x| x.contains("eprintln!") || x.contains("println!"));
            if !logged {
                bad.push(format!(
                    "{}:{}: {}",
                    f.strip_prefix(&src).unwrap().display(),
                    i + 1,
                    l.trim()
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "raw error text outside logs (map it in shell::copy):\n{}",
        bad.join("\n")
    );
}

/// Desktop widgets: what the card says when a widget can't show its data. The developer's detail
/// (exit code, JSON path) is behind "Show Details", never the sentence.
#[test]
fn widget_sentences_pass() {
    use wol_core::widgets::SourceError as E;
    let all = [
        E::NotFound("./friends.sh".into()),
        E::Timeout(5),
        E::Failed {
            code: Some(3),
            stderr: "no token".into(),
        },
        E::TooLarge,
        E::BadJson("line 1, column 2".into()),
        E::NeedsConsent("api.example.com".into()),
        E::Offline("api.example.com".into()),
    ];
    for e in &all {
        assert_clean(
            &format!("widget {e:?}"),
            &copy::widget_source(e, "Game Friends"),
        );
    }
    assert_clean("widget problems", &copy::widget_problems("Stocks"));
    assert_clean("widget missing", copy::WIDGET_MISSING);
    assert_clean("widget newer", copy::WIDGET_NEWER);
    assert_clean("city search", copy::CITY_SEARCH);
    assert_clean("weather offline", copy::WEATHER_OFFLINE);
}

/// `wol games steam` notifications: Steam opened from the Dock or Launchpad has no terminal.
#[test]
fn games_notices_pass() {
    for (title, body) in wol_core::games::notice::ALL {
        assert_clean(&format!("games “{title}”"), body);
    }
}
