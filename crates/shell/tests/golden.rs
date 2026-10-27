//! Golden screenshots of every surface, light and dark, with the layout check on.
//! Runs under a private `gtk4-broadwayd` (headless). `WOL_GOLDEN_UPDATE=1` rewrites goldens.
//! Renders land in `$CARGO_TARGET_TMPDIR/golden/`.
use gtk::{gdk, glib, prelude::*};
use shell::kit::{self, widgets as w};
use shell::theme::{self, Variant};
use shell::ui;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[path = "golden_cases/store_package.rs"]
mod package_cases;
#[path = "golden_cases/widgets.rs"]
mod widget_cases;
#[path = "golden_cases/wol_file.rs"]
mod wol_file;

const WALL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../design/wallpapers/wonderland-dusk.svg"
);
/// Max share of pixels allowed to differ (any channel > 24) before a golden fails.
const TOLERANCE: f64 = 0.005;

struct Case {
    name: String,
    size: (i32, i32),
    class: &'static str,
    wallpaper: bool,
    build: Box<dyn Fn() -> gtk::Widget>,
}

fn cases() -> Vec<Case> {
    let mut v: Vec<Case> = vec![
        Case {
            name: "menubar".into(),
            size: (900, 28),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let b = ui::menubar::content(&shell::system::Fake);
                b.set_app("Files".into(), "wol-files".into());
                b.root.clone().upcast()
            }),
        },
        Case {
            // No focused window: no app name and no app/Window menus (no resident Finder).
            name: "menubar-desktop".into(),
            size: (900, 28),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let b = ui::menubar::content(&shell::system::Fake);
                b.set_app(String::new(), String::new());
                b.root.clone().upcast()
            }),
        },
        Case {
            name: "about-app".into(),
            size: (320, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                ui::panels::about_card("Text Editor", "wolos-files", "org.gnome.TextEditor")
                    .upcast()
            }),
        },
        Case {
            name: "dock".into(),
            size: (900, 120),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let running = ["foot".to_string(), "org.gnome.nautilus".into()]
                    .into_iter()
                    .collect();
                ui::dock::content(&running).root.clone().upcast()
            }),
        },
        Case {
            // A helper (wol mcp) asks before a change: Deny, Always Allow, Allow Once.
            name: "approval".into(),
            size: (400, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                ui::approval::content(
                    Rc::new(shell::system::Fake),
                    shared::shell::Approval {
                        helper: "Claude Desktop".into(),
                        tool: "appearance set".into(),
                        request: shared::shell::ApprovalRequest::Change {
                            capability: "appearance".into(),
                            what: "switch to Dark appearance".into(),
                        },
                    },
                )
            }),
        },
        Case {
            // An administrator change from a helper: the password, every time.
            name: "approval-admin".into(),
            size: (400, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                ui::approval::content(
                    Rc::new(shell::system::Fake),
                    shared::shell::Approval {
                        helper: "Claude Desktop".into(),
                        tool: "wifi join".into(),
                        request: shared::shell::ApprovalRequest::WifiJoin {
                            ssid: "Wonderland".into(),
                            password: Some("hunter22".into()),
                        },
                    },
                )
            }),
        },
        Case {
            name: "controlcenter".into(),
            size: (368, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| ui::control_center::content(Rc::new(shell::system::Fake))),
        },
        Case {
            name: "spotlight".into(),
            size: (728, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let items = Rc::new(std::cell::RefCell::new(ui::spotlight::index(vec![])));
                ui::spotlight::content(items, Default::default(), "te").0
            }),
        },
        Case {
            name: "spotlight-files".into(),
            size: (728, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let items = Rc::new(std::cell::RefCell::new(ui::spotlight::index(vec![])));
                let names = shell::files::Index::parse(
                    "Documents/\nDocuments/Taxes 2026.pdf\nDocuments/Tax Receipts/\n\
                     Pictures/tax receipt.jpeg\nDownloads/syntax-cheatsheet.md\n",
                    std::path::Path::new("/home/alice"),
                );
                ui::spotlight::content(items, Rc::new(std::cell::RefCell::new(names)), "tax").0
            }),
        },
        Case {
            name: "spotlight-empty".into(),
            size: (728, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let items = Rc::new(std::cell::RefCell::new(ui::spotlight::index(vec![])));
                ui::spotlight::content(items, Default::default(), "qwzx kettle").0
            }),
        },
        Case {
            name: "lock".into(),
            size: (900, 600),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| ui::lock::lock_screen(&w::UserPanel::new("Alice Liddell"))),
        },
        Case {
            name: "lock-rejected".into(),
            size: (900, 600),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| {
                let p = w::UserPanel::new("Alice Liddell");
                p.reject(shell::login::Verdict::Denied.message());
                ui::lock::lock_screen(&p)
            }),
        },
        Case {
            name: "login".into(),
            size: (900, 600),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| {
                ui::lock::Login::new(&[
                    ("alice".into(), "alice".into()),
                    ("dinah".into(), "dinah".into()),
                    ("hatter".into(), "Mad Hatter".into()),
                ])
                .root
                .clone()
            }),
        },
        Case {
            name: "login-password".into(),
            size: (900, 600),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| {
                let l = ui::lock::Login::new(&[
                    ("alice".into(), "alice".into()),
                    ("dinah".into(), "dinah".into()),
                ]);
                l.pick("dinah", "dinah");
                l.root.clone()
            }),
        },
        Case {
            name: "switcher".into(),
            size: (900, 200),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let c = |a: &str, class: &str, f: i64| shell::hypr::Client {
                    focus: f,
                    ..shell::hypr::Client::new(a, class, "")
                };
                let apps = shell::switcher::apps(
                    &[
                        c("0x1", "foot", 0),
                        c("0x2", "org.gnome.Nautilus", 1),
                        c("0x3", "firefox", 2),
                        c("0x4", "foot", 3),
                        c("0x5", "org.wolos.settings", 4),
                        c("0x6", "org.gnome.Geary", 5),
                    ],
                    &[],
                );
                let s = ui::switcher::Strip::new(&apps, 1);
                s.root.set_halign(gtk::Align::Center);
                s.root.set_valign(gtk::Align::Center);
                s.root.clone().upcast()
            }),
        },
        Case {
            // ⌥⌘Esc: open apps, most recent first, the frontmost chosen.
            name: "force-quit".into(),
            size: (420, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let c = |a: &str, class: &str, f: i64| shell::hypr::Client {
                    focus: f,
                    ..shell::hypr::Client::new(a, class, "")
                };
                let apps = shell::switcher::apps(
                    &[
                        c("0x1", "org.gnome.Nautilus", 0),
                        c("0x2", "firefox", 1),
                        c("0x3", "foot", 2),
                        c("0x4", "org.gnome.Geary", 3),
                    ],
                    &[],
                );
                let p = ui::force_quit::Picker::new(|_, _| {});
                p.set_apps(&apps);
                p.root.clone().upcast()
            }),
        },
        Case {
            name: "force-quit-empty".into(),
            size: (420, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let p = ui::force_quit::Picker::new(|_, _| {});
                p.set_apps(&[]);
                p.root.clone().upcast()
            }),
        },
        Case {
            name: "overview".into(),
            size: (1280, 800),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| {
                let c = |a: &str, class: &str, title: &str, ws: i64, size: [i32; 2]| {
                    let mut c = shell::hypr::Client::new(a, class, title);
                    c.workspace.id = ws;
                    c.size = size;
                    c
                };
                let clients = vec![
                    c("0x1", "foot", "~/src/wolos — fish", 1, [900, 560]),
                    c("0x2", "wol-files", "Documents", 1, [1100, 700]),
                    c("0x3", "firefox", "WolOS — The friendly BSD", 1, [1280, 760]),
                    c("0x4", "wol-mail", "Inbox", 1, [800, 600]),
                    c("0x5", "wol-music", "Music", 2, [800, 600]),
                    c("0x6", "wol-notes", "Notes", 3, [800, 600]),
                ];
                let desktops = shell::overview::desktops(&clients, 1);
                let here = clients
                    .iter()
                    .filter(|c| c.workspace.id == 1)
                    .map(|client| ui::overview::Window {
                        client,
                        thumb: None,
                    })
                    .collect();
                ui::overview::content((1280, 800), &desktops, 1, here, |_| {}).0
            }),
        },
        Case {
            name: "capture-bar".into(),
            size: (900, 140),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let b = ui::capture::Toolbar::new(
                    shell::capture::Mode::Selection,
                    shell::capture::Options::default(),
                );
                b.root.set_halign(gtk::Align::Center);
                b.root.set_valign(gtk::Align::End);
                b.root.clone().upcast()
            }),
        },
        Case {
            name: "capture-thumb".into(),
            size: (480, 240),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let shot = Path::new(WALL);
                let t = gdk::Texture::from_filename(shot).expect("wallpaper");
                let th = ui::capture::thumbnail(&t, shot);
                let b = w::vbox(0);
                b.set_vexpand(true);
                th.set_vexpand(true);
                b.append(&th);
                b.upcast()
            }),
        },
        Case {
            name: "banners".into(),
            size: (420, 360),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                let n = |app: &str, summary: &str, body: &str| shell::ui::notify::Note {
                    id: 1,
                    app: app.into(),
                    icon: String::new(),
                    summary: summary.into(),
                    body: body.into(),
                    critical: false,
                    timeout_ms: -1,
                    image: String::new(),
                    actions: vec![],
                };
                let col = w::vbox(0);
                col.add_css_class("banners-preview");
                col.set_halign(gtk::Align::End);
                col.set_valign(gtk::Align::Start);
                col.append(&ui::notify::banner(
                    &n("Mail", "Dinah", "Tea at four? The Hatter insists it is always six o'clock, so bring a watch that agrees with him and some extra cups."),
                    1,
                ));
                col.append(&ui::notify::banner(
                    &n("Music", "Now Playing", "White Rabbit — Jefferson Airplane"),
                    3,
                ));
                col.append(&ui::notify::banner(
                    &n("Calendar", "Croquet with the Queen", ""),
                    1,
                ));
                col.upcast()
            }),
        },
        Case {
            name: "banner-screenshot".into(),
            size: (420, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                // What wol-screenshot sends: the folder in the title, the picture as thumbnail.
                let note = shell::ui::notify::Note {
                    id: 1,
                    app: "Screenshot".into(),
                    icon: "wolos-photos".into(),
                    summary: "Screenshot saved to Desktop".into(),
                    body: String::new(),
                    critical: false,
                    timeout_ms: -1,
                    image: concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../design/wallpapers/wonderland-day.svg"
                    )
                    .into(),
                    actions: vec![],
                };
                let col = w::vbox(0);
                col.add_css_class("banners-preview");
                col.append(&ui::notify::banner(&note, 1));
                col.upcast()
            }),
        },
        Case {
            name: "banner-action".into(),
            size: (420, 1),
            class: "preview",
            wallpaper: true,
            build: Box::new(|| {
                // The NVIDIA driver finished installing online: one Restart button.
                let note = shell::ui::notify::Note {
                    id: 1,
                    app: "WolOS".into(),
                    icon: String::new(),
                    summary: "Graphics driver installed".into(),
                    body: "Restart to finish setting up your graphics card.".into(),
                    critical: false,
                    timeout_ms: -1,
                    image: String::new(),
                    actions: vec![("restart".into(), "Restart".into())],
                };
                let col = w::vbox(0);
                col.add_css_class("banners-preview");
                col.append(&ui::notify::banner(&note, 1));
                col.upcast()
            }),
        },
    ];
    for (sw, sh, tag) in [(1280, 800, "1280"), (1024, 640, "1024")] {
        for st in shell::setup::STEPS {
            v.push(Case {
                name: format!("setup-{}-{tag}", st.id()),
                size: (sw, sh),
                class: "lock-window",
                wallpaper: false,
                build: Box::new(move || {
                    let s = ui::setup::Setup::new(&shell::system::Fake);
                    s.show(st);
                    s.root.clone()
                }),
            });
        }
    }
    v.push(Case {
        name: "login-hint".into(),
        size: (900, 600),
        class: "lock-window",
        wallpaper: false,
        build: Box::new(|| {
            let l = ui::lock::Login::new(&[
                ("alice".into(), "alice".into()),
                ("dinah".into(), "dinah".into()),
            ]);
            l.pick("alice", "alice");
            l.panel.reject(shell::login::Verdict::Denied.message());
            let hint = shell::login::hint_message(3, Some("white rabbit"));
            l.panel.message.set_label(&hint.unwrap_or_default());
            l.root.clone()
        }),
    });
    for v2 in [(1280, 800, "1280"), (1024, 640, "1024")] {
        v.push(Case {
            name: format!("setup-account-filled-{}", v2.2),
            size: (v2.0, v2.1),
            class: "lock-window",
            wallpaper: false,
            build: Box::new(|| {
                use shell::setup::Field;
                let s = ui::setup::Setup::new(&shell::system::Fake);
                s.show(shell::setup::Step::Account);
                s.type_into(Field::FullName, "Alice Liddell");
                s.type_into(Field::Password, "curiouser");
                s.type_into(Field::Verify, "curious");
                s.next(); // Continue: the problem shows at the foot, the field is outlined
                s.root.clone()
            }),
        });
    }
    for (sw, sh, tag) in [(1280, 800, "1280"), (1024, 640, "1024")] {
        for page in [
            "choose",
            "choose-windows",
            "secure-boot",
            "installing",
            "done",
            "failed",
        ] {
            v.push(Case {
                name: format!("installer-{page}-{tag}"),
                size: (sw, sh),
                class: "lock-window",
                wallpaper: false,
                build: Box::new(move || {
                    use shell::install::{Check, Disk, Report, Stage};
                    use ui::installer::{Installer, Page};
                    let disk = |name: &str, model: &str, gb: u64| Disk {
                        name: name.into(),
                        model: model.into(),
                        bytes: gb * 1_000_000_000,
                    };
                    let windows = Check {
                        others: vec!["Windows".into()],
                        ..Check::default()
                    };
                    let i = Installer::new(
                        vec![
                            disk("nda0", "Samsung SSD 980 PRO", 1000),
                            disk("ada0", "WD Blue SA510", 500),
                        ],
                        vec![Check::default(), windows],
                    );
                    match page {
                        "choose" => i.pick(0),
                        "choose-windows" => i.pick(1),
                        "secure-boot" => i.show(Page::SecureBoot),
                        "installing" => {
                            i.show_progress(&Report::new(Stage::Packages, 53), Some(200.0))
                        }
                        "done" => i.show(Page::Done),
                        _ => i.show_failed(Stage::Disk),
                    }
                    i.root.clone()
                }),
            });
        }
    }
    for (sw, sh, tag) in [(900, 600, "wide"), (480, 360, "narrow")] {
        for p in ui::store::PAGES.into_iter().chain(["Search"]) {
            v.push(Case {
                name: format!("store-{}-{tag}", p.to_lowercase()),
                size: (sw, sh),
                class: "preview-app",
                wallpaper: false,
                build: Box::new(move || {
                    let s = ui::store::content(
                        Rc::new(shell::system::Fake),
                        ui::store::Env {
                            data_dirs: vec![],
                            pkg_search: false,
                        },
                    );
                    if p == "Search" {
                        s.search_now("photo");
                    } else {
                        s.aw.select(p);
                    }
                    s.aw.root.clone().upcast()
                }),
            });
        }
        for p in ui::settings::PAGES
            .into_iter()
            .chain(ui::settings::SUBPAGES)
        {
            v.push(Case {
                name: format!(
                    "settings-{}-{tag}",
                    p.to_lowercase().replace([' ', '&'], "")
                ),
                size: (sw, sh),
                class: "preview-app",
                wallpaper: false,
                build: Box::new(move || {
                    let a = ui::settings::content(
                        Rc::new(shell::system::Fake),
                        &ui::settings::Host::demo(),
                    );
                    a.open(p);
                    a.root.clone().upcast()
                }),
            });
        }
    }
    // Session menu and the Restart / Shut Down / Log Out question (60 s countdown).
    v.push(Case {
        name: "session".into(),
        size: (443, 142),
        class: "preview",
        wallpaper: true,
        build: Box::new(|| ui::panels::SessionMenu::new().root.upcast()),
    });
    for (action, width) in [("shutdown", 448), ("logout", 440)] {
        v.push(Case {
            name: format!("session-confirm-{action}"),
            size: (width, 144),
            class: "preview",
            wallpaper: true,
            build: Box::new(move || {
                let m = ui::panels::SessionMenu::new();
                m.preview(action);
                m.root.upcast()
            }),
        });
    }
    v.extend(control_center_cases());
    v.extend(form_kit_cases());
    v.extend(widget_cases::cases());
    v.extend(package_cases::cases());
    for (tag, (sw, _)) in [("wide", w::APP_SIZE), ("narrow", w::APP_NARROW)] {
        v.push(Case {
            name: format!("settings-windows-tiling-{tag}"),
            size: (sw, 1),
            class: "preview-app",
            wallpaper: false,
            build: Box::new(|| {
                let p = w::vbox(w::S4);
                p.add_css_class("page");
                p.add_css_class("app-window");
                p.append(&ui::settings::windows_section(
                    Rc::new(shell::system::Fake),
                    wol_core::hyprprefs::DesktopPrefs {
                        tiling: true,
                        layout: wol_core::hyprprefs::Layout::Master,
                        ..Default::default()
                    },
                ));
                p.upcast()
            }),
        });
    }
    v
}

