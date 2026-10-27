//! Effects & Looks: a live picture of the desktop, Looks (presets that set every effect at once:
//! shipped ones, imported ones, or Custom once you change anything), and every effect on its own:
//! corners, spacing, blur and glass, window opacity, borders with gradients, shadows, dimming and
//! motion. Changes apply live through one `hyprctl` batch and are journaled (Restore undoes them).
use super::*;
use kit::pickers::{LOOKS_HERO, LooksPreview, PresetCard, Tokens};
use wol_core::looks::{self, Cost, Looks, Motion, Preset, Switch, range};

const MOTIONS: [(Motion, &str); 3] = [
    (Motion::Zoom, "Zoom"),
    (Motion::Pop, "Pop"),
    (Motion::Slide, "Slide"),
];
const SWITCHES: [(Switch, &str); 3] = [
    (Switch::Slide, "Slide"),
    (Switch::Vertical, "Vertical"),
    (Switch::Fade, "Fade"),
];
/// Gradient angles offered (degrees).
const ANGLES: [(u16, &str); 4] = [
    (0, "Across"),
    (45, "Diagonal"),
    (90, "Down"),
    (135, "Other diagonal"),
];

/// Shipped Looks in a source checkout (development and goldens), when the system dir is absent.
const SOURCE_LOOKS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../desktop/overlay/usr/local/share/wolos/looks"
);

fn config_home() -> PathBuf {
    PathBuf::from(kit::config_home())
}

/// Where Looks come from: the shipped dir (or the checkout's copy), then the user's own.
fn preset_dirs() -> Vec<(PathBuf, bool)> {
    let mut dirs = looks::dirs(&config_home());
    if !std::path::Path::new(looks::SYSTEM_DIR).is_dir() {
        dirs[0].0 = PathBuf::from(SOURCE_LOOKS);
    }
    dirs
}

/// Integrated graphics (shares memory with the processor): Intel, or an AMD APU's "Graphics".
pub(crate) fn integrated_gpu(hw: &super::machine::HardwareSnapshot) -> bool {
    hw.devices
        .iter()
        .filter(|d| d.kind == super::machine::Kind::Gpu)
        .any(|d| {
            d.id.starts_with("8086:")
                || d.name.starts_with("Intel")
                || (d.name.contains("Radeon") && d.name.ends_with("Graphics"))
        })
}

/// Colours a border may take: the accents of this appearance, plus white and clear.
fn swatches() -> Vec<(String, String)> {
    let dark = kit::style::active_id()
        .as_deref()
        .is_some_and(|id| id.contains("dark"));
    let mut v: Vec<(String, String)> = accent::ACCENTS
        .iter()
        .map(|(_, name, light, dk)| (name.to_string(), if dark { dk } else { light }.to_string()))
        .collect();
    v.push(("White".into(), shared::theme::Color::WHITE.to_hex()));
    v.push(("Clear".into(), looks::CLEAR.into()));
    v
}

/// The pane's live state: presets, the picture, the gallery cards and the controls.
struct Fx {
    env: Rc<Env>,
    status: gtk::Label,
    presets: RefCell<Vec<Preset>>,
    hero: LooksPreview,
    gallery: gtk::FlowBox,
    cards: RefCell<Vec<(String, gtk::ToggleButton, LooksPreview)>>,
    custom: RefCell<Option<(gtk::ToggleButton, LooksPreview)>>,
    controls: gtk::Box,
    hint: gtk::Label,
    igpu: Cell<bool>,
    /// Set while cards are checked from code (no apply).
    syncing: Cell<bool>,
}

impl Fx {
    fn looks(&self) -> Looks {
        self.env.desk.get().looks
    }

    /// One change from a control: saved, applied live, the picture and the gallery follow.
    fn change(self: &Rc<Self>, f: impl FnOnce(&mut Looks)) {
        self.env.desk.set(&self.status, |p| {
            f(&mut p.looks);
            p.looks = p.looks.normalized();
        });
        self.after();
    }

    /// A whole look at once (preset, import, Restore Defaults): controls are rebuilt to match.
    fn apply(self: &Rc<Self>, l: &Looks) {
        let l = l.normalized();
        self.env.desk.set(&self.status, |p| p.looks = l.clone());
        self.after();
        self.build_controls();
    }

