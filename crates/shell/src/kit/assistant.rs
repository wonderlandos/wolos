//! Assistant pieces (Setup Assistant, installer; ADR 0013): full-screen pages that cross-fade, one
//! fixed-size glass card per question over the lock backdrop, the welcome hero, choice lists,
//! avatar tones, problem lines, Return/Escape. Surfaces in `ui/` build from these only.
use super::widgets::{self as w, S2, S3, S4, S6};
use gtk::{glib, prelude::*};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Card size: every question page has it, so Back/Continue never move between pages.
pub const CARD: (i32, i32) = (600, 540);
/// List rows the card shows at once (whole rows; the rest scroll).
pub const ROWS: i32 = 5;
pub const ROW: i32 = 36;
/// Header art on a card (illustrations; symbolic icons use `w::ICON_L`).
pub const ART: i32 = 96;
/// Smallest stage art on a moment card (it grows into the room the text leaves).
pub const ART_L: i32 = 160;
/// Account avatar and the tone dots under it.
pub const AVATAR: i32 = 64;
pub const TONE: i32 = 20;
/// Form label column on the account card.
pub const LABEL_CHARS: i32 = 12;
/// Welcome title column: never wider than the calm left part of the art.
pub const HERO_COLUMN: i32 = 520;
/// Cross-fade between pages (same as `--glass-motion`).
pub const PAGE_MS: u32 = super::reveal::REVEAL_MS;

/// Installed illustrations (`image/build.sh`), else the repo design tree (dev, tests).
pub const ILLUSTRATION_DIRS: [&str; 2] = [
    "/usr/local/share/wolos/illustrations",
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../design/illustrations/png"
    ),
];

/// PNG for illustration `name`, the @2x one when present (crisp on HiDPI, scaled down otherwise).
pub fn illustration_file(name: &str) -> Option<PathBuf> {
    ILLUSTRATION_DIRS.iter().find_map(|d| {
        ["@2x", "@1x"]
            .iter()
            .map(|s| Path::new(d).join(format!("{name}{s}.png")))
            .find(|p| p.exists())
    })
}

/// Illustration in a `size` square (the art is square, drawn from the @2x file so it stays crisp
/// on HiDPI); the WolOS mark when the art is not installed.
pub fn illustration(name: &str, size: i32) -> gtk::Widget {
    let i = match illustration_file(name) {
        Some(p) => {
            let i = gtk::Image::from_file(&p);
            i.set_pixel_size(size);
            i.add_css_class("illustration");
            i
        }
        None => w::icon("wolos-mark", size),
    };
    i.set_halign(gtk::Align::Center);
    i.upcast()
}

/// Where the hero image goes in a `(w, h)` area: scaled to cover, anchored bottom-right (the
/// characters stand there; the sky on the top-left gives way first). (scale, x, y).
pub fn cover_anchored(img: (f64, f64), area: (f64, f64)) -> (f64, f64, f64) {
    let s = (area.0 / img.0).max(area.1 / img.1);
    (s, area.0 - img.0 * s, area.1 - img.1 * s)
}

/// Full-bleed art (scale-to-cover, anchored bottom-right); a dusk gradient when not installed.
/// Where stage art goes in a `(w, h)` area: scaled to fit (never past the file's own pixels),
/// centred. (scale, x, y).
pub fn contain_centred(img: (f64, f64), area: (f64, f64)) -> (f64, f64, f64) {
    let s = (area.0 / img.0).min(area.1 / img.1).min(1.0);
    (s, (area.0 - img.0 * s) / 2.0, (area.1 - img.1 * s) / 2.0)
}

/// Illustration `name` as a cairo surface (drawing areas paint it at any scale).
fn surface(name: &str) -> Option<gtk::cairo::ImageSurface> {
    let t = gtk::gdk::Texture::from_filename(illustration_file(name)?).ok()?;
    let (wd, ht) = (t.width(), t.height());
    let stride = gtk::cairo::Format::ARgb32
        .stride_for_width(wd as u32)
        .ok()?;
    let mut data = vec![0u8; stride as usize * ht as usize];
    t.download(&mut data, stride as usize);
    gtk::cairo::ImageSurface::create_for_data(data, gtk::cairo::Format::ARgb32, wd, ht, stride).ok()
}

/// Where an image of size `.0` goes in an area of size `.1`: (scale, x, y).
type Place = fn((f64, f64), (f64, f64)) -> (f64, f64, f64);

