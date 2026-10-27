//! Menu bar app and Window menus: every item is a real action. Pure: frontmost window + clients +
//! usable zone in, Hyprland dispatch batch out. `wol-app <verb>` runs them (menu clicks and keybinds).
use crate::hypr::Client;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    About,
    Settings,
    Hide,
    HideOthers,
    Quit,
    /// Ends the app's process (a frozen app ignores Quit).
    ForceQuit,
    Minimise,
    Zoom,
    Fill,
    Centre,
    ToDesktop(u8),
    /// An app command (⌘C …) handed to the app in front as its own key.
    Key(Cmd),
}

/// App commands. ⌘ is Command: the app in front gets the key it listens for (Ctrl on this
/// system, the terminal's own key in a terminal). `wol-app <verb>` sends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Copy,
    Paste,
    Cut,
    Undo,
    Redo,
    SelectAll,
    Save,
    Open,
    Print,
    Find,
    FindNext,
    NewTab,
    /// Closes the tab, else the window (the app's Ctrl+W).
    Close,
    New,
    Reload,
    Location,
    ZoomIn,
    ZoomOut,
    ActualSize,
    /// ⌃⌘Space: GTK text fields open their emoji chooser on Ctrl+.
    Emoji,
}

impl Cmd {
    pub const ALL: [Cmd; 20] = [
        Cmd::Copy,
        Cmd::Paste,
        Cmd::Cut,
        Cmd::Undo,
        Cmd::Redo,
        Cmd::SelectAll,
        Cmd::Save,
        Cmd::Open,
        Cmd::Print,
        Cmd::Find,
        Cmd::FindNext,
        Cmd::NewTab,
        Cmd::Close,
        Cmd::New,
        Cmd::Reload,
        Cmd::Location,
        Cmd::ZoomIn,
        Cmd::ZoomOut,
        Cmd::ActualSize,
        Cmd::Emoji,
    ];

    /// (`wol-app` verb, Hyprland MODS, key) of the ⌘ shortcut in the shipped conf.
    const fn spec(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Cmd::Copy => ("copy", "SUPER", "C"),
            Cmd::Paste => ("paste", "SUPER", "V"),
            Cmd::Cut => ("cut", "SUPER", "X"),
            Cmd::Undo => ("undo", "SUPER", "Z"),
            Cmd::Redo => ("redo", "SUPER SHIFT", "Z"),
            Cmd::SelectAll => ("select-all", "SUPER", "A"),
            Cmd::Save => ("save", "SUPER", "S"),
            Cmd::Open => ("open", "SUPER", "O"),
            Cmd::Print => ("print", "SUPER", "P"),
            Cmd::Find => ("find", "SUPER", "F"),
            Cmd::FindNext => ("find-next", "SUPER", "G"),
            Cmd::NewTab => ("new-tab", "SUPER", "T"),
            Cmd::Close => ("close", "SUPER", "W"),
            Cmd::New => ("new", "SUPER", "N"),
            Cmd::Reload => ("reload", "SUPER", "R"),
            Cmd::Location => ("location", "SUPER", "L"),
            Cmd::ZoomIn => ("zoom-in", "SUPER", "equal"),
            Cmd::ZoomOut => ("zoom-out", "SUPER", "minus"),
            Cmd::ActualSize => ("actual-size", "SUPER", "0"),
            Cmd::Emoji => ("emoji", "CTRL SUPER", "Space"),
        }
    }
    pub fn verb(self) -> &'static str {
        self.spec().0
    }
    /// The shortcut as the conf spells it (MODS, key).
    pub fn keys(self) -> (&'static str, &'static str) {
        let (_, m, k) = self.spec();
        (m, k)
    }
}

/// What the app in front receives for a [`Cmd`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handoff {
    /// One tap of MODS + key (Hyprland `sendshortcut`) to the keyboard focus.
    Keys(&'static str, &'static str),
    /// Close the window (a terminal's Ctrl+W deletes a word).
    CloseWindow,
    /// Nothing: the app has no such command and its Ctrl key would do harm.
    Nothing,
}

