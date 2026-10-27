//! Setup Assistant model (ADR 0004, ADR 0013): steps, choices, account draft and the flow that
//! drives them. Pure; the UI (`ui::setup`) renders a [`Flow`] and runs its [`Job`]s.
use crate::login::Verdict;
use shared::login::{AVATAR_TONES, NewAccount};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Language,
    Wifi,
    Account,
    Appearance,
    Done,
}

/// Welcome, four questions, Done.
pub const STEPS: [Step; 6] = [
    Step::Welcome,
    Step::Language,
    Step::Wifi,
    Step::Account,
    Step::Appearance,
    Step::Done,
];

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::Language => "Language & Region",
            Step::Wifi => "Wi-Fi",
            Step::Account => "Create Your Account",
            Step::Appearance => "Appearance",
            Step::Done => "You're All Set",
        }
    }

    /// One sentence under the title: what this page is for.
    pub fn subtitle(self) -> &'static str {
        match self {
            Step::Welcome => "Let's get you set up.",
            Step::Language => "WolOS uses this language and keyboard. You can change them later.",
            Step::Wifi => "Choose a network to get online. You can skip this and join later.",
            Step::Account => "You'll use this name and password to sign in.",
            Step::Appearance => "Choose a look. You can change it any time in System Settings.",
            Step::Done => "Your account is ready.",
        }
    }

    /// Stack page name.
    pub fn id(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Language => "language",
            Step::Wifi => "wifi",
            Step::Account => "account",
            Step::Appearance => "appearance",
            Step::Done => "done",
        }
    }

    fn index(self) -> usize {
        STEPS.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Option<Step> {
        STEPS.get(self.index() + 1).copied()
    }

    pub fn prev(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| STEPS[i])
    }
}

pub struct Language {
    pub name: &'static str,
    pub region: &'static str,
    pub locale: &'static str,
    pub keymap: &'static str,
}

const fn lang(
    name: &'static str,
    region: &'static str,
    locale: &'static str,
    keymap: &'static str,
) -> Language {
    Language {
        name,
        region,
        locale,
        keymap,
    }
}

pub const LANGUAGES: [Language; 7] = [
    lang("English", "United States", "en_US.UTF-8", "us"),
    lang("English", "United Kingdom", "en_GB.UTF-8", "gb"),
    lang("Português", "Brasil", "pt_BR.UTF-8", "br"),
    lang("Português", "Portugal", "pt_PT.UTF-8", "pt"),
    lang("Español", "España", "es_ES.UTF-8", "es"),
    lang("Français", "France", "fr_FR.UTF-8", "fr"),
    lang("Deutsch", "Deutschland", "de_DE.UTF-8", "de"),
];

/// (xkb layout, name): the same list Settings > Keyboard offers.
pub const KEYBOARDS: [(&str, &str); 8] = wol_core::prefs::LAYOUTS;

/// Applies xkb layout `k` (`us(intl)` = layout `us`, variant `intl`) to the running compositor, so
/// the password typed in Setup is typed on the keyboard the session will use.
pub fn keyboard_command(k: &str) -> Vec<String> {
    let (layout, variant) = match k.split_once('(') {
        Some((l, v)) => (l, v.trim_end_matches(')')),
        None => (k, ""),
    };
    vec![
        "hyprctl".into(),
        "--batch".into(),
        format!("keyword input:kb_layout {layout} ; keyword input:kb_variant {variant}"),
    ]
}

/// Time zone prefix -> locale, for machines that know where they are but not the language.
const ZONES: [(&str, &str); 14] = [
    ("America/Sao_Paulo", "pt_BR.UTF-8"),
    ("America/Fortaleza", "pt_BR.UTF-8"),
    ("America/Recife", "pt_BR.UTF-8"),
    ("America/Manaus", "pt_BR.UTF-8"),
    ("America/Bahia", "pt_BR.UTF-8"),
    ("Brazil/", "pt_BR.UTF-8"),
    ("Europe/Lisbon", "pt_PT.UTF-8"),
    ("Europe/London", "en_GB.UTF-8"),
    ("Europe/Dublin", "en_GB.UTF-8"),
    ("Europe/Madrid", "es_ES.UTF-8"),
    ("Europe/Paris", "fr_FR.UTF-8"),
    ("Europe/Berlin", "de_DE.UTF-8"),
    ("Europe/Vienna", "de_DE.UTF-8"),
    ("Europe/Zurich", "de_DE.UTF-8"),
];