/// Paints `img` at `place(img size, area size)`.
fn paint_with(area: &gtk::DrawingArea, img: gtk::cairo::ImageSurface, place: Place) {
    area.set_draw_func(move |_, cr, wd, ht| {
        let (iw, ih) = (img.width() as f64, img.height() as f64);
        let (s, x, y) = place((iw, ih), (wd as f64, ht as f64));
        cr.translate(x, y);
        cr.scale(s, s);
        if cr.set_source_surface(&img, 0.0, 0.0).is_ok() {
            cr.source().set_filter(gtk::cairo::Filter::Good);
            let _ = cr.paint();
        }
    });
}

/// Moment-card art: fills the room the text leaves (at least `ART_L` tall), fitted and centred,
/// so the card's insets stay equal and nothing floats; the WolOS mark when not installed.
pub fn stage(name: &str) -> gtk::Widget {
    let Some(img) = surface(name) else {
        return w::icon("wolos-mark", ART_L).upcast();
    };
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .content_height(ART_L)
        .build();
    area.add_css_class("assistant-stage");
    paint_with(&area, img, contain_centred);
    area.upcast()
}

pub fn hero(name: &str) -> gtk::Widget {
    let surface = surface(name);
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    area.add_css_class("hero-art");
    match surface {
        Some(img) => paint_with(&area, img, cover_anchored),
        None => area.add_css_class("hero-fallback"),
    }
    area.upcast()
}

/// Welcome: `art` full-bleed, title (two lines) and subtitle on its calm left, then `action`.
pub fn welcome(art: &str, title: &str, subtitle: &str, action: &gtk::Button) -> gtk::Widget {
    let o = gtk::Overlay::new();
    o.add_css_class("hero");
    o.set_child(Some(&hero(art)));
    let col = w::vbox(S6);
    col.add_css_class("hero-column");
    col.set_valign(gtk::Align::Center);
    col.set_halign(gtk::Align::Start);
    col.set_width_request(HERO_COLUMN);
    let words = w::vbox(S3);
    let t = w::label(title, "hero-title");
    t.set_wrap(true);
    words.append(&t);
    words.append(&w::label(subtitle, "hero-subtitle"));
    col.append(&words);
    action.set_halign(gtk::Align::Start);
    action.add_css_class("hero-action");
    col.append(action);
    o.add_overlay(&col);
    o.upcast()
}

/// Pages that cross-fade; each fills the screen (cards centre themselves).
pub fn pager() -> gtk::Stack {
    let s = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(PAGE_MS)
        .hexpand(true)
        .vexpand(true)
        .build();
    s.add_css_class("assistant-pager");
    s
}