/// A terminal app: does it have tabs, and its find key (with Ctrl+Shift).
struct Term {
    tabs: bool,
    find: &'static str,
}

const fn term(tabs: bool, find: &'static str) -> Term {
    Term { tabs, find }
}

/// Terminal window classes (lowercase). Their copy/paste/new window are Ctrl+Shift+C/V/N.
const TERMINALS: [(&str, Term); 10] = [
    ("foot", term(false, "R")),
    ("footclient", term(false, "R")),
    ("alacritty", term(false, "F")),
    ("xterm", term(false, "F")),
    ("kitty", term(true, "F")),
    ("org.wezfurlong.wezterm", term(true, "F")),
    ("com.mitchellh.ghostty", term(true, "F")),
    ("org.gnome.console", term(true, "F")),
    ("org.gnome.ptyxis", term(true, "F")),
    ("gnome-terminal-server", term(true, "F")),
];

/// What `c` sends to an app of window class `class` ("" = no window: desktop or a shell panel).
pub fn send(c: Cmd, class: &str) -> Handoff {
    let class = class.to_ascii_lowercase();
    let Some((_, t)) = TERMINALS.iter().find(|t| t.0 == class) else {
        return match c {
            Cmd::Emoji => Handoff::Keys("CTRL", "period"),
            _ => {
                let (_, m, k) = c.spec();
                Handoff::Keys(
                    if m.contains("SHIFT") {
                        "CTRL SHIFT"
                    } else {
                        "CTRL"
                    },
                    k,
                )
            }
        };
    };
    match c {
        Cmd::Copy => Handoff::Keys("CTRL SHIFT", "C"),
        Cmd::Paste => Handoff::Keys("CTRL SHIFT", "V"),
        Cmd::New => Handoff::Keys("CTRL SHIFT", "N"),
        Cmd::Find => Handoff::Keys("CTRL SHIFT", t.find),
        Cmd::ZoomIn | Cmd::ZoomOut | Cmd::ActualSize => Handoff::Keys("CTRL", c.spec().2),
        Cmd::NewTab if t.tabs => Handoff::Keys("CTRL SHIFT", "T"),
        Cmd::Close if t.tabs => Handoff::Keys("CTRL SHIFT", "W"),
        Cmd::Close => Handoff::CloseWindow,
        _ => Handoff::Nothing,
    }
}

/// Hyprland request for `c` with `class`/`address` in front; `panel` = a shell panel that may hold
/// the keyboard is open (then a window is never closed: the user may be typing in the panel).
/// `sendshortcut` over the socket is a press and release, so the app sees one tap, never a held key.
pub fn key_batch(c: Cmd, class: &str, address: &str, panel: bool) -> Option<String> {
    match send(c, class) {
        Handoff::Keys(m, k) => Some(format!("dispatch sendshortcut {m}, {k},")),
        Handoff::CloseWindow if !panel && !address.is_empty() => {
            Some(format!("dispatch closewindow address:{address}"))
        }
        _ => None,
    }
}

/// Shell surfaces that never take the keyboard (kit::surface specs and the banner/OSD panels).
const NO_KEYBOARD: [&str; 7] = [
    "wol-wallpaper",
    "wol-desktop",
    "wol-menubar",
    "wol-dock",
    "wol-panel-banners",
    "wol-panel-osd",
    "wol-panel-capture-thumb",
];

/// True when a mapped WolOS layer may hold the keyboard (Spotlight, Control Center, sheets…).
pub fn panel_has_keys(namespaces: &[String]) -> bool {
    namespaces
        .iter()
        .any(|n| n.starts_with("wol-") && !NO_KEYBOARD.contains(&n.as_str()))
}