/// FreeBSD console keymap (`keymap=` in rc.conf, `br.kbd`) -> xkb layout.
fn xkb_of_console(keymap: &str) -> Option<&'static str> {
    let base = keymap.trim().trim_matches('"').trim_end_matches(".kbd");
    let base = base.split('.').next().unwrap_or(base);
    Some(match base {
        "us" => "us",
        "us.intl" | "intl" => "us(intl)",
        "br" | "br275" => "br",
        "pt" => "pt",
        "uk" | "gb" => "gb",
        "de" => "de",
        "fr" => "fr",
        "es" => "es",
        _ => return None,
    })
}

/// What the machine already knows: `LANG`, the time zone (`/var/db/zoneinfo`) and the console
/// keymap (`keymap=` in `/etc/rc.conf`). Any of them may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    pub lang: Option<String>,
    pub zone: Option<String>,
    pub keymap: Option<String>,
}

impl Hints {
    /// Reads the hints on this machine (`root` = `/`).
    pub fn read(root: &std::path::Path) -> Hints {
        let file = |p: &str| std::fs::read_to_string(root.join(p)).ok();
        let keymap = file("etc/rc.conf").and_then(|rc| {
            rc.lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("keymap="))
                .map(|v| v.trim_matches('"').to_string())
        });
        Hints {
            lang: std::env::var("LANG").ok(),
            zone: file("var/db/zoneinfo").map(|z| z.trim().to_string()),
            keymap,
        }
    }

    /// Language index: `LANG` when it names one of ours, else the time zone, else English (US).
    pub fn language(&self) -> usize {
        let by_locale = |loc: &str| LANGUAGES.iter().position(|l| l.locale == loc);
        let from_lang = self.lang.as_deref().and_then(|l| {
            let l = l.split('.').next().unwrap_or(l);
            by_locale(&format!("{l}.UTF-8"))
        });
        let from_zone = self.zone.as_deref().and_then(|z| {
            ZONES
                .iter()
                .find(|(p, _)| z.starts_with(p))
                .and_then(|(_, loc)| by_locale(loc))
        });
        from_lang.or(from_zone).unwrap_or(0)
    }

    /// Keyboard index: the console keymap when set, else the language's.
    pub fn keyboard(&self, language: usize) -> usize {
        let xkb = self
            .keymap
            .as_deref()
            .and_then(xkb_of_console)
            .unwrap_or(LANGUAGES[language.min(LANGUAGES.len() - 1)].keymap);
        KEYBOARDS.iter().position(|k| k.0 == xkb).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Appearance {
    Light,
    Dark,
    #[default]
    Auto,
}

pub const APPEARANCES: [Appearance; 3] = [Appearance::Light, Appearance::Dark, Appearance::Auto];

impl Appearance {
    pub fn wire(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Auto => "auto",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
            Appearance::Auto => "Auto",
        }
    }
}

/// The three looks Setup offers: (accent id, card title). More in System Settings.
pub const THEMES: [(&str, &str); 3] = [
    ("violet", "Wonderland"),
    ("blue", "Ocean"),
    ("graphite", "Graphite"),
];

/// Account name macOS-style: lowercase ASCII letters and digits of the full name, accents folded,
/// starting with a letter, at most 32.
pub fn account_name(full: &str) -> String {
    let fold = |c: char| -> Option<char> {
        let c = c.to_lowercase().next()?;
        let base = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        };
        (base.is_ascii_lowercase() || base.is_ascii_digit()).then_some(base)
    };
    full.chars()
        .filter_map(fold)
        .skip_while(|c| c.is_ascii_digit())
        .take(32)
        .collect()
}