/// Settings-style Kit components (`kit::form` and the pickers panes share), on the app surface.
fn form_kit_cases() -> Vec<Case> {
    use shell::kit::form as f;
    use wol_core::shortcuts::Combo;
    let case = |name: &str, build: fn() -> gtk::Widget| Case {
        name: format!("kit-{name}"),
        size: (w::APP_NARROW.0, 1),
        class: "preview-app",
        wallpaper: false,
        build: Box::new(move || {
            let p = w::vbox(w::S4);
            p.add_css_class("page");
            p.add_css_class("app-window");
            p.append(&build());
            p.upcast()
        }),
    };
    fn combo(m: &str, k: &str) -> Combo {
        Combo::parse(m, k).unwrap()
    }
    vec![
        case("form-rows", || {
            w::section(
                "Windows",
                &[
                    f::toggle_row(
                        "Tile windows",
                        "New windows fill the screen side by side",
                        true,
                        |_| {},
                    ),
                    f::picker_row("Layout", "", &["Dwindle", "Master"], 0, |_| {}),
                    f::segmented_row("Appearance", "", &["Light", "Dark", "Auto"], 2, |_| {}),
                    f::stepper_row("Gaps", "", 12, (0, 32, 4), |v| format!("{v} pt"), |_| {}),
                    f::info_row("Processor", "Intel Core i5 (8 cores)"),
                    f::entry_row("Computer name", "wonderland", "Name", |_| {}),
                    f::nav_row("help-about-symbolic", "About", "Name, hardware, serial").upcast(),
                ],
            )
            .upcast()
        }),
        case("slider-row", || {
            w::section(
                "",
                &[
                    f::slider_row(
                        "Key repeat rate",
                        (5.0, 60.0, 5.0),
                        25.0,
                        ("Slow", "Fast"),
                        |_| {},
                    ),
                    f::slider_row(
                        "Delay until repeat",
                        (150.0, 1000.0, 50.0),
                        400.0,
                        ("Long", "Short"),
                        |_| {},
                    ),
                ],
            )
            .upcast()
        }),
        case("shortcut-recorder", || {
            let c = combo("SUPER", "Space");
            let ok = f::ShortcutRecorder::new(Some(&c), |_| {}, |_| {});
            let taken = f::ShortcutRecorder::new(Some(&combo("SUPER SHIFT", "4")), |_| {}, |_| {});
            taken.set_conflict(true);
            let off = f::ShortcutRecorder::new(None, |_| {}, |_| {});
            let fixed =
                f::ShortcutRecorder::new(Some(&combo("SUPER", "mouse:272")), |_| {}, |_| {});
            fixed.set_editable(false);
            let swap = w::button("Swap", w::Kind::Default, || {});
            let cancel = w::button("Cancel", w::Kind::Plain, || {});
            w::section(
                "Screenshots",
                &[
                    f::row("Show Spotlight search", "", &ok.button),
                    f::row("Capture the screen", "", &taken.button),
                    f::notice("⇧⌘4 is used by “Capture a selection”.", &[swap, cancel]).upcast(),
                    f::row("Record the screen", "", &off.button),
                    f::row("Move window", "", &fixed.button),
                ],
            )
            .upcast()
        }),
        case("permission-row", || {
            w::section(
                "Screen Recording",
                &[
                    f::permission_row(
                        "wolos-browser",
                        "Browser",
                        "Can record the screen",
                        true,
                        |_| {},
                    ),
                    f::permission_row(
                        "wolos-terminal",
                        "Terminal",
                        "Asked yesterday",
                        false,
                        |_| {},
                    ),
                ],
            )
            .upcast()
        }),
        case("wallpaper-tile", || {
            let r = w::wrap_row(w::S3, 3);
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/wallpapers");
            for (i, n) in ["wonderland-day", "wonderland-dusk", "wonderland-night"]
                .iter()
                .enumerate()
            {
                let p = std::path::PathBuf::from(format!("{dir}/{n}.svg"));
                r.insert(&f::wallpaper_tile(Some(&p), n, i == 1), -1);
            }
            r.upcast()
        }),
        case("empty-state", || {
            w::empty_state(
                "bluetooth-disabled-symbolic",
                "No Bluetooth Hardware",
                "This computer has no Bluetooth adapter. Plug in a USB adapter to use wireless devices.",
                None,
            )
            .upcast()
        }),
        case("search-field", || w::search_field("Search").upcast()),
        case("preset-card", || {
            use shell::kit::pickers::{PresetCard, Tokens};
            use wol_core::looks::Looks;
            let r = w::wrap_row(w::S3, 2);
            let soft = Looks {
                rounding: 18,
                ..Looks::default()
            };
            for (name, about, l, on) in [
                (
                    "WolOS",
                    "Soft glass, gentle motion.",
                    Looks::default(),
                    true,
                ),
                ("Rounded", "Round corners and wide gaps.", soft, false),
            ] {
                let c = PresetCard::new(name, about, &l, Tokens::current(), on);
                r.insert(&c.button, -1);
            }
            r.upcast()
        }),
        case("color-well", || {
            use shell::kit::pickers::{ColorWell, Tokens};
            let row = w::hbox(w::S2);
            for hex in ["#7c5cffff", "#33ccffee", "#ffffff40"] {
                row.append(&ColorWell::new(hex, &[], Tokens::current(), |_| {}).button);
            }
            w::section("", &[f::row("Border colours", "", &row)]).upcast()
        }),
        case("usage-bar", || {
            let parts: Vec<(String, String, f64)> = [
                ("Documents", "61 GB", 0.12),
                ("Apps", "38 GB", 0.07),
                ("System", "24 GB", 0.05),
                ("Restore points", "12 GB", 0.02),
                ("Other", "8 GB", 0.02),
            ]
            .iter()
            .map(|(n, s, f)| ((*n).into(), (*s).into(), *f))
            .collect();
            shell::kit::pickers::usage_bar(&parts).upcast()
        }),
        case("key-legend", || {
            shell::kit::pickers::key_legend(&ui::settings::KEY_LEGEND)
        }),
        case("toast", || {
            let aw = w::AppWindow::new("", false, false);
            let t = shell::kit::toast::Toast::new(&aw);
            t.show("Wallpaper changed", "Undo", || {});
            t.revealer.set_transition_duration(0);
            aw.root.set_size_request(-1, 160);
            aw.root.upcast()
        }),
    ]
}