/// Top of a card.
pub enum Art<'a> {
    Icon(&'a str),
    Illustration(&'a str),
    Widget(gtk::Widget),
}

/// One question: art, title, subtitle, `body`, then Back · status · Continue at the foot.
pub struct Card {
    pub root: gtk::Box,
    pub back: gtk::Button,
    pub next: gtk::Button,
    /// Problem or progress line at the foot (hidden when empty).
    pub status: gtk::Label,
    pub subtitle: gtk::Label,
    pub busy: gtk::Spinner,
    foot: gtk::Box,
}

impl Card {
    /// Question card: art, title and subtitle on top, `body` filling the middle.
    pub fn new(art: Art, title: &str, subtitle: &str, body: &impl IsA<gtk::Widget>) -> Card {
        let art: gtk::Widget = match art {
            Art::Icon(name) => {
                let i = w::icon(name, w::ICON_L);
                i.add_css_class("assistant-icon");
                i.upcast()
            }
            Art::Illustration(name) => illustration(name, ART),
            Art::Widget(wd) => wd,
        };
        art.set_halign(gtk::Align::Center);
        Card::build(&art, title, subtitle, body, false)
    }

    /// Moment card (done, installing, failed): illustration `art` large (`stage`), then title,
    /// subtitle and `body` above the foot.
    pub fn moment(art: &str, title: &str, subtitle: &str, body: &impl IsA<gtk::Widget>) -> Card {
        Card::build(&stage(art), title, subtitle, body, true)
    }

    fn build(
        art: &gtk::Widget,
        title: &str,
        subtitle: &str,
        body: &impl IsA<gtk::Widget>,
        moment: bool,
    ) -> Card {
        let c = w::glass_panel(S6);
        c.add_css_class("assistant-card");
        c.set_size_request(CARD.0, CARD.1);
        c.set_halign(gtk::Align::Center);
        c.set_valign(gtk::Align::Center);
        let head = w::vbox(S2);
        head.append(art);
        head.append(&w::centered(title, "large-title"));
        let sub = w::centered(subtitle, "assistant-subtitle");
        wraps(&sub);
        head.append(&sub);
        let b = w::vbox(0);
        b.add_css_class("assistant-body");
        b.append(body);
        // Question: the body takes the spare room. Moment: the stage art does.
        b.set_vexpand(!moment);
        c.append(&head);
        c.append(&b);
        let foot = w::hbox(S2);
        foot.add_css_class("assistant-foot");
        let back = w::button("Back", w::Kind::Default, || {});
        foot.append(&back);
        let status = w::label("", "assistant-status");
        wraps(&status);
        status.set_xalign(1.0);
        status.set_visible(false);
        let gap = w::spacer();
        foot.append(&gap);
        foot.append(&status);
        let busy = gtk::Spinner::new();
        busy.add_css_class("assistant-busy");
        busy.set_visible(false);
        foot.append(&busy);
        let next = w::button("Continue", w::Kind::Primary, || {});
        foot.append(&next);
        c.append(&foot);
        Card {
            root: c,
            back,
            next,
            status,
            subtitle: sub,
            busy,
            foot,
        }
    }

    /// No foot at all (a page that only shows progress).
    pub fn without_foot(&self) {
        self.foot.set_visible(false);
    }

    /// Extra button left of Continue ("Not Now").
    pub fn add_action(&self, b: &gtk::Button) {
        self.foot.insert_child_after(b, Some(&self.busy));
    }

    /// Problem (`problem` = true) or progress line at the foot; empty hides it.
    pub fn say(&self, text: &str, problem: bool) {
        self.status.set_label(text);
        self.status.set_visible(!text.is_empty());
        if problem {
            self.status.add_css_class("problem");
        } else {
            self.status.remove_css_class("problem");
        }
    }

    /// Working: spinner on, buttons off (and back).
    pub fn set_busy(&self, on: bool) {
        self.busy.set_visible(on);
        self.busy.set_spinning(on);
        self.next.set_sensitive(!on);
        self.back.set_sensitive(!on);
    }
}

/// One list row: optional leading icon, title, optional detail on the right.
pub struct Choice {
    pub icon: Option<String>,
    pub title: String,
    pub detail: String,
    /// Trailing symbolic icon (e.g. the lock of a secured network).
    pub trail: Option<String>,
}

/// One-choice list showing [`ROWS`] whole rows; a check marks the chosen one.
pub struct ChoiceList {
    pub root: gtk::ScrolledWindow,
    checks: Rc<Vec<gtk::Image>>,
}

fn check_only(checks: &[gtk::Image], i: usize) {
    for (j, c) in checks.iter().enumerate() {
        c.set_opacity(if i == j { 1.0 } else { 0.0 });
    }
}

impl ChoiceList {
    pub fn new(
        rows: Vec<Choice>,
        selected: Option<usize>,
        picked: impl Fn(usize) + 'static,
    ) -> Self {
        let list = w::vbox(0);
        list.set_valign(gtk::Align::Start);
        let n = rows.len() as i32;
        let mut checks = Vec::new();
        let mut buttons = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            let row = w::hbox(S3);
            row.add_css_class("list-row");
            row.add_css_class("assistant-row");
            if let Some(ic) = &r.icon {
                let i = w::icon(ic, w::ICON);
                i.add_css_class("dim-icon");
                row.append(&i);
            }
            let t = w::label(&r.title, "");
            t.set_hexpand(true);
            row.append(&t);
            if !r.detail.is_empty() {
                row.append(&w::label(&r.detail, "secondary"));
            }
            if let Some(ic) = &r.trail {
                let i = w::icon(ic, w::ICON);
                i.add_css_class("dim-icon");
                row.append(&i);
            }
            let check = w::icon("object-select-symbolic", w::ICON);
            check.add_css_class("accent-icon");
            check.set_opacity(if Some(i) == selected { 1.0 } else { 0.0 });
            row.append(&check);
            checks.push(check);
            let b = gtk::Button::builder().child(&row).build();
            b.add_css_class("row-button");
            b.add_css_class("assistant-choice");
            list.append(&b);
            buttons.push(b);
        }
        let h = ROW * n.clamp(1, ROWS);
        let root = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(h)
            .max_content_height(h)
            .build();
        root.add_css_class("assistant-list");
        root.add_css_class("whole-rows");
        let checks = Rc::new(checks);
        let picked = Rc::new(picked);
        for (i, b) in buttons.iter().enumerate() {
            let (cs, picked) = (checks.clone(), picked.clone());
            b.connect_clicked(move |_| {
                check_only(&cs, i);
                picked(i);
            });
        }
        ChoiceList { root, checks }
    }

    pub fn select(&self, i: usize) {
        check_only(&self.checks, i);
    }
}

