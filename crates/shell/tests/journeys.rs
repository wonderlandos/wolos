//! User journeys (archicture/modules/ux-journeys.md) driven through the real flows with fakes:
//! what the person does, what they end up reading. Every sentence a journey shows on a failure
//! must pass `shell::copy::lint`.
use shared::admin::AdminReply;
use shell::copy;
use shell::system::{Action, ActionError, CoreSystem, Sysfs, System};
use std::path::PathBuf;
use std::sync::Arc;
use wol_core::controls::Controls;
use wol_core::system::fake::FakeHost;

fn friendly(what: &str, s: &str) {
    let p = copy::lint(s);
    assert!(p.is_empty(), "{what}: “{s}”: {p:?}");
}

/// FreeBSD machine whose administrator service answers `reply` (None = not running).
fn machine(reply: Option<AdminReply>) -> (Arc<FakeHost>, CoreSystem) {
    let h = Arc::new(FakeHost::new(true));
    let c = Controls {
        host: h.clone(),
        stdin: Arc::new(|_, _| Some(String::new())),
        admin: Arc::new(move |_, _| reply),
        wlan: "wlan0".into(),
    };
    let (_tick, ticks) = std::sync::mpsc::channel();
    let files = Sysfs {
        root: std::env::temp_dir().join("wol-journeys-no-sysfs"),
    };
    (h, CoreSystem::with_ticks(c, files, None, ticks))
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-journey-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// J1: Setup asks at most five questions; the keyboard and the account name answer themselves.
#[test]
fn j1_setup_asks_at_most_five_questions() {
    use shell::setup::{Draft, KEYBOARDS, LANGUAGES, STEPS, Step};
    let questions: Vec<&str> = STEPS
        .iter()
        .filter(|s| !matches!(s, Step::Welcome | Step::Done))
        .map(|s| s.title())
        .collect();
    assert!(questions.len() <= 5, "{questions:?}");
    let mut step = Step::Welcome;
    let mut continues = 0;
    while let Some(n) = step.next() {
        step = n;
        continues += 1;
    }
    assert_eq!((step, continues), (Step::Done, STEPS.len() - 1));

    let mut d = Draft::default();
    let pt = LANGUAGES.iter().position(|l| l.region == "Brasil").unwrap();
    d.set_language(pt);
    assert_eq!(KEYBOARDS[d.keyboard].0, "br", "language picks the keyboard");
    d.set_full_name("Alice Liddell");
    assert_eq!(d.name, "aliceliddell", "account name is filled in");
    d.password = "curiouser".into();
    assert_eq!(
        d.account_problem().map(|p| p.1),
        Some("Passwords don't match.")
    );
    d.verify = "curiouser".into();
    assert_eq!(
        d.account_problem(),
        None,
        "full name + password twice is enough"
    );
}

/// J2: a wrong Wi-Fi password says so in words; it never opens the administrator sheet
/// (NeedsAuth/Denied) and never reports success.
#[test]
fn j2_wrong_wifi_password_says_so() {
    let (h, sys) = machine(Some(AdminReply::Ok));
    h.command("wpa_cli -i wlan0 status", "wpa_state=SCANNING\n")
        .command(
            "wpa_cli -i wlan0 list_networks",
            "network id / ssid / bssid / flags\n0\tHome\tany\t[TEMP-DISABLED]\n",
        );
    let join = |p: &str| sys.perform(&Action::WifiJoin("Home".into(), Some(p.into())));
    let e = join("password1").unwrap_err();
    assert_eq!(e, ActionError::WrongPassword);
    let said = copy::wifi_join(e, "Home");
    assert!(
        said.contains("“Home”") && said.contains("password"),
        "{said}"
    );
    friendly("wrong password", &said);
    friendly("wrong password, generic", e.message());

    let e = join("short").unwrap_err();
    assert_eq!(e, ActionError::Invalid, "too short: refused before it runs");
    assert!(copy::wifi_join(e, "Home").contains("8 to 63"));

    h.command(
        "wpa_cli -i wlan0 status",
        "wpa_state=COMPLETED\nssid=Home\n",
    );
    assert_eq!(join("password1"), Ok(()));
}

/// J3: an app that does not install says what to do; nothing raw reaches the tile.
#[test]
fn j3_install_failures_say_what_to_do() {
    for (reply, want, hint) in [
        (Some(AdminReply::Failed), ActionError::Failed, "online"),
        (None, ActionError::Unavailable, "Restart"),
        (
            Some(AdminReply::NotAdmin),
            ActionError::NotAdmin,
            "administrator",
        ),
    ] {
        let (_, sys) = machine(reply);
        let e = sys
            .perform(&Action::PkgInstall("firefox".into()))
            .unwrap_err();
        assert_eq!(e, want);
        let said = copy::install(e, "Firefox");
        assert!(said.contains("Firefox") && said.contains(hint), "{said}");
        friendly("install", &said);
        friendly("update", copy::update(e));
    }
    let (_, sys) = machine(Some(AdminReply::Ok));
    assert_eq!(
        sys.perform(&Action::PkgInstall("-rf".into())),
        Err(ActionError::Invalid),
        "not an app: refused before it runs"
    );
    assert!(wol_core::store::install_plan("rm -rf /").is_none());
    friendly("update check", copy::UPDATE_CHECK);
}

/// J4: Dark Mode, then Undo: the theme is back, and a second Undo says there is nothing left.
#[test]
fn j4_theme_change_can_be_undone() {
    use wol_core::journal::Journal;
    let d = temp("theme");
    let (state, config) = (d.join("state"), d.join("config"));
    let j = Journal::new(&state, &config);
    let file = shared::theme::ACTIVE_FILE;
    j.save("theme", "settings", &[(file, Some("wol-light\n"))], 1)
        .unwrap();
    let id = j
        .save("theme", "control-center", &[(file, Some("wol-dark\n"))], 2)
        .unwrap();
    assert!(id.is_some(), "the change is journaled");
    let e = j.undo(None, "settings", 3).unwrap();
    assert_eq!(e.what, "theme");
    assert_eq!(
        std::fs::read_to_string(config.join(file)).unwrap(),
        "wol-light\n"
    );
    assert_eq!(
        wol_core::journal::reload_for(&e).theme,
        Some(Some("wol-light".into())),
        "open apps restyle to the restored theme"
    );
    j.undo(None, "settings", 4).unwrap();
    let again = j.undo(None, "settings", 5).unwrap_err();
    assert_eq!(copy::undo(&again), "There's nothing to undo.");
    for e in copy::undo_errors() {
        friendly("undo", copy::undo(&e));
    }
    std::fs::remove_dir_all(d).unwrap();
}

/// J11: the user turns a helper capability off; the helper is refused and cannot undo the
/// refusal itself; the user can.
#[test]
fn j11_revoked_helpers_stay_revoked() {
    use wol_core::automation::{self, Policy, policy};
    use wol_core::journal::Journal;
    let d = temp("helpers");
    let config = d.join("config");
    let j = Journal::new(&d.join("state"), &config);
    let policy_of = || {
        Policy::parse(
            std::fs::read_to_string(config.join(policy::FILE))
                .ok()
                .as_deref(),
        )
    };
    assert!(policy_of().check("appearance").is_ok(), "on by default");
    automation::set_capability(&j, "settings", "appearance", false, 1).unwrap();
    assert!(policy_of().check("appearance").is_err(), "revoked");
    let refused = j.undo(None, "mcp:Claude", 2).unwrap_err();
    friendly("helper undo", copy::undo(&refused));
    assert!(policy_of().check("appearance").is_err(), "still revoked");
    j.undo(None, "settings", 3).unwrap();
    assert!(
        policy_of().check("appearance").is_ok(),
        "the user can undo it"
    );
    std::fs::remove_dir_all(d).unwrap();
}

/// J8: a frozen app ignores Quit; the app menu's Force Quit ends every window of it.
#[test]
fn j8_force_quit_ends_a_frozen_app() {
    use shell::appmenu::{Act, app_menu, batch, parse_verb, verb};
    use shell::hypr::Client;
    let menu = app_menu("Firefox", "firefox");
    let row = menu
        .iter()
        .find(|r| r.2 == Some(Act::ForceQuit))
        .expect("Force Quit in the app menu");
    assert_eq!(row.0, "Force Quit Firefox");
    assert_eq!(
        parse_verb(&verb(Act::ForceQuit).unwrap()),
        Some(Act::ForceQuit)
    );
    let front = Client::new("0xa", "firefox", "Mail");
    let all = [
        front.clone(),
        Client::new("0xb", "firefox", "Docs"),
        Client::new("0xc", "foot", "Terminal"),
    ];
    assert_eq!(
        batch(Act::ForceQuit, &front, &all, (0, 28, 1280, 670)).unwrap(),
        "[[BATCH]]dispatch killwindow address:0xa;dispatch killwindow address:0xb"
    );
}