/// Account card fields (problems are shown under the one they concern).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    FullName,
    Name,
    Password,
    Verify,
    Hint,
}

/// Avatar text: initials, "?" before any input.
pub fn initials_or_placeholder(full: &str) -> String {
    let i = crate::login::initials(full);
    if i.is_empty() { "?".into() } else { i }
}

/// Everything the assistant collects.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub language: usize,
    pub keyboard: usize,
    pub full_name: String,
    pub name: String,
    name_edited: bool,
    pub password: String,
    pub verify: String,
    pub hint: String,
    /// Index into [`AVATAR_TONES`].
    pub avatar: usize,
    pub appearance: Appearance,
    /// Index into [`THEMES`].
    pub theme: usize,
}

impl Draft {
    /// Language and keyboard as the machine suggests them.
    pub fn detected(h: &Hints) -> Draft {
        let language = h.language();
        Draft {
            language,
            keyboard: h.keyboard(language),
            ..Draft::default()
        }
    }

    /// Picking a language also picks its keyboard.
    pub fn set_language(&mut self, i: usize) {
        self.language = i.min(LANGUAGES.len() - 1);
        let km = LANGUAGES[self.language].keymap;
        self.keyboard = KEYBOARDS.iter().position(|k| k.0 == km).unwrap_or(0);
    }

    /// Derives the account name until the user edits it.
    pub fn set_full_name(&mut self, full: &str) {
        self.full_name = full.to_string();
        if !self.name_edited {
            self.name = account_name(full);
        }
    }

    pub fn set_name(&mut self, name: &str) {
        self.name_edited = name != account_name(&self.full_name);
        self.name = name.to_string();
    }

    /// First thing to fix on the account card: the field and one sentence; None to continue.
    pub fn account_problem(&self) -> Option<(Field, &'static str)> {
        if self.full_name.trim().is_empty() {
            return Some((Field::FullName, "Enter your full name."));
        }
        let rules = wol_core::settings::users::validate_account(&self.account());
        if rules == Err("name") || self.name.is_empty() {
            return Some((
                Field::Name,
                "Use lowercase letters and digits, starting with a letter.",
            ));
        }
        if self.password.is_empty() {
            return Some((Field::Password, "Choose a password."));
        }
        if self.password != self.verify {
            return Some((Field::Verify, "Passwords don't match."));
        }
        if !self.hint.is_empty() && self.hint == self.password {
            return Some((Field::Hint, "The hint can't be the password."));
        }
        match rules {
            Ok(()) => None,
            Err("hint") => Some((Field::Hint, "Keep the hint short, without colons.")),
            Err("full name") => Some((Field::FullName, "Remove colons from the name.")),
            Err(_) => Some((Field::Password, "Choose a different password.")),
        }
    }

    pub fn tone(&self) -> &'static str {
        AVATAR_TONES[self.avatar.min(AVATAR_TONES.len() - 1)]
    }

    pub fn accent(&self) -> &'static str {
        THEMES[self.theme.min(THEMES.len() - 1)].0
    }

    pub fn account(&self) -> NewAccount {
        NewAccount {
            name: self.name.clone(),
            full_name: self.full_name.trim().to_string(),
            hint: self.hint.clone(),
            locale: LANGUAGES[self.language.min(LANGUAGES.len() - 1)]
                .locale
                .into(),
            keymap: KEYBOARDS[self.keyboard.min(KEYBOARDS.len() - 1)].0.into(),
            appearance: self.appearance.wire().into(),
            avatar: self.tone().into(),
            accent: self.accent().into(),
            password: self.password.clone(),
        }
    }
}

/// The network picked on the Wi-Fi page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiPick {
    pub ssid: String,
    pub secure: bool,
    pub connected: bool,
    pub password: String,
}