/// Labelled form row: `title` on the left, `field` filling the rest.
pub fn form_row(title: &str, field: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = w::hbox(S3);
    r.add_css_class("assistant-form-row");
    let l = w::label(title, "assistant-form-label");
    l.set_width_chars(LABEL_CHARS);
    l.set_xalign(1.0);
    r.append(&l);
    field.set_hexpand(true);
    r.append(field);
    r
}

/// Problem line under a form row, aligned with the fields (hidden when empty).
pub fn problem_line() -> gtk::Label {
    let p = w::label("", "assistant-problem");
    p.set_wrap(true);
    p.set_visible(false);
    p
}

/// Shows or hides a [`problem_line`].
pub fn set_problem(p: &gtk::Label, text: &str) {
    p.set_label(text);
    p.set_visible(!text.is_empty());
}

/// Avatar CSS class for a tone (`AVATAR_TONES`), e.g. `avatar-tone-green`.
pub fn tone_class(tone: &str) -> String {
    format!("avatar-tone-{tone}")
}

/// Paints `avatar` in `tone` (removing any other tone).
pub fn set_tone(avatar: &impl IsA<gtk::Widget>, tone: &str) {
    for t in shared::login::AVATAR_TONES {
        avatar.remove_css_class(&tone_class(t));
    }
    avatar.add_css_class(&tone_class(tone));
}

/// Monogram avatar with tone dots under it; `picked` gets the tone index.
pub struct AvatarPicker {
    pub root: gtk::Box,
    pub avatar: gtk::Label,
}

impl AvatarPicker {
    pub fn new(selected: usize, picked: impl Fn(usize) + 'static) -> AvatarPicker {
        let tones = shared::login::AVATAR_TONES;
        let root = w::vbox(S2);
        root.set_halign(gtk::Align::Center);
        let avatar = w::avatar("", AVATAR);
        avatar.add_css_class("assistant-avatar");
        set_tone(&avatar, tones[selected.min(tones.len() - 1)]);
        root.append(&avatar);
        let dots = w::hbox(S2);
        dots.set_halign(gtk::Align::Center);
        let picked = Rc::new(picked);
        let mut first: Option<gtk::ToggleButton> = None;
        for (i, t) in tones.iter().enumerate() {
            let dot = w::vbox(0);
            dot.set_size_request(TONE, TONE);
            dot.add_css_class("tone-dot");
            dot.add_css_class(&tone_class(t));
            let b = gtk::ToggleButton::builder()
                .child(&dot)
                .active(i == selected)
                .tooltip_text(*t)
                .build();
            b.add_css_class("tone-button");
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let (av, picked, t) = (avatar.clone(), picked.clone(), *t);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    set_tone(&av, t);
                    picked(i);
                }
            });
            dots.append(&b);
        }
        root.append(&dots);
        AvatarPicker { root, avatar }
    }
}

/// A row of selectable preview tiles (appearance, looks); one stays chosen.
pub fn tile_row(tiles: &[gtk::ToggleButton]) -> gtk::Box {
    let r = w::hbox(S4);
    r.set_halign(gtk::Align::Center);
    for (i, t) in tiles.iter().enumerate() {
        if i > 0 {
            t.set_group(Some(&tiles[0]));
        }
        r.append(t);
    }
    r
}

/// Empty card body (the page says it all in its title and subtitle).
pub fn blank() -> gtk::Box {
    w::vbox(0)
}

/// Work in progress in place of a list: spinner and one line ("Looking for networks…").
pub fn waiting(text: &str) -> gtk::Box {
    let b = w::vbox(S3);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_vexpand(true);
    b.add_css_class("assistant-waiting");
    let s = w::spinner();
    s.set_vexpand(false);
    b.append(&s);
    b.append(&w::centered(text, "assistant-subtitle"));
    b
}