fn pump(ms: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let ctx = glib::MainContext::default();
    while std::time::Instant::now() < until {
        while ctx.iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Control Center: laptop (battery) and desktop variants, every module on, each detail page, and
/// its Kit components on their own.
fn control_center_cases() -> Vec<Case> {
    use shell::kit::controls as k;
    use shell::system::{Fake, FakeDesktop, System};
    use wol_core::prefs::ControlCenterPrefs;
    let cc = |name: &str, sys: fn() -> Rc<dyn System>, all: bool, page: &'static str| Case {
        name: name.into(),
        size: (368, 1),
        class: "preview",
        wallpaper: true,
        build: Box::new(move || {
            let mut p = ControlCenterPrefs::default();
            if all {
                for (key, _) in ControlCenterPrefs::MODULES {
                    p.set(key, true);
                }
            }
            ui::control_center::content_with(sys(), p, page)
        }),
    };
    let laptop: fn() -> Rc<dyn System> = || Rc::new(Fake);
    let desktop: fn() -> Rc<dyn System> = || Rc::new(FakeDesktop);
    let kit_case = |name: &str, build: fn() -> gtk::Widget| Case {
        name: format!("kit-{name}"),
        size: (368, 1),
        class: "preview",
        wallpaper: true,
        build: Box::new(move || {
            let p = w::glass_panel(w::S2);
            p.set_width_request(w::CONTROL_CENTER_WIDTH);
            p.append(&build());
            p.upcast()
        }),
    };
    vec![
        cc("controlcenter-desktop", desktop, false, "main"),
        cc("controlcenter-all", laptop, true, "main"),
        cc("controlcenter-wifi", laptop, false, "wifi"),
        cc("controlcenter-wifi-other", laptop, false, "wifi:other"),
        cc("controlcenter-bluetooth", laptop, false, "bluetooth"),
        cc("controlcenter-sound", laptop, false, "sound"),
        cc("controlcenter-focus", laptop, false, "focus"),
        kit_case("toggle-tile", || {
            let r = w::hbox(w::S2);
            r.set_homogeneous(true);
            let on = k::ToggleTile::new("night-light-symbolic", "Night Shift");
            on.set_on(true);
            r.append(&on.button);
            r.append(&k::ToggleTile::new("wolos-contrast-symbolic", "Increase Contrast").button);
            r.append(&k::ToggleTile::new("camera-photo-symbolic", "Screenshot").button);
            r.upcast()
        }),
        kit_case("device-row", || {
            let c = w::vbox(0);
            c.add_css_class("module");
            for (icon, t, s, on, tr) in [
                (
                    "network-wireless-signal-excellent-symbolic",
                    "Wonderland",
                    "Connected",
                    true,
                    Some("system-lock-screen-symbolic"),
                ),
                (
                    "bluetooth-active-symbolic",
                    "Magic Trackpad",
                    "",
                    false,
                    None,
                ),
                (
                    "audio-speakers-symbolic",
                    "Built-in Speakers",
                    "",
                    true,
                    Some("object-select-symbolic"),
                ),
            ] {
                c.append(&k::DeviceRow::new(icon, t, s, on, tr).button);
            }
            c.upcast()
        }),
        kit_case("detail-page", || {
            let p = k::DetailPage::new(
                "Bluetooth",
                Some(true),
                "Devices",
                Some("Bluetooth Settings…"),
            );
            p.set_rows(&[], "No paired devices");
            p.root.upcast()
        }),
        kit_case("slider-module", || {
            k::SliderModule::new(
                "Sound",
                "audio-volume-medium-symbolic",
                40.0,
                |_| {},
                Some("Output"),
            )
            .root
            .upcast()
        }),
        kit_case("now-playing", || {
            // Full card (widgets) with cover art missing: the player's app icon stands in; the
            // compact card (Control Center) with no player known: the note.
            let c = w::vbox(w::S2);
            let full = k::NowPlayingCard::new();
            full.set(&Fake.now_playing().unwrap());
            full.set_app_icon(Fake.now_playing_icon().as_deref());
            c.append(&full.root);
            let compact = k::NowPlayingCard::compact();
            compact.set(&Fake.now_playing().unwrap());
            c.append(&compact.root);
            c.upcast()
        }),
        kit_case("battery-tile", || {
            use shell::status::BatteryStatus as B;
            let r = w::hbox(w::S2);
            r.set_homogeneous(true);
            for (p, st, m, low) in [
                (82, B::Discharging, Some(191), false),
                (18, B::Discharging, Some(40), true),
                (7, B::Discharging, Some(12), false),
                (54, B::Charging, Some(80), false),
            ] {
                let t = k::BatteryTile::new();
                t.set(p, st, m, low);
                r.append(&t.button);
            }
            r.upcast()
        }),
    ]
}

fn render(win: &gtk::Window) -> gdk::Texture {
    let (wd, ht) = (win.width(), win.height());
    let p = gtk::WidgetPaintable::new(Some(win));
    let mut node = None;
    for _ in 0..20 {
        let snap = gtk::Snapshot::new();
        p.snapshot(&snap, wd as f64, ht as f64);
        node = snap.to_node();
        if node.is_some() {
            break;
        }
        win.queue_draw();
        pump(100);
    }
    let node =
        node.unwrap_or_else(|| panic!("render node: window {wd}x{ht} mapped={}", win.is_mapped()));
    let r = gtk::gsk::CairoRenderer::new();
    r.realize(None::<&gdk::Surface>).expect("cairo renderer");
    let t = r.render_texture(
        node,
        Some(&gtk::graphene::Rect::new(0.0, 0.0, wd as f32, ht as f32)),
    );
    r.unrealize();
    t
}

/// Text that must meet WCAG AA (4.5:1) against what is behind it, measured from the render.
fn contrast_classes(case: &str) -> &'static [&'static str] {
    if case.starts_with("lock") || case.starts_with("login") || case.starts_with("setup") {
        &[
            "lock-field",
            "lock-message",
            "lock-name",
            "lock-caption",
            "lock-date",
        ]
    } else if case.starts_with("controlcenter")
        || case.starts_with("kit-")
        || case.starts_with("approval")
    {
        &["secondary", "title"]
    } else {
        &[]
    }
}