/// Work the UI runs off the main thread, then reports with [`Flow::job_done`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    JoinWifi { ssid: String, password: String },
    CreateAccount(NewAccount),
}

/// What one Continue (Return) does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Show this page.
    Go(Step),
    /// Stay; say this under `Field` (None = at the foot of the page).
    Stay(Option<Field>, String),
    /// Run this, then call [`Flow::job_done`].
    Run(Job),
    /// Get Started: fade into the desktop.
    Finish,
}

/// Does the [`Job`]s (wol-login's socket on the machine, a fake in tests).
pub trait Backend {
    fn run(&self, job: &Job) -> Verdict;
}

/// Setup state machine: the page, the draft, the Wi-Fi pick. Wired machines skip Wi-Fi; once the
/// account exists there is no way back.
#[derive(Clone, Debug)]
pub struct Flow {
    pub step: Step,
    pub draft: Draft,
    pub wifi: Option<WifiPick>,
    wired: bool,
    created: bool,
}

impl Flow {
    pub fn new(draft: Draft, wired: bool) -> Flow {
        Flow {
            step: Step::Welcome,
            draft,
            wifi: None,
            wired,
            created: false,
        }
    }

    fn after(&self, s: Step) -> Option<Step> {
        match s.next() {
            Some(Step::Wifi) if self.wired => Step::Wifi.next(),
            n => n,
        }
    }

    fn before(&self, s: Step) -> Option<Step> {
        match s.prev() {
            Some(Step::Wifi) if self.wired => Step::Wifi.prev(),
            p => p,
        }
    }

    fn go(&mut self, s: Step) -> Next {
        self.step = s;
        Next::Go(s)
    }

    /// Continue / Return on the current page.
    pub fn advance(&mut self) -> Next {
        match self.step {
            Step::Wifi => match &self.wifi {
                Some(w) if !w.connected => {
                    if w.secure && w.password.is_empty() {
                        Next::Stay(None, format!("Enter the password for “{}”.", w.ssid))
                    } else {
                        Next::Run(Job::JoinWifi {
                            ssid: w.ssid.clone(),
                            password: w.password.clone(),
                        })
                    }
                }
                _ => self.skip_wifi(),
            },
            Step::Account => match self.draft.account_problem() {
                Some((f, why)) => Next::Stay(Some(f), why.into()),
                None => self.go(Step::Appearance),
            },
            Step::Appearance => Next::Run(Job::CreateAccount(self.draft.account())),
            Step::Done => Next::Finish,
            s => match self.after(s) {
                Some(n) => self.go(n),
                None => Next::Finish,
            },
        }
    }

    /// "Not Now" on the Wi-Fi page.
    pub fn skip_wifi(&mut self) -> Next {
        self.go(Step::Account)
    }

    /// Escape / Back: None where there is no way back (Welcome, Done).
    pub fn back(&mut self) -> Option<Step> {
        if self.created || self.step == Step::Done {
            return None;
        }
        let p = self.before(self.step)?;
        self.step = p;
        Some(p)
    }

    /// The result of a [`Job`] from [`Flow::advance`].
    pub fn job_done(&mut self, job: &Job, v: Verdict) -> Next {
        match (job, v) {
            (Job::JoinWifi { .. }, Verdict::Ok) => {
                if let Some(w) = &mut self.wifi {
                    w.connected = true;
                }
                self.go(Step::Account)
            }
            (Job::JoinWifi { ssid, .. }, Verdict::Denied) => Next::Stay(
                None,
                format!("“{ssid}” didn't accept that password. Check it and try again."),
            ),
            (Job::JoinWifi { ssid, .. }, Verdict::Error) => Next::Stay(
                None,
                format!("“{ssid}” didn't answer. Try again, or choose Not Now and join later."),
            ),
            (Job::CreateAccount(_), Verdict::Ok) => {
                self.created = true;
                self.go(Step::Done)
            }
            (Job::CreateAccount(_), Verdict::Denied) => Next::Stay(
                None,
                "That account name can't be used. Press Back and choose another one.".into(),
            ),
            (Job::CreateAccount(_), Verdict::Error) => Next::Stay(
                None,
                "Your account couldn't be created just now. Press Continue to try again.".into(),
            ),
        }
    }