/// Section caption inside a card body.
pub fn caption(text: &str) -> gtk::Label {
    let l = w::label(text, "assistant-caption");
    l.set_halign(gtk::Align::Center);
    l
}

/// Warning box: icon and one or two sentences (the installer's erase warning); the label to
/// change them later.
pub fn warning(text: &str) -> (gtk::Box, gtk::Label) {
    let b = w::hbox(S3);
    b.add_css_class("assistant-warning");
    let i = w::icon("dialog-warning-symbolic", w::ICON_TOOL);
    i.add_css_class("warning-icon");
    b.append(&i);
    let l = w::label(text, "");
    wraps(&l);
    b.append(&l);
    (b, l)
}

/// Wraps at the width the card gives; long text never widens the fixed-size card.
fn wraps(l: &gtk::Label) {
    l.set_wrap(true);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
}

/// Progress: bar and a line under it (what happens, time left).
pub struct Progress {
    pub root: gtk::Box,
    pub bar: gtk::ProgressBar,
    pub detail: gtk::Label,
}

impl Progress {
    pub fn new() -> Progress {
        let root = w::vbox(S2);
        root.add_css_class("assistant-progress");
        let bar = w::progress();
        root.append(&bar);
        let detail = w::centered("", "assistant-subtitle");
        root.append(&detail);
        Progress { root, bar, detail }
    }

    pub fn set(&self, fraction: f64, detail: &str) {
        self.bar.set_fraction(fraction.clamp(0.0, 1.0));
        self.detail.set_label(detail);
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

/// Return continues and Escape goes back, wherever the focus is (captured before fields see them).
pub fn keys(win: &impl IsA<gtk::Widget>, enter: impl Fn() + 'static, escape: impl Fn() + 'static) {
    let k = gtk::EventControllerKey::new();
    k.set_propagation_phase(gtk::PropagationPhase::Capture);
    k.connect_key_pressed(move |_, key, _, _| {
        use gtk::gdk::Key;
        if key == Key::Return || key == Key::KP_Enter || key == Key::ISO_Enter {
            enter();
            glib::Propagation::Stop
        } else if key == Key::Escape {
            escape();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    win.add_controller(k);
}

/// Runs a full-screen assistant: `build` gets the window (black, `lock-window`) once the app starts.
pub fn run(
    app_id: &str,
    snapshot: &'static str,
    build: impl Fn(&gtk::ApplicationWindow) + 'static,
) -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(app_id).build();
    app.connect_activate(move |app| {
        super::style::install("");
        let win = gtk::ApplicationWindow::new(app);
        win.add_css_class("lock-window");
        build(&win);
        super::snapshot_on_map(&win, snapshot);
        win.fullscreen();
        win.present();
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_covers_and_keeps_the_bottom_right() {
        // 960x640 art on 16:10: width decides, the top gives way.
        let (s, x, y) = cover_anchored((960.0, 640.0), (1280.0, 800.0));
        assert!((s - 1280.0 / 960.0).abs() < 1e-9);
        assert_eq!(x, 0.0);
        assert!(y < 0.0 && (y + 640.0 * s - 800.0).abs() < 1e-9);
        // 4:3: height decides, the left gives way, the right edge stays.
        let (s, x, y) = cover_anchored((960.0, 640.0), (1024.0, 768.0));
        assert!((s - 1.2).abs() < 1e-9);
        assert!(x < 0.0 && (x + 960.0 * s - 1024.0).abs() < 1e-9);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn stage_art_fits_centred_and_never_blows_up() {
        // Room wider than tall: height decides, centred across.
        assert_eq!(
            contain_centred((512.0, 512.0), (534.0, 300.0)),
            (300.0 / 512.0, 117.0, 0.0)
        );
        // More room than pixels: drawn 1:1 in the middle.
        assert_eq!(
            contain_centred((320.0, 320.0), (534.0, 400.0)),
            (1.0, 107.0, 40.0)
        );
    }

    #[test]
    fn illustrations_resolve_to_png() {
        let p = illustration_file("setup-welcome").expect("welcome art in the design tree");
        assert!(p.to_string_lossy().ends_with("setup-welcome@2x.png"));
        assert!(illustration_file("no-such-art").is_none());
    }
}