fn find_class(w: &gtk::Widget, class: &str, out: &mut Vec<gtk::Widget>) {
    if w.is_drawable() && w.has_css_class(class) {
        out.push(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        find_class(&ch, class, out);
        c = ch.next_sibling();
    }
}

fn luminance(p: &[u8]) -> f64 {
    let ch = |v: u8| {
        let c = v as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(p[0]) + 0.7152 * ch(p[1]) + 0.0722 * ch(p[2])
}

/// Contrast between the widget's background (median pixel) and its most extreme pixel (glyph core).
fn contrast(px: &(i32, i32, Vec<u8>), r: &gtk::graphene::Rect) -> f64 {
    let (w, h, ref data) = *px;
    let (x0, y0) = (r.x().max(0.0) as i32, r.y().max(0.0) as i32);
    let (x1, y1) = (
        ((r.x() + r.width()) as i32).min(w),
        ((r.y() + r.height()) as i32).min(h),
    );
    let mut l: Vec<f64> = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * w + x) as usize * 4))
        .map(|i| luminance(&data[i..i + 3]))
        .collect();
    if l.is_empty() {
        return 21.0;
    }
    l.sort_by(f64::total_cmp);
    let (lo, mid, hi) = (l[0], l[l.len() / 2], l[l.len() - 1]);
    ((hi + 0.05) / (mid + 0.05)).max((mid + 0.05) / (lo + 0.05))
}