/// Menu row: label ("" = separator), shortcut shown right-aligned, action.
pub type Row = (String, Option<&'static str>, Option<Act>);

/// Desktops offered by "Move to Desktop N" (Ctrl+1..4 switch to them).
pub const DESKTOPS: u8 = 4;
/// Title bar hyprbars draws on windows without client decorations (realtime titlebar::HEIGHT).
pub const BAR: i32 = 28;
/// Tag wol-hypr puts on windows that draw their own title bar.
pub const CSD_TAG: &str = "wol-csd";

/// Apps known to open their preferences on Ctrl+, (no fake "Settings…" elsewhere).
const SETTINGS_ON_CTRL_COMMA: &[&str] = &["org.gnome.TextEditor", "org.gnome.Nautilus"];

pub fn app_menu(name: &str, class: &str) -> Vec<Row> {
    if class.is_empty() {
        return Vec::new();
    }
    let row = |l: String, k, a| (l, k, Some(a));
    let sep = || (String::new(), None, None);
    let mut m = vec![row(format!("About {name}"), None, Act::About), sep()];
    if SETTINGS_ON_CTRL_COMMA.contains(&class) {
        m.push(row("Settings…".into(), Some("Ctrl+,"), Act::Settings));
        m.push(sep());
    }
    m.extend([
        row(format!("Hide {name}"), Some("Super+H"), Act::Hide),
        row("Hide Others".into(), Some("Super+Alt+H"), Act::HideOthers),
        sep(),
        row(format!("Quit {name}"), Some("Super+Q"), Act::Quit),
        // Named and in the menu on purpose: no key kills the frontmost app by accident.
        row(format!("Force Quit {name}"), None, Act::ForceQuit),
    ]);
    m
}

pub fn window_menu() -> Vec<Row> {
    let row = |l: &str, k, a| (l.to_string(), k, Some(a));
    let sep = || (String::new(), None, None);
    let mut m = vec![
        row("Minimise", Some("Super+M"), Act::Minimise),
        row("Zoom", None, Act::Zoom),
        sep(),
        row("Fill", None, Act::Fill),
        row("Centre", None, Act::Centre),
        sep(),
    ];
    m.extend((1..=DESKTOPS).map(|n| {
        (
            format!("Move to Desktop {n}"),
            None,
            Some(Act::ToDesktop(n)),
        )
    }));
    m
}

/// `wol-app` verb for an action (About is drawn by the shell, not dispatched).
pub fn verb(a: Act) -> Option<String> {
    Some(match a {
        Act::About => return None,
        Act::Settings => "settings".into(),
        Act::Hide => "hide".into(),
        Act::HideOthers => "hide-others".into(),
        Act::Quit => "quit".into(),
        Act::ForceQuit => "force-quit".into(),
        Act::Minimise => "minimise".into(),
        Act::Zoom => "zoom".into(),
        Act::Fill => "fill".into(),
        Act::Centre => "centre".into(),
        Act::ToDesktop(n) => format!("desktop {n}"),
        Act::Key(c) => c.verb().into(),
    })
}

pub fn parse_verb(v: &str) -> Option<Act> {
    let mut w = v.split_whitespace();
    let a = match (w.next()?, w.next()) {
        ("settings", None) => Act::Settings,
        ("hide", None) => Act::Hide,
        ("hide-others", None) => Act::HideOthers,
        ("quit", None) => Act::Quit,
        ("force-quit", None) => Act::ForceQuit,
        ("minimise", None) => Act::Minimise,
        ("zoom", None) => Act::Zoom,
        ("fill", None) => Act::Fill,
        ("centre", None) => Act::Centre,
        ("desktop", Some(n)) => match n.parse() {
            Ok(n) if (1..=DESKTOPS).contains(&n) => Act::ToDesktop(n),
            _ => return None,
        },
        (v, None) => match Cmd::ALL.into_iter().find(|c| c.verb() == v) {
            Some(c) => Act::Key(c),
            None => return None,
        },
        _ => return None,
    };
    w.next().is_none().then_some(a)
}

const HIDDEN: &str = "special:hidden";

/// Hyprland `[[BATCH]]` request for `a` on the frontmost window; None = nothing to do.
pub fn batch(
    a: Act,
    front: &Client,
    clients: &[Client],
    zone: (i32, i32, i32, i32),
) -> Option<String> {
    let at = &front.address;
    let hide = |c: &&Client| c.workspace.name != HIDDEN;
    let hide_cmd = |c: &Client| {
        format!(
            "dispatch movetoworkspacesilent {HIDDEN},address:{}",
            c.address
        )
    };
    let bar = if front.tags.iter().any(|t| t == CSD_TAG) {
        0
    } else {
        BAR
    };
    let (zx, zy, zw, zh) = zone;
    let cmds: Vec<String> = match a {
        Act::About => return None,
        Act::Settings => vec![format!("dispatch sendshortcut CTRL, comma, address:{at}")],
        Act::Hide => clients
            .iter()
            .filter(|c| c.class == front.class)
            .filter(hide)
            .map(hide_cmd)
            .collect(),
        Act::HideOthers => clients
            .iter()
            .filter(|c| c.class != front.class)
            .filter(hide)
            .map(hide_cmd)
            .collect(),
        Act::Quit => clients
            .iter()
            .filter(|c| c.class == front.class)
            .map(|c| format!("dispatch closewindow address:{}", c.address))
            .collect(),
        Act::ForceQuit => clients
            .iter()
            .filter(|c| c.class == front.class)
            .map(|c| format!("dispatch killwindow address:{}", c.address))
            .collect(),
        Act::Minimise => vec![hide_cmd(front)],
        Act::Zoom => vec!["dispatch fullscreen 1".into()],
        Act::Fill => vec![
            format!("dispatch setfloating address:{at}"),
            format!(
                "dispatch resizewindowpixel exact {zw} {},address:{at}",
                zh - bar
            ),
            format!(
                "dispatch movewindowpixel exact {zx} {},address:{at}",
                zy + bar
            ),
        ],
        Act::Centre => {
            let (w, h) = (front.size[0], front.size[1] + bar);
            let (x, y) = (zx + (zw - w) / 2, zy + (zh - h).max(0) / 2 + bar);
            vec![format!(
                "dispatch movewindowpixel exact {x} {y},address:{at}"
            )]
        }
        Act::ToDesktop(n) => vec![format!("dispatch movetoworkspace {n},address:{at}")],
        Act::Key(c) => return key_batch(c, &front.class, at, false),
    };
    (!cmds.is_empty()).then(|| format!("[[BATCH]]{}", cmds.join(";")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(addr: &str, class: &str, ws: &str) -> Client {
        let mut c = Client::new(addr, class, "t");
        c.workspace.name = ws.into();
        c.size = [400, 300];
        c
    }
    const ZONE: (i32, i32, i32, i32) = (0, 28, 1280, 670);

    fn labels(rows: &[Row]) -> Vec<(&str, Option<&str>)> {
        rows.iter().map(|r| (r.0.as_str(), r.1)).collect()
    }

    #[test]
    fn app_menu_names_the_app_and_only_offers_real_actions() {
        assert_eq!(
            labels(&app_menu("Terminal", "Alacritty")),
            [
                ("About Terminal", None),
                ("", None),
                ("Hide Terminal", Some("Super+H")),
                ("Hide Others", Some("Super+Alt+H")),
                ("", None),
                ("Quit Terminal", Some("Super+Q")),
                ("Force Quit Terminal", None),
            ]
        );
        let editor = app_menu("Text Editor", "org.gnome.TextEditor");
        assert_eq!(editor[2].0, "Settings…");
        assert_eq!(editor[2].2, Some(Act::Settings));
        // The empty desktop has no app to act on.
        assert!(app_menu("Desktop", "").is_empty());
    }

    #[test]
    fn window_menu_has_real_actions_and_every_desktop() {
        let w = window_menu();
        let names: Vec<&str> = w.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(
            names,
            [
                "Minimise",
                "Zoom",
                "",
                "Fill",
                "Centre",
                "",
                "Move to Desktop 1",
                "Move to Desktop 2",
                "Move to Desktop 3",
                "Move to Desktop 4"
            ]
        );
        assert_eq!(w[0].1, Some("Super+M"));
        assert!(w.iter().all(|r| r.0.is_empty() == r.2.is_none()));
    }

    #[test]
    fn verbs_round_trip() {
        for a in [
            Act::Settings,
            Act::Hide,
            Act::HideOthers,
            Act::Quit,
            Act::ForceQuit,
            Act::Minimise,
            Act::Zoom,
            Act::Fill,
            Act::Centre,
            Act::ToDesktop(3),
        ] {
            assert_eq!(parse_verb(&verb(a).unwrap()), Some(a), "{a:?}");
        }
        assert_eq!(verb(Act::About), None);
        assert_eq!(parse_verb("desktop 9"), None);
        assert_eq!(parse_verb("fly"), None);
    }

    #[test]
    fn batches_target_windows_by_address() {
        let front = c("0xa", "foot", "1");
        let all = [
            front.clone(),
            c("0xb", "foot", "2"),
            c("0xc", "firefox", "1"),
            c("0xd", "gimp", "special:hidden"),
        ];
        let b = |a| batch(a, &front, &all, ZONE).unwrap();
        assert_eq!(
            b(Act::Hide),
            "[[BATCH]]dispatch movetoworkspacesilent special:hidden,address:0xa;dispatch movetoworkspacesilent special:hidden,address:0xb"
        );
        // Already hidden windows stay put.
        assert_eq!(
            b(Act::HideOthers),
            "[[BATCH]]dispatch movetoworkspacesilent special:hidden,address:0xc"
        );
        assert_eq!(
            b(Act::Quit),
            "[[BATCH]]dispatch closewindow address:0xa;dispatch closewindow address:0xb"
        );
        // A frozen app ignores closewindow: Force Quit ends it, every window of the app.
        assert_eq!(
            b(Act::ForceQuit),
            "[[BATCH]]dispatch killwindow address:0xa;dispatch killwindow address:0xb"
        );
        assert_eq!(
            b(Act::Minimise),
            "[[BATCH]]dispatch movetoworkspacesilent special:hidden,address:0xa"
        );
        assert_eq!(b(Act::Zoom), "[[BATCH]]dispatch fullscreen 1");
        assert_eq!(
            b(Act::ToDesktop(2)),
            "[[BATCH]]dispatch movetoworkspace 2,address:0xa"
        );
        assert_eq!(
            b(Act::Settings),
            "[[BATCH]]dispatch sendshortcut CTRL, comma, address:0xa"
        );
        assert_eq!(batch(Act::About, &front, &all, ZONE), None);
    }

    /// Terminals: ⌘C/⌘V copy and paste (Ctrl+Shift+C/V); a terminal's Ctrl+letter is a control
    /// character (Ctrl+C interrupts, Ctrl+Z suspends, Ctrl+S freezes output), so none is ever sent.
    #[test]
    fn terminals_get_their_own_keys_and_never_a_control_character() {
        use Handoff::*;
        for (cmd, class, want) in [
            (Cmd::Copy, "foot", Keys("CTRL SHIFT", "C")),
            (Cmd::Paste, "foot", Keys("CTRL SHIFT", "V")),
            (Cmd::Paste, "Alacritty", Keys("CTRL SHIFT", "V")),
            (Cmd::New, "foot", Keys("CTRL SHIFT", "N")),
            (Cmd::Find, "foot", Keys("CTRL SHIFT", "R")),
            (Cmd::Find, "kitty", Keys("CTRL SHIFT", "F")),
            (Cmd::ZoomIn, "foot", Keys("CTRL", "equal")),
            (Cmd::ActualSize, "footclient", Keys("CTRL", "0")),
            (Cmd::Close, "foot", CloseWindow),
            (Cmd::Close, "org.gnome.Console", Keys("CTRL SHIFT", "W")),
            (Cmd::NewTab, "kitty", Keys("CTRL SHIFT", "T")),
            (Cmd::NewTab, "foot", Nothing),
            (Cmd::Undo, "foot", Nothing),
            (Cmd::Save, "foot", Nothing),
            (Cmd::Copy, "Firefox", Keys("CTRL", "C")),
            (Cmd::Close, "firefox", Keys("CTRL", "W")),
            (Cmd::Redo, "org.gnome.TextEditor", Keys("CTRL SHIFT", "Z")),
            (Cmd::Emoji, "org.gnome.TextEditor", Keys("CTRL", "period")),
            // Empty desktop or a shell panel in front: the keyboard focus gets the Ctrl key.
            (Cmd::Paste, "", Keys("CTRL", "V")),
        ] {
            assert_eq!(send(cmd, class), want, "{cmd:?} in {class}");
        }
        for class in TERMINALS.iter().map(|t| t.0) {
            for c in Cmd::ALL {
                if let Keys(m, k) = send(c, class) {
                    let letter = k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic();
                    assert!(!letter || m.contains("SHIFT"), "{c:?} in {class}: {m} {k}");
                }
            }
        }
    }

    #[test]
    fn commands_dispatch_to_the_keyboard_focus() {
        assert_eq!(
            key_batch(Cmd::Copy, "org.gnome.TextEditor", "0xa", false).as_deref(),
            Some("dispatch sendshortcut CTRL, C,")
        );
        assert_eq!(
            key_batch(Cmd::Copy, "foot", "0xa", true).as_deref(),
            Some("dispatch sendshortcut CTRL SHIFT, C,"),
            "a panel field takes Ctrl+Shift+C too (kit::widgets::terminal_clipboard_keys)"
        );
        assert_eq!(
            key_batch(Cmd::Close, "foot", "0xa", false).as_deref(),
            Some("dispatch closewindow address:0xa")
        );
        // A shell panel may hold the keyboard: ⌘W must not close the terminal behind it.
        assert_eq!(key_batch(Cmd::Close, "foot", "0xa", true), None);
        assert_eq!(
            key_batch(Cmd::Close, "", "", false).as_deref(),
            Some("dispatch sendshortcut CTRL, W,"),
            "no window in front: the panel with the keyboard gets Ctrl+W"
        );
        assert_eq!(key_batch(Cmd::Undo, "foot", "0xa", false), None);
        for c in Cmd::ALL {
            assert_eq!(parse_verb(&verb(Act::Key(c)).unwrap()), Some(Act::Key(c)));
        }
        assert_ne!(parse_verb("zoom"), parse_verb("zoom-in"));
    }

    /// Surfaces that never take the keyboard do not count as a panel in front.
    #[test]
    fn keyboard_panels_from_layer_namespaces() {
        let j = r#"{"eDP-1":{"levels":{"0":[{"namespace":"wol-wallpaper"}],"2":[{"namespace":"wol-menubar"},{"namespace":"wol-dock"}],"3":[{"namespace":"wol-panel-banners"},{"namespace":"wol-panel-osd"}]}}}"#;
        assert!(!panel_has_keys(&crate::hypr::parse_layers(j)));
        let j = r#"{"eDP-1":{"levels":{"3":[{"namespace":"wol-panel-spotlight"}]}}}"#;
        assert!(panel_has_keys(&crate::hypr::parse_layers(j)));
        assert!(!panel_has_keys(&crate::hypr::parse_layers(
            r#"{"eDP-1":{"levels":{"3":[{"namespace":"waybar"}]}}}"#
        )));
    }

    #[test]
    fn fill_and_centre_respect_the_title_bar() {
        let front = c("0xa", "foot", "1");
        // hyprbars draws 28 px above the client area.
        assert_eq!(
            batch(Act::Fill, &front, &[], ZONE).unwrap(),
            "[[BATCH]]dispatch setfloating address:0xa;dispatch resizewindowpixel exact 1280 642,address:0xa;dispatch movewindowpixel exact 0 56,address:0xa"
        );
        // Frame 400x328 centred in 1280x670 at y 28: frame top 199, client top 227.
        assert_eq!(
            batch(Act::Centre, &front, &[], ZONE).unwrap(),
            "[[BATCH]]dispatch movewindowpixel exact 440 227,address:0xa"
        );
        let mut csd = front.clone();
        csd.tags = vec![CSD_TAG.into()];
        assert_eq!(
            batch(Act::Fill, &csd, &[], ZONE).unwrap(),
            "[[BATCH]]dispatch setfloating address:0xa;dispatch resizewindowpixel exact 1280 670,address:0xa;dispatch movewindowpixel exact 0 28,address:0xa"
        );
    }
}