    fn after(&self) {
        let l = self.looks();
        self.hero.set(&l);
        self.sync_gallery();
        self.hint
            .set_visible(self.igpu.get() && l.cost() == Cost::Heavy);
    }

    /// Checks the card whose look is on screen, or Custom.
    fn sync_gallery(&self) {
        let l = self.looks();
        let at = looks::matching(&self.presets.borrow(), &l).map(|p| p.id.clone());
        self.syncing.set(true);
        for (id, b, _) in self.cards.borrow().iter() {
            b.set_active(at.as_deref() == Some(id.as_str()));
        }
        if let Some((b, pv)) = &*self.custom.borrow() {
            pv.set(&l);
            if let Some(child) = b.parent() {
                child.set_visible(at.is_none());
            }
            b.set_active(at.is_none());
        }
        self.syncing.set(false);
    }

    fn fill_gallery(self: &Rc<Self>) {
        while let Some(c) = self.gallery.first_child() {
            self.gallery.remove(&c);
        }
        self.cards.borrow_mut().clear();
        let tokens = Tokens::current();
        let cur = self.looks();
        let mut first: Option<gtk::ToggleButton> = None;
        let join = |b: &gtk::ToggleButton, first: &mut Option<gtk::ToggleButton>| {
            if let Some(f) = first {
                b.set_group(Some(f));
            } else {
                *first = Some(b.clone());
            }
        };
        let custom = PresetCard::new("Custom", "Your own mix of effects.", &cur, tokens, false);
        join(&custom.button, &mut first);
        self.gallery.insert(&custom.button, -1);
        *self.custom.borrow_mut() = Some((custom.button.clone(), custom.preview.clone()));
        for p in self.presets.borrow().iter() {
            let c = PresetCard::new(&p.name, &p.description, &p.looks, tokens, false);
            join(&c.button, &mut first);
            let (fx, l) = (Rc::downgrade(self), p.looks.clone());
            c.button.connect_toggled(move |b| {
                if let Some(fx) = fx.upgrade()
                    && b.is_active()
                    && !fx.syncing.get()
                {
                    fx.apply(&l);
                }
            });
            self.gallery.insert(&c.button, -1);
            self.cards
                .borrow_mut()
                .push((p.id.clone(), c.button.clone(), c.preview.clone()));
        }
        self.sync_gallery();
    }

    fn reload_presets(self: &Rc<Self>) {
        *self.presets.borrow_mut() = looks::load(&preset_dirs());
        self.fill_gallery();
    }