fn contrast_issues(win: &gtk::Window, tex: &gdk::Texture, classes: &[&str]) -> Vec<String> {
    let px = pixels(tex);
    let mut out = Vec::new();
    for class in classes {
        let mut found = Vec::new();
        find_class(win.upcast_ref(), class, &mut found);
        for wd in found {
            if wd
                .downcast_ref::<gtk::Label>()
                .is_some_and(|l| l.label().is_empty())
            {
                continue;
            }
            let Some(r) = wd.compute_bounds(win) else {
                continue;
            };
            let c = contrast(&px, &r);
            if c < 4.5 {
                out.push(format!("{class}: contrast {c:.2}:1 < 4.5:1"));
            }
        }
    }
    out
}

fn pixels(t: &gdk::Texture) -> (i32, i32, Vec<u8>) {
    let mut d = gdk::TextureDownloader::new(t);
    d.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = d.download_bytes();
    let (w, h) = (t.width(), t.height());
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&bytes[y * stride..y * stride + w as usize * 4]);
    }
    (w, h, out)
}

/// Share of differing pixels, or 1.0 on size mismatch.
fn diff(a: &gdk::Texture, b: &gdk::Texture) -> f64 {
    let ((aw, ah, pa), (bw, bh, pb)) = (pixels(a), pixels(b));
    if (aw, ah) != (bw, bh) {
        return 1.0;
    }
    let bad = pa
        .chunks(4)
        .zip(pb.chunks(4))
        .filter(|(x, y)| x.iter().zip(*y).any(|(p, q)| p.abs_diff(*q) > 24))
        .count();
    bad as f64 / (aw * ah) as f64
}

