//! Desktop widget goldens (ADR 0010): every built-in at every size, a few empty states, the
//! example custom package, the error and consent cards, and Edit Widgets with its gallery.
use super::Case;
use gtk::prelude::*;
use shared::widgets::{Builtin, Desktop, Face, Kind, Placed, Size};
use shell::kit::data as d;
use shell::kit::widgets as w;
use shell::ui::widgets::{Board, Content};
use shell::ui::widgets_builtin::{self as b, Data, WeatherView};
use shell::ui::widgets_layout as lay;
use std::path::PathBuf;

/// The example package shipped with the system (`desktop/overlay/.../widgets/game-friends`).
pub fn example_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../desktop/overlay/usr/local/share/wolos/widgets/game-friends"
    ))
}

fn example() -> wol_core::widgets::Package {
    wol_core::widgets::load_package(&example_dir(), true)
}

/// A card centred on the wallpaper with one grid margin around it.
fn card_case(name: String, size: Size, build: Box<dyn Fn() -> gtk::Box>) -> Case {
    let (wd, ht) = d::card_size(size);
    Case {
        name,
        size: (wd + 2 * w::S6, ht + 2 * w::S6),
        class: "preview",
        wallpaper: true,
        build: Box::new(move || {
            let c = build();
            c.set_halign(gtk::Align::Center);
            c.set_valign(gtk::Align::Center);
            c.upcast()
        }),
    }
}

fn custom_card(size: Size) -> gtk::Box {
    let p = example();
    let m = p.manifest.as_ref().expect("example package is valid");
    let data = m.sample.clone().unwrap_or_default();
    let none = PathBuf::new();
    let cx = lay::Ctx {
        dir: &p.dir,
        cache: &none,
        interactive: true,
    };
    lay::card(m, size, &data, &cx)
}

fn placed(id: &str, kind: Kind, size: Size, col: u16, row: u16) -> Placed {
    Placed {
        id: id.into(),
        kind,
        size,
        col,
        row,
        options: Default::default(),
    }
}

/// A desktop with one of each kind, as a user might arrange it.
fn board(edit: bool) -> gtk::Widget {
    let mut desk = Desktop::default();
    let bi = |x| Kind::Builtin(x);
    desk.widgets = vec![
        placed("clock", bi(Builtin::Clock), Size::Small, 0, 0),
        placed("calendar", bi(Builtin::Calendar), Size::Small, 1, 0),
        placed("weather", bi(Builtin::Weather), Size::Medium, 0, 1),
        placed("music", bi(Builtin::Music), Size::Medium, 0, 2),
        placed(
            "game-friends",
            Kind::Custom("game-friends".into()),
            Size::Medium,
            0,
            3,
        ),
    ];
    let board = Board::new(desk, None, vec![example()]);
    for (id, x) in [
        ("clock", Builtin::Clock),
        ("calendar", Builtin::Calendar),
        ("weather", Builtin::Weather),
        ("music", Builtin::Music),
    ] {
        board.set_content(id, Content::Builtin(b::sample(x)));
    }
    let p = example();
    let m = p.manifest.clone().expect("example package is valid");
    board.set_content(
        "game-friends",
        Content::Custom {
            data: m.sample.clone().unwrap_or_default(),
            manifest: std::rc::Rc::new(m),
            dir: p.dir.clone(),
        },
    );
    if edit {
        board.set_edit(true);
    }
    let root = board.root.clone();
    // The board lives as long as its widgets.
    root.connect_destroy(move |_| {
        let _ = &board;
    });
    root.upcast()
}

/// Clicks the first button under `w` (Show Details).
fn press_first_button(w: &gtk::Widget) -> bool {
    let mut c = w.first_child();
    while let Some(x) = c {
        if let Some(b) = x.downcast_ref::<gtk::Button>() {
            b.emit_clicked();
            return true;
        }
        if press_first_button(&x) {
            return true;
        }
        c = x.next_sibling();
    }
    false
}

pub fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    for x in Builtin::ALL {
        for s in Size::ALL {
            v.push(card_case(
                format!("widget-{}-{}", x.id(), s.id()),
                s,
                Box::new(move || b::card(&b::sample(x), s, &b::Hooks::default())),
            ));
        }
    }
    let states: Vec<(&str, Size, Data)> = vec![
        (
            "clock-digital",
            Size::Small,
            Data::Clock {
                now: b::sample_now(),
                face: Face::Digital,
            },
        ),
        (
            "clock-digital",
            Size::Medium,
            Data::Clock {
                now: b::sample_now(),
                face: Face::Digital,
            },
        ),
        (
            "weather-noplace",
            Size::Small,
            Data::Weather(WeatherView::NoPlace),
        ),
        (
            "weather-offline",
            Size::Medium,
            Data::Weather(WeatherView::Offline(b::sample_place())),
        ),
        ("music-idle", Size::Medium, Data::Music(None)),
        (
            "calendar-noevents",
            Size::Medium,
            Data::Calendar {
                now: b::sample_now(),
                events: Some(Vec::new()),
            },
        ),
    ];
    for (tag, s, data) in states {
        v.push(card_case(
            format!("widget-{tag}-{}", s.id()),
            s,
            Box::new(move || b::card(&data, s, &b::Hooks::default())),
        ));
    }
    for s in Size::ALL {
        v.push(card_case(
            format!("widget-custom-{}", s.id()),
            s,
            Box::new(move || custom_card(s)),
        ));
    }
    let broken = r#"{"name": "Stocks", "sizes": ["small", "huge"],
        "layouts": {"small": {"type": "stak", "children": []}}}"#;
    for s in [Size::Small, Size::Medium] {
        v.push(card_case(
            format!("widget-error-{}", s.id()),
            s,
            Box::new(move || {
                let problems = shared::widgets::manifest::Manifest::parse("stocks", broken)
                    .expect_err("broken manifest");
                lay::error_card(
                    s,
                    "Stocks",
                    &shell::copy::widget_problems("Stocks"),
                    &lay::problem_lines(&problems),
                )
            }),
        ));
    }
    // Show Details pressed: the developer's lines, scrollable.
    v.push(card_case(
        "widget-error-details-medium".into(),
        Size::Medium,
        Box::new(move || {
            let problems = shared::widgets::manifest::Manifest::parse("stocks", broken)
                .expect_err("broken manifest");
            let c = lay::error_card(
                Size::Medium,
                "Stocks",
                &shell::copy::widget_problems("Stocks"),
                &lay::problem_lines(&problems),
            );
            assert!(press_first_button(c.upcast_ref()), "Show Details");
            c
        }),
    ));
    // A source that exited with an error: a sentence, never the exit code.
    v.push(card_case(
        "widget-source-error-small".into(),
        Size::Small,
        Box::new(|| {
            let e = wol_core::widgets::SourceError::Failed {
                code: Some(3),
                stderr: "no token".into(),
            };
            lay::error_card(
                Size::Small,
                "Game Friends",
                &shell::copy::widget_source(&e, "Game Friends"),
                &[e.message()],
            )
        }),
    ));
    v.push(card_case(
        "widget-consent-medium".into(),
        Size::Medium,
        Box::new(|| lay::consent_card(Size::Medium, "Game Friends", "api.example.com", || {})),
    ));
    v.push(Case {
        name: "widgets-desktop".into(),
        size: (1280, 800),
        class: "preview",
        wallpaper: true,
        build: Box::new(|| board(false)),
    });
    v.push(Case {
        name: "widgets-edit".into(),
        size: (1280, 800),
        class: "preview",
        wallpaper: true,
        build: Box::new(|| board(true)),
    });
    v
}