    fn build_controls(self: &Rc<Self>) {
        clear(&self.controls);
        let l = self.looks();
        let fx = self;
        let slider = |title: &str,
                      (lo, hi): (f64, f64),
                      step: f64,
                      value: f64,
                      ends: (&str, &str),
                      set: fn(&mut Looks, f64)| {
            let f = Rc::downgrade(fx);
            form::slider_row(title, (lo, hi, step), value.clamp(lo, hi), ends, move |v| {
                if let Some(f) = f.upgrade() {
                    f.change(|l| set(l, v))
                }
            })
        };
        let toggle = |title: &str,
                      sub: &str,
                      on: bool,
                      set: fn(&mut Looks, bool),
                      shows: Vec<gtk::Widget>| {
            for x in &shows {
                x.set_visible(on);
            }
            let f = Rc::downgrade(fx);
            form::toggle_row(title, sub, on, move |v| {
                for x in &shows {
                    x.set_visible(v);
                }
                if let Some(f) = f.upgrade() {
                    f.change(|l| set(l, v))
                }
            })
        };
        let u = |r: (u8, u8)| (f64::from(r.0), f64::from(r.1));
        let fl = |r: (f32, f32)| (f64::from(r.0), f64::from(r.1));

        // Windows: shape and spacing.
        self.controls.append(&w::section(
            "Windows",
            &[
                slider(
                    "Corner radius",
                    u(range::ROUNDING),
                    2.0,
                    l.rounding.into(),
                    ("Square", "Round"),
                    |l, v| l.rounding = v as u8,
                ),
                slider(
                    "Space around windows",
                    u(range::GAPS),
                    4.0,
                    l.gaps.into(),
                    ("Narrow", "Wide"),
                    |l, v| l.gaps = v as u8,
                ),
            ],
        ));

        // Glass: blur behind windows and panels, and how see-through things are.
        let strength = slider(
            "Blur strength",
            u(range::BLUR_SIZE),
            1.0,
            l.blur_size.into(),
            ("Subtle", "Strong"),
            |l, v| l.blur_size = v as u8,
        );
        let quality = slider(
            "Blur quality",
            u(range::BLUR_PASSES),
            1.0,
            l.blur_passes.into(),
            ("Faster", "Smoother"),
            |l, v| l.blur_passes = v as u8,
        );
        let vibrancy = slider(
            "Color behind glass",
            (0.0, 1.0),
            0.1,
            l.vibrancy.into(),
            ("Muted", "Vivid"),
            |l, v| l.vibrancy = v as f32,
        );
        let blur = toggle(
            "Blur",
            "Frosted glass behind windows, menus and the Dock.",
            l.blur,
            |l, v| l.blur = v,
            vec![strength.clone(), quality.clone(), vibrancy.clone()],
        );
        self.controls.append(&w::section(
            "Glass",
            &[
                blur,
                strength,
                quality,
                vibrancy,
                slider(
                    "Menu bar, Dock and Control Center",
                    fl(range::GLASS),
                    0.05,
                    l.glass.into(),
                    ("Clear", "Solid"),
                    |l, v| l.glass = v as f32,
                ),
                slider(
                    "Active window",
                    fl(range::OPACITY),
                    0.05,
                    l.active_opacity.into(),
                    ("See-through", "Solid"),
                    |l, v| l.active_opacity = v as f32,
                ),
                slider(
                    "Other windows",
                    fl(range::OPACITY),
                    0.05,
                    l.inactive_opacity.into(),
                    ("See-through", "Solid"),
                    |l, v| l.inactive_opacity = v as f32,
                ),
            ],
        ));

        // Borders: width, a colour or a gradient of up to three, the angle, and other windows' border.
        let tokens = Tokens::current();
        let wells = w::hbox(w::S2);
        wells.set_valign(gtk::Align::Center);
        self.fill_wells(&wells, tokens);
        let colors = row_sub(
            "Active window",
            "One color, or up to three for a gradient.",
            &wells,
        );
        let angle_at = ANGLES
            .iter()
            .position(|a| a.0 == l.border_angle)
            .unwrap_or(1);
        let names: Vec<&str> = ANGLES.iter().map(|a| a.1).collect();
        let f = Rc::downgrade(fx);
        let angle = form::picker_row("Gradient direction", "", &names, angle_at, move |i| {
            if let Some(f) = f.upgrade() {
                f.change(|l| l.border_angle = ANGLES[i].0)
            }
        });
        angle.set_visible(l.border_colors.len() > 1);
        let f = Rc::downgrade(fx);
        let inactive =
            kit::pickers::ColorWell::new(&l.inactive_border, &swatches(), tokens, move |c| {
                if let Some(f) = f.upgrade() {
                    f.change(|l| l.inactive_border = c)
                }
            });
        let other = row("Other windows", &inactive.button);
        let width = {
            let f = Rc::downgrade(fx);
            let shows = [colors.clone(), angle.clone(), other.clone()];
            for x in &shows {
                x.set_visible(l.border_size > 0);
            }
            let multi = l.border_colors.len() > 1;
            let a = angle.clone();
            form::slider_row(
                "Border width",
                (f64::from(range::BORDER.0), f64::from(range::BORDER.1), 1.0),
                f64::from(l.border_size),
                ("None", "Thick"),
                move |v| {
                    for x in &shows {
                        x.set_visible(v > 0.0);
                    }
                    a.set_visible(v > 0.0 && multi);
                    if let Some(f) = f.upgrade() {
                        f.change(|l| l.border_size = v as u8)
                    }
                },
            )
        };
        angle.set_visible(l.border_size > 0 && l.border_colors.len() > 1);
        self.controls
            .append(&w::section("Borders", &[width, colors, angle, other]));

        // Depth: shadows and dimming.
        let softness = slider(
            "Shadow size",
            u(range::SHADOW),
            5.0,
            l.shadow_range.into(),
            ("Tight", "Soft"),
            |l, v| l.shadow_range = v as u8,
        );
        let dim = slider(
            "Dimming",
            fl(range::DIM),
            0.05,
            l.dim_strength.into(),
            ("Light", "Strong"),
            |l, v| l.dim_strength = v as f32,
        );
        self.controls.append(&w::section(
            "Depth",
            &[
                toggle(
                    "Shadows",
                    "Windows float above the desktop.",
                    l.shadow,
                    |l, v| l.shadow = v,
                    vec![softness.clone()],
                ),
                softness,
                toggle(
                    "Dim other windows",
                    "The window you work in stands out.",
                    l.dim_inactive,
                    |l, v| l.dim_inactive = v,
                    vec![dim.clone()],
                ),
                dim,
            ],
        ));

        // Motion.
        let f = Rc::downgrade(fx);
        let style = form::segmented_row(
            "Windows open with",
            "",
            &MOTIONS.map(|m| m.1),
            MOTIONS.iter().position(|m| m.0 == l.motion).unwrap_or(0),
            move |i| {
                if let Some(f) = f.upgrade() {
                    f.change(|l| l.motion = MOTIONS[i].0)
                }
            },
        );
        let f = Rc::downgrade(fx);
        let switch = form::segmented_row(
            "Desktops change with",
            "",
            &SWITCHES.map(|m| m.1),
            SWITCHES.iter().position(|m| m.0 == l.switch).unwrap_or(0),
            move |i| {
                if let Some(f) = f.upgrade() {
                    f.change(|l| l.switch = SWITCHES[i].0)
                }
            },
        );
        let speed = slider(
            "Speed",
            fl(range::SPEED),
            0.25,
            l.speed.into(),
            ("Slow", "Fast"),
            |l, v| l.speed = v as f32,
        );
        self.controls.append(&w::section(
            "Motion",
            &[
                toggle(
                    "Animations",
                    "",
                    l.animations,
                    |l, v| l.animations = v,
                    vec![style.clone(), switch.clone(), speed.clone()],
                ),
                style,
                switch,
                speed,
            ],
        ));
        let d = self.env.desk.get();
        if d.reduce_motion || d.reduce_transparency {
            let what = match (d.reduce_transparency, d.reduce_motion) {
                (true, true) => {
                    "Reduce transparency and Reduce motion are on: blur and animations stay off."
                }
                (true, false) => {
                    "Reduce transparency is on: blur stays off and windows stay solid."
                }
                _ => "Reduce motion is on: animations stay off.",
            };
            let a11y = form::nav_row(
                "preferences-desktop-accessibility-symbolic",
                "Accessibility",
                what,
            );
            let open = self.env.opener("Accessibility");
            a11y.connect_clicked(move |_| open());
            self.controls.append(&w::section("", &[a11y.upcast()]));
        }
    }