thread_local! {
    static WARNINGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn main() {
    let n = 40 + std::process::id() % 200;
    let display = format!(":{n}");
    let port = (18000 + n).to_string();
    /// Kills the private broadwayd on every exit path (panics included).
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let broadway = Kill(
        std::process::Command::new("gtk4-broadwayd")
            .args([display.as_str(), "--port", port.as_str()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("gtk4-broadwayd (GTK 4 package) is required"),
    );
    let sock = Path::new(&std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
        .join(format!("broadway{}.socket", n + 1));
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let cfg = std::env::temp_dir().join(format!("wol-golden-{}", std::process::id()));
    unsafe {
        std::env::set_var("GDK_BACKEND", "broadway");
        std::env::set_var("BROADWAY_DISPLAY", &display);
        std::env::set_var("WOL_FAKE_NOW", "1");
        std::env::set_var("XDG_CONFIG_HOME", &cfg);
        std::env::set_var("GTK_THEME", "Adwaita");
    }
    // Any GLib/GTK warning or critical fails the case that produced it (style bugs, bad sizes).
    {
        glib::log_set_writer_func(|level, fields| {
            if matches!(
                level,
                glib::LogLevel::Warning | glib::LogLevel::Critical | glib::LogLevel::Error
            ) {
                let msg = fields
                    .iter()
                    .find(|f| f.key() == "MESSAGE")
                    .and_then(|f| f.value_str().map(str::to_owned))
                    .unwrap_or_default();
                WARNINGS.with_borrow_mut(|v| v.push(format!("{level:?}: {msg}")));
            }
            glib::log_writer_default(level, fields)
        });
    }
    gtk::init().expect("gtk init");
    // Product symbolic icons (installed into the WolOS theme on the image).
    gtk::IconTheme::for_display(&gdk::Display::default().unwrap()).add_search_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../design/icons/symbolic"
    ));
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden");
    std::fs::create_dir_all(&out).unwrap();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let update = std::env::var_os("WOL_GOLDEN_UPDATE").is_some();
    let only = std::env::var("WOL_GOLDEN_ONLY").ok();
    let mut failures = Vec::new();
    for variant in [Variant::Light, Variant::Dark] {
        let t = theme::resolve(
            Some(if variant == Variant::Light {
                "wol-light"
            } else {
                "wol-dark"
            }),
            false,
        );
        let extra = format!(
            "window.preview {{ background: url(\"file://{WALL}\") center / cover; }} window.preview-app {{ background: none; }}"
        );
        kit::style::apply(&t, &extra);
        for c in cases() {
            let name = format!(
                "{}-{}",
                c.name,
                if variant == Variant::Light {
                    "light"
                } else {
                    "dark"
                }
            );
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            let win = gtk::Window::builder()
                .decorated(false)
                .default_width(c.size.0)
                .default_height(c.size.1)
                .build();
            win.add_css_class(c.class);
            if !c.wallpaper && c.class == "preview-app" {
                win.add_css_class("app");
            }
            let child = (c.build)();
            // Broadway clamps windows to its 1024x768 screen; a sized frame holds the declared size.
            let frame = gtk::Overlay::new();
            frame.set_child(Some(&child));
            if c.size.1 > 1 {
                frame.set_size_request(c.size.0, c.size.1);
            }
            win.set_child(Some(&frame));
            win.present();
            pump(900);
            fn count(n: &shell::layout::Node) -> usize {
                1 + n.children.iter().map(count).sum::<usize>()
            }
            let nodes = win
                .child()
                .map(|c| count(&kit::check::tree(&c, win.upcast_ref(), "")))
                .unwrap_or(0);
            assert!(nodes > 3, "{name}: layout tree has {nodes} nodes");
            let mut issues: Vec<String> = kit::check::issues(&win)
                .into_iter()
                .map(|i| i.message)
                .collect();
            if c.size.1 > 1 && (win.width() > c.size.0 || win.height() > c.size.1) {
                issues.push(format!(
                    "content needs {}x{}, surface is {}x{}",
                    win.width(),
                    win.height(),
                    c.size.0,
                    c.size.1
                ));
            }
            let tex = render(&win);
            // A golden must be the size it claims (height 1 = natural height).
            if (tex.width(), tex.height()) != c.size && (c.size.1 > 1 || tex.width() != c.size.0) {
                issues.push(format!(
                    "rendered {}x{}, declared {}x{}",
                    tex.width(),
                    tex.height(),
                    c.size.0,
                    c.size.1
                ));
            }
            issues.extend(WARNINGS.with_borrow_mut(std::mem::take));
            issues.extend(contrast_issues(&win, &tex, contrast_classes(&c.name)));
            let png = out.join(format!("{name}.png"));
            tex.save_to_png(&png).unwrap();
            let gold = golden.join(format!("{name}.png"));
            if update {
                std::fs::copy(&png, &gold).unwrap();
            } else if let Ok(g) = gdk::Texture::from_filename(&gold) {
                let d = diff(&tex, &g);
                if d > TOLERANCE {
                    issues.push(format!(
                        "differs from golden by {:.2}% of pixels",
                        d * 100.0
                    ));
                }
            } else {
                issues.push("no golden (run with WOL_GOLDEN_UPDATE=1)".into());
            }
            for i in &issues {
                eprintln!("{name}: {i}");
            }
            if !issues.is_empty() {
                failures.push(name.clone());
            }
            println!(
                "{name}: {} ({nodes} widgets checked)",
                if issues.is_empty() { "ok" } else { "FAIL" }
            );
            win.destroy();
        }
    }
    drop(broadway);
    let _ = std::fs::remove_dir_all(cfg);
    println!("renders: {}", out.display());
    if !failures.is_empty() {
        eprintln!("golden failures: {failures:?}");
        std::process::exit(1);
    }
}