    /// Continue, running any job right away (tests; the UI runs jobs off the main thread).
    pub fn drive(&mut self, b: &dyn Backend) -> Next {
        match self.advance() {
            Next::Run(job) => {
                let v = b.run(&job);
                self.job_done(&job, v)
            }
            n => n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn account_name_from_full_name() {
        for (full, want) in [
            ("Alice Liddell", "aliceliddell"),
            ("  José  Álvarez-Núñez ", "josealvareznunez"),
            ("Ünïcödé", "unicode"),
            ("42 Wonderland", "wonderland"),
            ("", ""),
            ("李", ""),
        ] {
            assert_eq!(account_name(full), want, "{full}");
        }
        assert_eq!(account_name(&"a".repeat(40)).len(), 32);
    }

    #[test]
    fn steps_in_order() {
        assert_eq!(
            STEPS.map(Step::title),
            [
                "Welcome",
                "Language & Region",
                "Wi-Fi",
                "Create Your Account",
                "Appearance",
                "You're All Set",
            ]
        );
        assert_eq!(Step::Welcome.next(), Some(Step::Language));
        assert_eq!(Step::Done.next(), None);
        assert_eq!(Step::Welcome.prev(), None);
        assert_eq!(Step::Wifi.prev(), Some(Step::Language));
        for s in STEPS {
            assert!(crate::copy::lint(s.subtitle()).is_empty(), "{s:?}");
        }
    }

    #[test]
    fn language_picks_keyboard() {
        let pt = LANGUAGES
            .iter()
            .position(|l| l.locale == "pt_BR.UTF-8")
            .unwrap();
        let mut d = Draft::default();
        d.set_language(pt);
        assert_eq!(KEYBOARDS[d.keyboard].0, "br");
        assert_eq!(LANGUAGES[0].locale, "en_US.UTF-8");
        assert!(
            LANGUAGES
                .iter()
                .all(|l| KEYBOARDS.iter().any(|k| k.0 == l.keymap))
        );
    }

    #[test]
    fn keyboard_applies_live() {
        assert_eq!(
            keyboard_command("us(intl)")[2],
            "keyword input:kb_layout us ; keyword input:kb_variant intl"
        );
        assert_eq!(
            keyboard_command("br")[2],
            "keyword input:kb_layout br ; keyword input:kb_variant "
        );
    }

    #[test]
    fn language_and_keyboard_are_detected() {
        let h = |lang: Option<&str>, zone: Option<&str>, keymap: Option<&str>| Hints {
            lang: lang.map(String::from),
            zone: zone.map(String::from),
            keymap: keymap.map(String::from),
        };
        let pick = |h: &Hints| {
            let d = Draft::detected(h);
            (LANGUAGES[d.language].locale, KEYBOARDS[d.keyboard].0)
        };
        assert_eq!(pick(&Hints::default()), ("en_US.UTF-8", "us"));
        assert_eq!(
            pick(&h(Some("C.UTF-8"), Some("America/Sao_Paulo"), None)),
            ("pt_BR.UTF-8", "br"),
            "the time zone speaks when LANG doesn't"
        );
        assert_eq!(
            pick(&h(Some("fr_FR.UTF-8"), Some("Europe/Berlin"), None)),
            ("fr_FR.UTF-8", "fr"),
            "LANG wins"
        );
        assert_eq!(
            pick(&h(None, Some("Europe/London"), Some("us.kbd"))),
            ("en_GB.UTF-8", "us"),
            "the console keymap names the keyboard"
        );
        assert_eq!(
            pick(&h(None, Some("Asia/Tokyo"), Some("jp.kbd"))),
            ("en_US.UTF-8", "us")
        );
        assert_eq!(
            pick(&h(None, None, Some("\"br.kbd\""))),
            ("en_US.UTF-8", "br")
        );

        let root = std::env::temp_dir().join(format!("wol-setup-hints-{}", std::process::id()));
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::create_dir_all(root.join("var/db")).unwrap();
        std::fs::write(
            root.join("etc/rc.conf"),
            "hostname=\"x\"\nkeymap=\"de.kbd\"\n",
        )
        .unwrap();
        std::fs::write(root.join("var/db/zoneinfo"), "Europe/Madrid\n").unwrap();
        let r = Hints::read(&root);
        assert_eq!(
            (r.zone.as_deref(), r.keymap.as_deref()),
            (Some("Europe/Madrid"), Some("de.kbd"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn account_draft() {
        use Field::*;
        let mut d = Draft::default();
        assert_eq!(
            d.account_problem(),
            Some((FullName, "Enter your full name."))
        );
        d.set_full_name("Alice Liddell");
        assert_eq!(d.name, "aliceliddell");
        d.set_name("alice");
        d.set_full_name("Alice P. Liddell");
        assert_eq!(d.name, "alice", "edited account name stays");
        d.set_name("Alice!");
        assert_eq!(d.account_problem().map(|p| p.0), Some(Name));
        d.set_name("alice");
        assert_eq!(d.account_problem(), Some((Password, "Choose a password.")));
        d.password = "curiouser".into();
        d.verify = "curiouserr".into();
        assert_eq!(
            d.account_problem(),
            Some((Verify, "Passwords don't match."))
        );
        d.verify = "curiouser".into();
        d.hint = "curiouser".into();
        assert_eq!(
            d.account_problem(),
            Some((Hint, "The hint can't be the password."))
        );
        d.hint = "white rabbit".into();
        assert_eq!(d.account_problem(), None);
        d.appearance = Appearance::Auto;
        let a = d.account();
        assert_eq!(
            (
                a.name.as_str(),
                a.full_name.as_str(),
                a.locale.as_str(),
                a.keymap.as_str(),
                a.appearance.as_str(),
                a.avatar.as_str(),
                a.accent.as_str(),
            ),
            (
                "alice",
                "Alice P. Liddell",
                "en_US.UTF-8",
                "us",
                "auto",
                "violet",
                "violet"
            )
        );
        assert_eq!(wol_core::settings::users::validate_account(&a), Ok(()));
        assert_eq!(initials_or_placeholder(""), "?");
        assert_eq!(initials_or_placeholder("alice liddell"), "AL");
        assert!(
            THEMES
                .iter()
                .all(|t| wol_core::theme::accent::ACCENTS.iter().any(|a| a.0 == t.0))
        );
    }

    /// wol-login as the assistant sees it: records every job, answers from a script.
    struct FakeLogin {
        jobs: RefCell<Vec<Job>>,
        answers: RefCell<Vec<Verdict>>,
    }

    impl FakeLogin {
        fn new(answers: &[Verdict]) -> FakeLogin {
            FakeLogin {
                jobs: Default::default(),
                answers: RefCell::new(answers.iter().rev().copied().collect()),
            }
        }
    }

    impl Backend for FakeLogin {
        fn run(&self, job: &Job) -> Verdict {
            self.jobs.borrow_mut().push(job.clone());
            self.answers.borrow_mut().pop().unwrap_or(Verdict::Ok)
        }
    }

    fn said(n: &Next) -> &str {
        match n {
            Next::Stay(_, s) => s,
            _ => "",
        }
    }

    /// Welcome to Get Started over Wi-Fi: a wrong password, a refused account, then success.
    #[test]
    fn setup_flow_end_to_end() {
        let fake = FakeLogin::new(&[
            Verdict::Denied, // Wi-Fi password
            Verdict::Ok,     // Wi-Fi again
            Verdict::Error,  // account: wol-login down
            Verdict::Ok,     // account
        ]);
        let hints = Hints {
            zone: Some("America/Sao_Paulo".into()),
            ..Hints::default()
        };
        let mut f = Flow::new(Draft::detected(&hints), false);
        assert_eq!(f.back(), None, "no way back from Welcome");
        assert_eq!(f.drive(&fake), Next::Go(Step::Language));
        assert_eq!(f.drive(&fake), Next::Go(Step::Wifi));

        // Nothing picked: Continue skips. Back, then pick a secured network.
        assert_eq!(f.back(), Some(Step::Language));
        f.drive(&fake);
        f.wifi = Some(WifiPick {
            ssid: "Rabbit Hole".into(),
            secure: true,
            ..WifiPick::default()
        });
        let n = f.drive(&fake);
        assert_eq!(said(&n), "Enter the password for “Rabbit Hole”.");
        f.wifi.as_mut().unwrap().password = "drinkme".into();
        let n = f.drive(&fake);
        assert_eq!(
            said(&n),
            "“Rabbit Hole” didn't accept that password. Check it and try again."
        );
        assert_eq!(f.step, Step::Wifi);
        assert_eq!(f.drive(&fake), Next::Go(Step::Account));

        // Account: one problem at a time, under its field.
        let n = f.drive(&fake);
        assert_eq!(
            n,
            Next::Stay(Some(Field::FullName), "Enter your full name.".into())
        );
        f.draft.set_full_name("Alice Liddell");
        f.draft.password = "curiouser".into();
        f.draft.verify = "curiouser".into();
        f.draft.avatar = 2;
        assert_eq!(f.drive(&fake), Next::Go(Step::Appearance));
        f.draft.appearance = Appearance::Dark;
        f.draft.theme = 1;

        let n = f.drive(&fake);
        assert_eq!(f.step, Step::Appearance, "{n:?}");
        assert!(said(&n).contains("try again"), "{n:?}");
        assert_eq!(f.drive(&fake), Next::Go(Step::Done));
        assert_eq!(f.back(), None, "the account exists: no way back");
        assert_eq!(f.drive(&fake), Next::Finish);

        let jobs = fake.jobs.borrow();
        assert_eq!(
            jobs[1],
            Job::JoinWifi {
                ssid: "Rabbit Hole".into(),
                password: "drinkme".into()
            }
        );
        let Job::CreateAccount(a) = &jobs[3] else {
            panic!("{jobs:?}")
        };
        let wire = a.encode();
        let body = std::str::from_utf8(&wire).unwrap();
        let (verb, rest) = body.split_once('\n').unwrap();
        let back = NewAccount::decode_verb(verb.trim_start_matches('!'), rest.as_bytes()).unwrap();
        assert_eq!(
            (
                back.name.as_str(),
                back.locale.as_str(),
                back.keymap.as_str(),
                back.appearance.as_str(),
                back.avatar.as_str(),
                back.accent.as_str(),
                back.password.as_str(),
            ),
            (
                "aliceliddell",
                "pt_BR.UTF-8",
                "br",
                "dark",
                "green",
                "blue",
                "curiouser"
            )
        );
        for n in [
            f.job_done(&jobs[0], Verdict::Error),
            f.job_done(&jobs[3], Verdict::Denied),
        ] {
            assert!(crate::copy::lint(said(&n)).is_empty(), "{n:?}");
        }
    }

    #[test]
    fn wired_machines_skip_wifi() {
        let fake = FakeLogin::new(&[]);
        let mut f = Flow::new(Draft::default(), true);
        f.drive(&fake);
        assert_eq!(f.drive(&fake), Next::Go(Step::Account));
        assert_eq!(f.back(), Some(Step::Language));
        // A network already joined: Continue goes on without asking.
        let mut f = Flow::new(Draft::default(), false);
        f.step = Step::Wifi;
        f.wifi = Some(WifiPick {
            ssid: "Wonderland".into(),
            secure: true,
            connected: true,
            password: String::new(),
        });
        assert_eq!(f.drive(&fake), Next::Go(Step::Account));
        assert!(fake.jobs.borrow().is_empty());
    }
}