    /// Border colour wells: one per gradient stop, plus add and remove.
    fn fill_wells(self: &Rc<Self>, wells: &gtk::Box, tokens: Tokens) {
        clear(wells);
        let colors = self.looks().border_colors;
        for (i, c) in colors.iter().enumerate() {
            let f = Rc::downgrade(self);
            let well = kit::pickers::ColorWell::new(c, &swatches(), tokens, move |hex| {
                if let Some(f) = f.upgrade() {
                    f.change(|l| {
                        if let Some(x) = l.border_colors.get_mut(i) {
                            *x = hex
                        }
                    })
                }
            });
            wells.append(&well.button);
        }
        if colors.len() > 1 {
            let f = Rc::downgrade(self);
            wells.append(&w::icon_button(
                "list-remove-symbolic",
                "Fewer colors",
                move || {
                    if let Some(f) = f.upgrade() {
                        f.change(|l| {
                            l.border_colors.pop();
                        });
                        f.build_controls();
                    }
                },
            ));
        }
        if colors.len() < range::BORDER_COLORS {
            let f = Rc::downgrade(self);
            wells.append(&w::icon_button(
                "list-add-symbolic",
                "Add a gradient color",
                move || {
                    if let Some(f) = f.upgrade() {
                        f.change(|l| {
                            let next = l.border_colors.last().cloned().unwrap_or_default();
                            l.border_colors.push(next);
                        });
                        f.build_controls();
                    }
                },
            ));
        }
    }
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Effects & Looks");
    let status = status_label();
    let tokens = Tokens::current();
    let hero = LooksPreview::new(LOOKS_HERO, &e.desk.get().looks, tokens);
    let picture = w::vbox(0);
    picture.add_css_class("list-row");
    picture.add_css_class("preview-row");
    picture.append(&hero.root);
    p.append(&w::section("", &[picture.upcast()]));
    let hint = w::footnote(
        "These effects ask a lot of built-in graphics. Less blur or fewer shadows keep windows smooth on this computer.",
    );
    hint.add_css_class("perf-hint");
    hint.set_visible(false);
    p.append(&hint);
    let gallery = w::wrap_row(w::S3, 3);
    let fx = Rc::new(Fx {
        env: e.clone(),
        status: status.clone(),
        presets: RefCell::new(Vec::new()),
        hero,
        gallery: gallery.clone(),
        cards: RefCell::new(Vec::new()),
        custom: RefCell::new(None),
        controls: w::vbox(w::S6),
        hint,
        igpu: Cell::new(false),
        syncing: Cell::new(false),
    });
    fx.reload_presets();
    p.append(&w::section("Looks", &[gallery.clone().upcast()]));
    if let Some(card) = gallery.parent() {
        card.add_css_class("preview-card");
    }
    let actions = w::hbox(w::S2);
    actions.set_halign(gtk::Align::End);
    {
        let f = Rc::downgrade(&fx);
        let import = w::button("Import…", w::Kind::Default, || {});
        import.connect_clicked(move |b| {
            let f = f.clone();
            kit::pickers::choose_file(b, "Import a Look", None, move |file| {
                let Some(f) = f.upgrade() else { return };
                match looks::import(&config_home(), &file) {
                    Ok(p) => {
                        f.reload_presets();
                        f.apply(&p.looks);
                    }
                    Err(x) => say(&f.status, &x.to_string()),
                }
            });
        });
        actions.append(&import);
    }
    {
        let f = Rc::downgrade(&fx);
        let export = w::button("Export…", w::Kind::Default, || {});
        export.connect_clicked(move |b| {
            let Some(fx) = f.upgrade() else { return };
            let l = fx.looks();
            let name = looks::matching(&fx.presets.borrow(), &l)
                .map_or("My Look".to_string(), |p| p.name.clone());
            let f = f.clone();
            kit::pickers::choose_file(
                b,
                "Export This Look",
                Some(&format!("{name}.json")),
                move |file| {
                    let Some(f) = f.upgrade() else { return };
                    let r = looks::export(&file, &name, &f.looks());
                    say(
                        &f.status,
                        &r.err().map(|x| x.to_string()).unwrap_or_default(),
                    );
                },
            );
        });
        actions.append(&export);
    }
    {
        let f = Rc::downgrade(&fx);
        actions.append(&w::button(
            "Restore Defaults",
            w::Kind::Default,
            move || {
                if let Some(f) = f.upgrade() {
                    f.apply(&Looks::default())
                }
            },
        ));
    }
    p.append(&actions);
    fx.build_controls();
    p.append(&fx.controls);
    p.append(&status);
    // Built-in graphics: a quiet hint when the look is heavy.
    let (m, f) = (e.machine(), Rc::downgrade(&fx));
    off_thread(
        move || integrated_gpu(&m.hardware()),
        move |igpu| {
            if let Some(f) = f.upgrade() {
                f.igpu.set(igpu);
                f.after();
            }
        },
    );
    p
}

#[cfg(test)]
mod tests {
    use super::super::machine::{Device, HardwareSnapshot, Kind};
    use super::*;

    #[test]
    fn integrated_graphics_is_recognised() {
        let gpu = |id: &str, name: &str| HardwareSnapshot {
            devices: vec![Device {
                kind: Kind::Gpu,
                id: id.into(),
                name: name.into(),
                ..Default::default()
            }],
        };
        assert!(integrated_gpu(&gpu("8086:46a6", "Intel Iris Xe Graphics")));
        assert!(integrated_gpu(&gpu(
            "1002:15bf",
            "AMD Radeon 780M Graphics"
        )));
        assert!(!integrated_gpu(&gpu(
            "10de:2684",
            "NVIDIA GeForce RTX 4090"
        )));
        assert!(!integrated_gpu(&gpu("1002:744c", "AMD Radeon RX 7900 XTX")));
        assert!(!integrated_gpu(&HardwareSnapshot::default()));
    }

    #[test]
    fn choices_cover_every_motion() {
        assert_eq!(MOTIONS.len(), 3);
        assert_eq!(SWITCHES.len(), 3);
        assert!(ANGLES.iter().any(|a| a.0 == Looks::default().border_angle));
    }
}
