//! IaraKit component set (HIG, archicture/modules/hig.md). The ONLY place widget metrics live:
//! surfaces pass tokens from this module, never numbers.
use gtk::{glib, prelude::*};
use std::rc::Rc;

/// Spacing scale (HIG §2).
pub const S1: i32 = 4;
pub const S2: i32 = 8;
pub const S3: i32 = 12;
pub const S4: i32 = 16;
pub const S6: i32 = 24;
pub const S8: i32 = 32;
/// Padding of every glass panel and card, equal on all four sides (kit.css `--inset`).
pub const INSET: i32 = S3;
/// Icon sizes (HIG §2): rows/toolbar, file and app tiles.
pub const ICON: i32 = 16;
/// Toolbar glyphs (screenshot bar).
pub const ICON_TOOL: i32 = 18;
pub const ICON_M: i32 = 32;
pub const ICON_L: i32 = 48;
pub const ICON_XL: i32 = 64;
/// Shell metrics.
pub const DOCK_TILE: i32 = 56;
pub const CONTROL_CENTER_WIDTH: i32 = 320;
pub const SPOTLIGHT_WIDTH: i32 = 680;
pub const PANEL_TOP: i32 = 160;
pub const SIDEBAR_WIDTH: i32 = 220;
/// Default app window and the narrow size every app must fit (HIG §5).
pub const APP_SIZE: (i32, i32) = (900, 600);
pub const APP_NARROW: (i32, i32) = (480, 360);
/// Below this window width the sidebar collapses.
pub const NARROW: i32 = 640;
pub const LOCK_FIELD_WIDTH: i32 = 200;
pub const LOCK_FIELD_HEIGHT: i32 = 28;
/// Setup assistant: card width, list row height, rows shown before scrolling.
pub const SETUP_CARD_WIDTH: i32 = 560;
pub const SETUP_ROW: i32 = 36;
pub const SETUP_ROWS: i32 = 6;
/// Caption width under icon tiles (switcher).
pub const TILE_CHARS: i32 = 14;
/// Mission Control desktop thumbnail and window title line.
pub const OVERVIEW_DESKTOP: (i32, i32) = (192, 108);
pub const OVERVIEW_TITLE: i32 = 28;
/// Screenshot thumbnail (bottom right after a capture).
pub const CAPTURE_THUMB: (i32, i32) = (200, 125);
/// About <App> panel width.
pub const ABOUT_WIDTH: i32 = 280;
/// Force Quit Applications panel: width, list height before it scrolls (about six apps).
pub const FORCE_QUIT: (i32, i32) = (360, 240);

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

/// Text styles: "" body 13, "title" 13 semibold, "secondary" 11 dim, "large-title" 22 semibold.
pub fn label(text: &str, style: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_valign(gtk::Align::Center);
    if !style.is_empty() {
        l.add_css_class(style);
    }
    l
}

pub fn centered(text: &str, style: &str) -> gtk::Label {
    let l = label(text, style);
    l.set_xalign(0.5);
    l.set_justify(gtk::Justification::Center);
    l
}

pub fn spacer() -> gtk::Box {
    let b = hbox(0);
    b.set_hexpand(true);
    b
}

pub fn icon(name: &str, size: i32) -> gtk::Image {
    let i = gtk::Image::from_icon_name(name);
    i.set_pixel_size(size);
    i.set_valign(gtk::Align::Center);
    i
}

thread_local! {
    /// Rendered app icons by (file, pixel size): each SVG is rasterised once per process.
    static APP_ICONS: std::cell::RefCell<std::collections::HashMap<(std::path::PathBuf, i32), gtk::gdk::Texture>> =
        Default::default();
}

/// Full-colour app icon rendered by librsvg (gdk-pixbuf) off the main thread at 2x for HiDPI,
/// then cached. GTK's own SVG path drops filters (shadows) to boxes, so app tiles never go through it.
pub fn app_icon(name: &str, size: i32) -> gtk::Image {
    match super::icon_file(name) {
        Some(path) => svg_icon(path, size),
        None => {
            let img = gtk::Image::new();
            img.set_pixel_size(size);
            img.set_icon_name(Some(name));
            img
        }
    }
}

/// `app_icon` for a known SVG file.
fn svg_icon(path: std::path::PathBuf, size: i32) -> gtk::Image {
    let img = gtk::Image::new();
    img.set_pixel_size(size);
    let px = size * 2;
    let key = (path, px);
    if let Some(t) = APP_ICONS.with_borrow(|c| c.get(&key).cloned()) {
        img.set_paintable(Some(&t));
        return img;
    }
    let i2 = img.clone();
    glib::spawn_future_local(async move {
        let p = key.0.clone();
        let r = gtk::gio::spawn_blocking(move || {
            gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&p, px, px, true)
                .ok()
                .map(|pb| gtk::gdk::Texture::for_pixbuf(&pb))
        })
        .await;
        if let Ok(Some(t)) = r {
            i2.set_paintable(Some(&t));
            APP_ICONS.with_borrow_mut(|c| c.insert(key, t));
        }
    });
    img
}

// ---- Buttons and controls ----

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Default,
    Primary,
    Destructive,
    Plain,
}

pub fn button(text: &str, kind: Kind, clicked: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("btn");
    b.add_css_class(match kind {
        Kind::Default => "btn-default",
        Kind::Primary => "btn-primary",
        Kind::Destructive => "btn-destructive",
        Kind::Plain => "btn-plain",
    });
    b.connect_clicked(move |_| clicked());
    b
}

/// Icon-only button: needs a tooltip (HIG §1).
pub fn icon_button(name: &str, tooltip: &str, clicked: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(&icon(name, ICON))
        .tooltip_text(tooltip)
        .build();
    b.add_css_class("icon-button");
    b.connect_clicked(move |_| clicked());
    b
}

pub fn toggle(on: bool, changed: impl Fn(bool) + 'static) -> gtk::Switch {
    let s = gtk::Switch::builder()
        .active(on)
        .valign(gtk::Align::Center)
        .build();
    s.add_css_class("toggle");
    s.connect_active_notify(move |s| changed(s.is_active()));
    s
}

pub fn slider(value: f64, changed: impl Fn(f64) + 'static) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    s.set_value(value);
    s.set_draw_value(false);
    s.set_hexpand(true);
    s.add_css_class("slider");
    s.connect_value_changed(move |s| changed(s.value()));
    s
}

pub fn dropdown(options: &[&str], selected: u32) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(options);
    d.set_selected(selected);
    d.set_valign(gtk::Align::Center);
    d.add_css_class("dropdown");
    d
}

pub fn segmented(options: &[&str], selected: usize, picked: impl Fn(usize) + 'static) -> gtk::Box {
    let row = hbox(0);
    row.add_css_class("segmented");
    row.set_valign(gtk::Align::Center);
    let picked = Rc::new(picked);
    let buttons: Vec<gtk::ToggleButton> = options
        .iter()
        .map(|o| gtk::ToggleButton::with_label(o))
        .collect();
    for (i, b) in buttons.iter().enumerate() {
        if i > 0 {
            b.set_group(Some(&buttons[0]));
        }
        b.set_active(i == selected);
        let picked = picked.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                picked(i)
            }
        });
        row.append(b);
    }
    row
}

pub fn search_field(placeholder: &str) -> gtk::SearchEntry {
    let s = gtk::SearchEntry::builder()
        .placeholder_text(placeholder)
        .build();
    s.add_css_class("search-field");
    s
}

pub fn password_field(placeholder: &str) -> gtk::PasswordEntry {
    let p = gtk::PasswordEntry::builder()
        .placeholder_text(placeholder)
        .build();
    p.add_css_class("text-field");
    p
}

pub fn spinner() -> gtk::Spinner {
    let s = gtk::Spinner::builder()
        .spinning(true)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    s.set_size_request(ICON_M, ICON_M);
    s
}

pub fn empty_state(
    icon_name: &str,
    title: &str,
    hint: &str,
    action: Option<gtk::Button>,
) -> gtk::Box {
    let b = vbox(S2);
    b.add_css_class("empty-state");
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_vexpand(true);
    let i = icon(icon_name, ICON_XL);
    i.add_css_class("dim-icon");
    b.append(&i);
    b.append(&centered(title, "large-title"));
    let h = centered(hint, "secondary");
    h.set_wrap(true);
    b.append(&h);
    if let Some(a) = action {
        a.set_halign(gtk::Align::Center);
        b.append(&a);
    }
    b
}

/// Context/app menu: (label, action). "" = separator.
pub type MenuItem<'a> = (&'a str, Box<dyn Fn()>);
/// Menu row with an optional shortcut shown right-aligned and dimmed. "" = separator.
pub type MenuRow = (String, Option<&'static str>, Box<dyn Fn()>);

pub fn menu(items: Vec<MenuItem>) -> gtk::Popover {
    menu_keys(
        items
            .into_iter()
            .map(|(t, a)| (t.to_string(), None, a))
            .collect(),
    )
}

pub fn menu_keys(rows: Vec<MenuRow>) -> gtk::Popover {
    let p = gtk::Popover::new();
    p.add_css_class("menu");
    p.set_has_arrow(false);
    let b = vbox(0);
    for (text, key, act) in rows {
        if text.is_empty() {
            b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            continue;
        }
        let row = hbox(S6);
        row.append(&label(&text, ""));
        if let Some(k) = key {
            row.append(&spacer());
            row.append(&label(k, "menu-key"));
        }
        let it = gtk::Button::builder().child(&row).build();
        it.add_css_class("menu-item");
        let p2 = p.clone();
        it.connect_clicked(move |_| {
            p2.popdown();
            act();
        });
        b.append(&it);
    }
    p.set_child(Some(&b));
    p
}

/// Menu bar title with a drop-down menu (app name, Window).
pub fn menubar_menu(child: &impl IsA<gtk::Widget>) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_child(Some(child));
    b.set_valign(gtk::Align::Center);
    b
}

// ---- Lists ----

/// Card list: rows separated by hairlines (Settings-style).
pub fn section(title: &str, rows: &[gtk::Widget]) -> gtk::Box {
    let b = vbox(S2);
    if !title.is_empty() {
        b.append(&label(title, "title"));
    }
    let card = vbox(0);
    card.add_css_class("card-list");
    for r in rows {
        card.append(r);
    }
    b.append(&card);
    b
}

/// Row: optional 16px icon, title (+ secondary subtitle), trailing control right-aligned.
pub fn list_row(
    icon_name: &str,
    title: &str,
    subtitle: &str,
    trailing: Option<&gtk::Widget>,
) -> gtk::Box {
    let row = hbox(S3);
    row.add_css_class("list-row");
    if !icon_name.is_empty() {
        row.append(&icon(icon_name, ICON));
    }
    let t = vbox(0);
    t.set_valign(gtk::Align::Center);
    t.set_hexpand(true);
    // Titles wrap instead of forcing the window wider (narrow layouts).
    let tl = label(title, "");
    tl.set_wrap(true);
    tl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    t.append(&tl);
    if !subtitle.is_empty() {
        t.append(&label(subtitle, "secondary"));
    }
    row.append(&t);
    if let Some(w) = trailing {
        w.set_valign(gtk::Align::Center);
        row.append(w);
    }
    row
}

/// Clickable row (lists of choices, results).
pub fn list_button(icon_name: &str, title: &str, subtitle: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(&list_row(icon_name, title, subtitle, None))
        .build();
    b.add_css_class("row-button");
    b
}

// ---- Sheets ----

pub fn sheet(child: &impl IsA<gtk::Widget>) -> gtk::Revealer {
    let card = vbox(S3);
    card.add_css_class("sheet");
    card.append(child);
    let r = gtk::Revealer::builder()
        .child(&card)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .visible(false)
        .build();
    // Collapsed, the sheet leaves the layout: a visible 0 px child still took its box's spacing
    // (Control Center card sat 8 px lower than the panel's side insets).
    r.connect_reveal_child_notify(|r| {
        if r.reveals_child() {
            r.set_visible(true)
        }
    });
    r.connect_child_revealed_notify(|r| {
        if !r.is_child_revealed() && !r.reveals_child() {
            r.set_visible(false)
        }
    });
    r
}

/// Admin authorization sheet (IaraOS ADR 0026).
pub struct AuthSheet {
    pub revealer: gtk::Revealer,
    pub icon: gtk::Image,
    /// "Authenticate"; helper approvals put the request here (wraps).
    pub title: gtk::Label,
    /// Who asks and why (helper approvals name the helper and the change).
    pub why: gtk::Label,
    pub field: gtk::PasswordEntry,
    pub cancel: gtk::Button,
    /// Hidden unless the caller offers a lasting yes (helper approvals: Always Allow).
    pub always: gtk::Button,
    pub allow: gtk::Button,
    pub message: gtk::Label,
}

pub fn authenticate_sheet(who: &str) -> AuthSheet {
    let b = vbox(S2);
    let head = hbox(S3);
    let icon = icon("system-lock-screen-symbolic", ICON_M);
    head.append(&icon);
    let t = vbox(0);
    t.set_hexpand(true);
    let title = label("Authenticate", "title");
    title.set_wrap(true);
    t.append(&title);
    let why = label(
        &format!("{who} is trying to make changes. Enter your password to allow this."),
        "secondary",
    );
    why.set_wrap(true);
    t.append(&why);
    head.append(&t);
    b.append(&head);
    let field = password_field("Password");
    b.append(&field);
    let message = label("", "secondary");
    b.append(&message);
    let row = hbox(S2);
    row.append(&spacer());
    let cancel = button("Cancel", Kind::Default, || {});
    let always = button("Always Allow", Kind::Default, || {});
    always.set_visible(false);
    let allow = button("Allow", Kind::Primary, || {});
    row.append(&cancel);
    row.append(&always);
    row.append(&allow);
    b.append(&row);
    AuthSheet {
        revealer: sheet(&b),
        icon,
        title,
        why,
        field,
        cancel,
        always,
        allow,
        message,
    }
}

// ---- App scaffold ----

/// HIG §1 AppWindow: 52px glass toolbar (traffic lights, title, actions, search), 220px sidebar, content, status bar.
pub struct AppWindow {
    pub root: gtk::Box,
    pub actions: gtk::Box,
    pub sidebar: gtk::ListBox,
    pub content: gtk::Stack,
    pub status: gtk::Label,
    pub title: gtk::Label,
}

impl AppWindow {
    pub fn new(title: &str, with_sidebar: bool, with_status: bool) -> Self {
        let root = vbox(0);
        root.add_css_class("app-window");
        let bar = hbox(S2);
        bar.add_css_class("toolbar");
        let lights = hbox(S2);
        lights.add_css_class("traffic-lights");
        for (c, tip) in [
            ("close", "Close"),
            ("minimize", "Minimize"),
            ("zoom", "Zoom"),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("light");
            b.add_css_class(c);
            b.set_tooltip_text(Some(tip));
            b.set_valign(gtk::Align::Center);
            if c == "close" {
                b.connect_clicked(|b| {
                    if let Some(w) = b.root().and_downcast::<gtk::Window>() {
                        w.close();
                    }
                });
            }
            if c == "minimize" {
                b.connect_clicked(|b| {
                    if let Some(w) = b.root().and_downcast::<gtk::Window>() {
                        w.minimize();
                    }
                });
            }
            lights.append(&b);
        }
        bar.append(&lights);
        let title_l = label(title, "title");
        title_l.add_css_class("toolbar-title");
        bar.append(&title_l);
        bar.append(&spacer());
        let actions = hbox(S2);
        bar.append(&actions);
        let handle = gtk::WindowHandle::builder().child(&bar).build();
        root.append(&handle);
        let body = hbox(0);
        body.set_vexpand(true);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("sidebar");
        sidebar.set_width_request(SIDEBAR_WIDTH);
        sidebar.set_vexpand(true);
        // Scrolls when the window is shorter than the page list (never forces the window taller).
        let side = gtk::ScrolledWindow::builder()
            .child(&sidebar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .build();
        side.add_css_class("sidebar-scroll");
        if with_sidebar {
            // Shown on realize when the window is wide enough; hidden first so a narrow
            // window's minimum size never includes it.
            side.set_visible(false);
            body.append(&side);
        }
        let content = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .hhomogeneous(false)
            .build();
        content.add_css_class("content");
        body.append(&content);
        root.append(&body);
        let status = label("", "secondary");
        status.add_css_class("status-bar");
        if with_status {
            root.append(&status);
        }
        // Narrow windows hide the sidebar (HIG: collapsible); content keeps the full width.
        if with_sidebar {
            let sb = side.clone();
            root.connect_realize(move |r| {
                let Some(win) = r.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let fit = |w: &gtk::Window, sb: &gtk::ScrolledWindow| {
                    let on = w.default_width() >= NARROW;
                    if sb.is_visible() != on {
                        sb.set_visible(on);
                    }
                };
                fit(&win, &sb);
                let sb = sb.clone();
                win.connect_default_width_notify(move |w| fit(w, &sb));
            });
        }
        AppWindow {
            root,
            actions,
            sidebar,
            content,
            status,
            title: title_l,
        }
    }

    /// Small-caps dim header in the sidebar (not selectable).
    pub fn source_section(&self, title: &str) {
        let r = gtk::ListBoxRow::builder()
            .child(&label(&title.to_uppercase(), "source-section"))
            .selectable(false)
            .activatable(false)
            .build();
        self.sidebar.append(&r);
    }

    /// 28px sidebar row with a 16px symbolic icon; `page` becomes the content.
    pub fn source_item(&self, icon_name: &str, title: &str, page: &impl IsA<gtk::Widget>) {
        let row = hbox(S2);
        row.append(&icon(icon_name, ICON));
        row.append(&label(title, ""));
        let r = gtk::ListBoxRow::builder().child(&row).name(title).build();
        r.add_css_class("source-item");
        self.sidebar.append(&r);
        let scroll = gtk::ScrolledWindow::builder()
            .child(page)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        self.content.add_named(&scroll, Some(title));
        let content = self.content.clone();
        if self.sidebar.selected_row().is_none() {
            self.sidebar.select_row(Some(&r));
            content.set_visible_child_name(title);
        }
        let st = self.content.clone();
        self.sidebar.connect_row_selected(move |_, sel| {
            if let Some(s) = sel
                && s.widget_name() != ""
            {
                st.set_visible_child_name(&s.widget_name());
            }
        });
    }

    pub fn select(&self, title: &str) {
        let mut i = 0;
        while let Some(r) = self.sidebar.row_at_index(i) {
            if r.widget_name().eq_ignore_ascii_case(title) {
                self.sidebar.select_row(Some(&r));
                return;
            }
            i += 1;
        }
    }
}

/// Settings page body: large padding column.
pub fn page(title: &str) -> gtk::Box {
    let p = vbox(S6);
    p.add_css_class("page");
    p.append(&label(title, "large-title"));
    p
}

// ---- Shell pieces ----

pub fn glass_panel(spacing: i32) -> gtk::Box {
    let b = vbox(spacing);
    b.add_css_class("glass-panel");
    b
}

/// Menu bar item: flat, hover pill.
pub fn menubar_item(child: &impl IsA<gtk::Widget>) -> gtk::Button {
    let b = gtk::Button::builder().child(child).build();
    b.add_css_class("menubar-item");
    b.set_valign(gtk::Align::Center);
    b
}

/// Dock tile: 56px app icon + running dot underneath.
pub struct DockTile {
    pub button: gtk::Button,
    dot: gtk::Box,
}

impl DockTile {
    pub fn new(icon_name: &str, name: &str, clicked: impl Fn() + 'static) -> Self {
        let col = vbox(0);
        let img = app_icon(icon_name, DOCK_TILE);
        col.append(&img);
        let dot = hbox(0);
        dot.add_css_class("running-dot");
        dot.set_halign(gtk::Align::Center);
        dot.set_opacity(0.0);
        col.append(&dot);
        let button = gtk::Button::builder()
            .child(&col)
            .tooltip_text(name)
            .build();
        button.add_css_class("dock-tile");
        button.connect_clicked(move |_| clicked());
        DockTile { button, dot }
    }
    pub fn set_running(&self, on: bool) {
        let o = if on { 1.0 } else { 0.0 };
        if self.dot.opacity() != o {
            self.dot.set_opacity(o);
        }
    }
}

/// Control Center toggle: 28px round icon (accent when on, grey when off, disabled when unavailable) + title/subtitle.
pub struct ControlTile {
    pub root: gtk::Box,
    pub circle: gtk::Button,
    pub detail: gtk::Button,
    subtitle: gtk::Label,
}

impl ControlTile {
    pub fn new(icon_name: &str, title: &str, subtitle: &str, state: Option<bool>) -> Self {
        let root = hbox(S2);
        root.add_css_class("control");
        let circle = gtk::Button::builder()
            .child(&icon(icon_name, ICON))
            .valign(gtk::Align::Center)
            .build();
        circle.add_css_class("control-circle");
        match state {
            Some(true) => circle.add_css_class("on"),
            None => circle.set_sensitive(false),
            _ => {}
        }
        root.append(&circle);
        let t = vbox(0);
        t.set_valign(gtk::Align::Center);
        t.append(&label(title, "title"));
        let sub = label(subtitle, "secondary");
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sub.set_max_width_chars(14);
        if !subtitle.is_empty() {
            t.append(&sub);
        }
        let detail = gtk::Button::builder().child(&t).hexpand(true).build();
        detail.add_css_class("control-detail");
        root.append(&detail);
        ControlTile {
            root,
            circle,
            detail,
            subtitle: sub,
        }
    }
    pub fn set_on(&self, on: bool) {
        if on {
            self.circle.add_css_class("on")
        } else {
            self.circle.remove_css_class("on")
        }
    }
    pub fn set_subtitle(&self, s: &str) {
        if self.subtitle.text() != s {
            self.subtitle.set_label(s);
        }
    }
}

/// Control Center module card.
pub fn module(child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = vbox(S2);
    b.add_css_class("module");
    b.append(child);
    b
}

/// macOS fat slider: title above, 24px pill track with the icon inside at the left.
pub fn control_slider(
    title: &str,
    icon_name: &str,
    value: f64,
    changed: impl Fn(f64) + 'static,
) -> gtk::Box {
    let b = vbox(S2);
    b.append(&label(title, "title"));
    let o = gtk::Overlay::new();
    let s = slider(value, changed);
    s.add_css_class("fat");
    o.set_child(Some(&s));
    let i = icon(icon_name, ICON);
    i.set_halign(gtk::Align::Start);
    i.add_css_class("fat-icon");
    i.set_can_target(false);
    o.add_overlay(&i);
    b.append(&o);
    module(&b)
}

/// Small square tile (Screenshot, Record): icon over a secondary label.
pub fn square_tile(
    icon_name: &str,
    title: &str,
    clicked: impl Fn(&gtk::Button) + 'static,
) -> gtk::Button {
    let c = vbox(S1);
    let i = icon(icon_name, ICON);
    i.set_halign(gtk::Align::Center);
    c.append(&i);
    c.append(&centered(title, "secondary"));
    let b = gtk::Button::builder().child(&c).hexpand(true).build();
    b.add_css_class("square-tile");
    b.connect_clicked(move |b| clicked(b));
    b
}

pub fn avatar(name: &str, size: i32) -> gtk::Label {
    let l = gtk::Label::new(Some(&crate::login::initials(name)));
    l.add_css_class("avatar");
    l.add_css_class(match size {
        96.. => "avatar-xl",
        48.. => "avatar-l",
        _ => "avatar-s",
    });
    // Fixed square, centred both ways: a stretched label draws a pill, not a circle.
    l.set_size_request(size, size);
    l.set_halign(gtk::Align::Center);
    l.set_valign(gtk::Align::Center);
    l
}
pub const AVATAR_L: i32 = 48;
pub const AVATAR_XL: i32 = 96;

/// Lock/login clock: big time + long date, ticking on minute boundaries.
pub fn clock_header() -> gtk::Box {
    let b = vbox(0);
    let (time, date) = (centered("", "lock-time"), centered("", "lock-date"));
    b.append(&time);
    b.append(&date);
    super::every_minute(move || {
        let (t, mut s) = (super::now(), String::with_capacity(32));
        crate::clock::lock_time(&mut s, t);
        time.set_label(&s);
        crate::clock::lock_date(&mut s, t);
        date.set_label(&s);
    });
    b
}

/// Avatar, name, glass password pill with a submit arrow, message line.
pub struct UserPanel {
    pub root: gtk::Box,
    pub name: gtk::Label,
    pub pill: gtk::Box,
    pub field: gtk::PasswordEntry,
    pub submit: gtk::Button,
    pub message: gtk::Label,
}

impl UserPanel {
    pub fn new(name: &str) -> Self {
        let root = vbox(S2);
        root.set_halign(gtk::Align::Center);
        root.append(&avatar(name, AVATAR_XL));
        let n = centered(name, "lock-name");
        root.append(&n);
        let pill = hbox(0);
        pill.add_css_class("lock-pill");
        pill.set_halign(gtk::Align::Center);
        pill.set_size_request(LOCK_FIELD_WIDTH, LOCK_FIELD_HEIGHT);
        let field = gtk::PasswordEntry::builder()
            .placeholder_text("Enter Password")
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        field.add_css_class("lock-field");
        field.set_width_chars(1);
        pill.append(&field);
        let submit = gtk::Button::builder()
            .label("→")
            .tooltip_text("Unlock")
            .valign(gtk::Align::Center)
            .sensitive(false)
            .build();
        submit.add_css_class("lock-submit");
        pill.append(&submit);
        let s2 = submit.clone();
        field.connect_changed(move |f| s2.set_sensitive(!f.text().is_empty()));
        let f2 = field.clone();
        submit.connect_clicked(move |_| {
            f2.emit_by_name::<()>("activate", &[]);
        });
        root.append(&pill);
        let message = centered("", "lock-message");
        root.append(&message);
        UserPanel {
            root,
            name: n,
            pill,
            field,
            submit,
            message,
        }
    }

    pub fn set_user(&self, name: &str) {
        self.name.set_label(name);
        if let Some(a) = self.root.first_child().and_downcast::<gtk::Label>() {
            a.set_label(&crate::login::initials(name));
        }
        self.message.set_label("");
        self.field.set_text("");
    }

    /// Wrong password: shake once, clear, say why.
    pub fn reject(&self, why: &str) {
        self.message.set_label(why);
        self.field.set_text("");
        self.pill.remove_css_class("shake");
        let p = self.pill.clone();
        glib::idle_add_local_once(move || p.add_css_class("shake"));
    }
}

// ---- Settings components (appended) ----

/// Appearance preview tile and accent swatch metrics.
pub const PREVIEW: (i32, i32) = (112, 72);
pub const SWATCH: i32 = 20;
/// Toolbar sheet width (Authenticate, Join, Add User).
pub const SHEET_WIDTH: i32 = 360;

/// Token colours one preview slice paints (from a resolved theme, never literals).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub surface: shared::theme::Color,
    pub sidebar: shared::theme::Color,
    pub card: shared::theme::Color,
    pub line: shared::theme::Color,
    pub dim: shared::theme::Color,
    pub accent: shared::theme::Color,
}

impl Palette {
    pub fn of(t: &crate::theme::Theme) -> Self {
        let c = |k: &str| {
            t.tokens
                .get(k)
                .and_then(|v| shared::theme::Color::parse(v))
                .unwrap_or(shared::theme::Color::WHITE)
        };
        Palette {
            surface: c("surface"),
            sidebar: c("sidebar"),
            card: c("card"),
            line: c("line"),
            dim: c("dim"),
            accent: c("accent"),
        }
    }
}

pub(crate) fn rgba(cr: &gtk::cairo::Context, c: shared::theme::Color) {
    cr.set_source_rgba(
        c.r as f64 / 255.0,
        c.g as f64 / 255.0,
        c.b as f64 / 255.0,
        c.a,
    );
}

pub(crate) fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Miniature window (sidebar, card, accent pill) per palette; several palettes split the tile diagonally (Auto).
fn draw_preview(cr: &gtk::cairo::Context, w: f64, h: f64, slices: &[Palette]) {
    let n = slices.len().max(1) as f64;
    for (i, p) in slices.iter().enumerate() {
        cr.save().ok();
        if slices.len() > 1 {
            // Diagonal split: slice i covers its share of the width, slanted.
            let x0 = w * i as f64 / n;
            let x1 = w * (i + 1) as f64 / n;
            let slant = h * 0.35;
            cr.move_to(if i == 0 { -1.0 } else { x0 + slant / 2.0 }, 0.0);
            cr.line_to(
                if i + 1 == slices.len() {
                    w + 1.0
                } else {
                    x1 + slant / 2.0
                },
                0.0,
            );
            cr.line_to(
                if i + 1 == slices.len() {
                    w + 1.0
                } else {
                    x1 - slant / 2.0
                },
                h,
            );
            cr.line_to(if i == 0 { -1.0 } else { x0 - slant / 2.0 }, h);
            cr.close_path();
            cr.clip();
        }
        rgba(cr, p.surface);
        cr.paint().ok();
        let sw = w * 0.3;
        rgba(cr, p.sidebar.over(p.surface));
        cr.rectangle(0.0, 0.0, sw, h);
        cr.fill().ok();
        // sidebar rows: one selected (accent), two dim
        rgba(cr, p.accent);
        rounded(cr, 6.0, 14.0, sw - 12.0, 7.0, 3.5);
        cr.fill().ok();
        rgba(cr, p.dim.over(p.sidebar.over(p.surface)));
        for k in 0..2 {
            rounded(cr, 6.0, 27.0 + 11.0 * k as f64, sw * 0.6, 4.0, 2.0);
            cr.fill().ok();
        }
        // content card with text lines
        let (cx, cy, cw, ch) = (sw + 8.0, 12.0, w - sw - 16.0, h - 24.0);
        rgba(cr, p.card);
        rounded(cr, cx, cy, cw, ch, 4.0);
        cr.fill_preserve().ok();
        rgba(cr, p.line.over(p.card));
        cr.set_line_width(1.0);
        cr.stroke().ok();
        rgba(cr, p.dim.over(p.card));
        for k in 0..3 {
            let lw = cw * [0.7, 0.5, 0.6][k];
            rounded(cr, cx + 6.0, cy + 8.0 + 9.0 * k as f64, lw - 12.0, 4.0, 2.0);
            cr.fill().ok();
        }
        rgba(cr, p.accent);
        rounded(cr, cx + cw - 26.0, cy + ch - 12.0, 20.0, 7.0, 3.5);
        cr.fill().ok();
        cr.restore().ok();
    }
}

/// Appearance tile: drawn preview over a caption; checked = accent ring. Group tiles with `set_group`.
pub fn theme_preview(slices: Vec<Palette>, title: &str, selected: bool) -> gtk::ToggleButton {
    let area = gtk::DrawingArea::builder()
        .content_width(PREVIEW.0)
        .content_height(PREVIEW.1)
        .build();
    area.add_css_class("preview-frame");
    area.set_overflow(gtk::Overflow::Hidden);
    area.set_draw_func(move |_, cr, w, h| draw_preview(cr, w as f64, h as f64, &slices));
    let b = vbox(S2);
    area.set_halign(gtk::Align::Center);
    b.append(&area);
    b.append(&centered(title, ""));
    let t = gtk::ToggleButton::builder()
        .child(&b)
        .active(selected)
        .build();
    t.add_css_class("preview-tile");
    t
}

/// Accent circle; checked shows a centre dot in the accent's text colour.
pub fn accent_swatch(
    color: shared::theme::Color,
    dot: shared::theme::Color,
    name: &str,
    selected: bool,
) -> gtk::ToggleButton {
    let area = gtk::DrawingArea::builder()
        .content_width(SWATCH)
        .content_height(SWATCH)
        .build();
    let t = gtk::ToggleButton::builder()
        .child(&area)
        .active(selected)
        .tooltip_text(name)
        .valign(gtk::Align::Center)
        .build();
    t.add_css_class("swatch");
    let t2 = t.downgrade();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        rgba(cr, color);
        cr.arc(w / 2.0, h / 2.0, w.min(h) / 2.0, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
        if t2.upgrade().is_some_and(|t| t.is_active()) {
            rgba(cr, dot);
            cr.arc(w / 2.0, h / 2.0, w.min(h) / 6.0, 0.0, std::f64::consts::TAU);
            cr.fill().ok();
        }
    });
    let a2 = area.clone();
    t.connect_toggled(move |_| a2.queue_draw());
    t
}

pub fn text_field(placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::builder().placeholder_text(placeholder).build();
    e.add_css_class("text-field");
    e
}

/// Selectable list row: icon, title (+ subtitle), checkmark when selected.
pub fn choice_row(icon_name: &str, title: &str, subtitle: &str, selected: bool) -> gtk::Button {
    let check = icon("object-select-symbolic", ICON);
    check.add_css_class("accent-icon");
    check.set_opacity(if selected { 1.0 } else { 0.0 });
    let b = gtk::Button::builder()
        .child(&list_row(
            icon_name,
            title,
            subtitle,
            Some(check.upcast_ref()),
        ))
        .build();
    b.add_css_class("row-button");
    b.add_css_class("choice-row");
    b
}

/// Indeterminate or fractional progress (long work in place, HIG §3).
pub fn progress() -> gtk::ProgressBar {
    let p = gtk::ProgressBar::new();
    p.set_hexpand(true);
    p.set_valign(gtk::Align::Center);
    p.add_css_class("progress");
    p
}

/// Sheet that slides down from the AppWindow toolbar (HIG §1), centred, fixed width.
pub fn toolbar_sheet(aw: &AppWindow, child: &impl IsA<gtk::Widget>) -> gtk::Revealer {
    let r = sheet(child);
    toolbar_place(aw, &r);
    r
}

/// Places an existing sheet (e.g. [`AuthSheet::revealer`]) under the toolbar, over the content.
pub fn toolbar_place(aw: &AppWindow, r: &gtk::Revealer) {
    r.set_halign(gtk::Align::Center);
    r.set_valign(gtk::Align::Start);
    r.set_width_request(SHEET_WIDTH);
    r.add_css_class("toolbar-sheet");
    // Overlay the content so opening a sheet never reflows the page.
    content_overlay(aw).add_overlay(r);
}

/// The overlay over `aw`'s body (sidebar + content), created the first time it is needed.
pub fn content_overlay(aw: &AppWindow) -> gtk::Overlay {
    let body = aw.content.parent().and_downcast::<gtk::Box>();
    if let Some(o) = body
        .as_ref()
        .and_then(|b| b.parent().and_downcast::<gtk::Overlay>())
    {
        return o;
    }
    let o = gtk::Overlay::new();
    o.set_vexpand(true);
    if let Some(body) = body {
        aw.root.remove(&body);
        o.set_child(Some(&body));
    }
    aw.root.append(&o);
    if aw.status.parent().is_some_and(|p| p == aw.root) {
        aw.root.reorder_child_after(&aw.status, Some(&o));
    }
    o
}

/// Slider for list rows: the knob stays inside its 16px trough (layout check clean).
pub fn row_slider(value: f64, changed: impl Fn(f64) + 'static) -> gtk::Scale {
    let s = slider(value, changed);
    s.add_css_class("row-slider");
    s
}

/// Secondary footnote under a section (wraps; never widens the page).
pub fn footnote(text: &str) -> gtk::Label {
    let l = label(text, "secondary");
    l.set_wrap(true);
    l.add_css_class("footnote");
    l
}

/// Horizontal row that wraps onto more lines when narrow (preview tiles, swatches). Not selectable.
pub fn wrap_row(spacing: i32, max_per_line: u32) -> gtk::FlowBox {
    let f = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(spacing as u32)
        .row_spacing(spacing as u32)
        .max_children_per_line(max_per_line)
        .min_children_per_line(1)
        .homogeneous(true)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    f.add_css_class("wrap-row");
    f
}

/// Minimum width of a trailing value in info rows (About), in characters.
pub const VALUE_CHARS: i32 = 18;

// ---- App Store components (appended) ----

/// Store tile width (Discover rows, category grids) and hero icon size.
pub const STORE_TILE: i32 = 196;
/// Summary width in characters inside a store tile.
pub const TILE_SUMMARY_CHARS: i32 = 18;
pub const RING: i32 = 24;

/// Hero banner icon size.
pub const HERO_ICON: i32 = 128;
/// Icon for packages outside the catalog (`pkg search` hits).
pub const PACKAGE_ICON: &str = "wolos-software";

/// Official icon of a store catalog app (`kit::store_icon_file`), else the generic package icon.
pub fn store_icon(id: &str, size: i32) -> gtk::Widget {
    match super::store_icon_file(id) {
        Some(p) => svg_icon(p, size).upcast(),
        None => app_icon(PACKAGE_ICON, size).upcast(),
    }
}

/// Indeterminate ring in the accent colour (install in place, HIG §3).
pub fn progress_ring() -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder()
        .content_width(RING)
        .content_height(RING)
        .valign(gtk::Align::Center)
        .build();
    a.add_css_class("progress-ring");
    a.set_draw_func(|a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let c = a.color();
        let t = a
            .frame_clock()
            .map_or(0.0, |f| f.frame_time() as f64 / 1_000_000.0);
        let r = w.min(h) / 2.0 - 2.0;
        cr.set_line_width(3.0);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.2);
        cr.arc(w / 2.0, h / 2.0, r, 0.0, std::f64::consts::TAU);
        cr.stroke().ok();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let s = (t * 4.0) % std::f64::consts::TAU;
        cr.arc(w / 2.0, h / 2.0, r, s, s + 1.6);
        cr.stroke().ok();
    });
    a.add_tick_callback(|a, _| {
        a.queue_draw();
        glib::ControlFlow::Continue
    });
    a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileState {
    Get,
    Installing,
    Open,
}

/// Store tile: icon, name, one-line summary, Get/Open pill (or progress ring while installing).
#[derive(Clone)]
pub struct AppTile {
    pub root: gtk::Box,
    pub action: gtk::Button,
    ring: gtk::DrawingArea,
}

impl AppTile {
    pub fn new(icon: &gtk::Widget, name: &str, summary: &str) -> Self {
        let root = hbox(S3);
        root.add_css_class("app-tile");
        root.set_width_request(STORE_TILE);
        root.append(icon);
        let t = vbox(0);
        t.set_hexpand(true);
        t.set_valign(gtk::Align::Center);
        let n = label(name, "title");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(TILE_SUMMARY_CHARS);
        n.add_css_class("may-ellipsize");
        t.append(&n);
        let s = label(summary, "secondary");
        s.set_wrap(true);
        s.set_lines(2);
        s.set_max_width_chars(TILE_SUMMARY_CHARS);
        s.set_width_chars(TILE_SUMMARY_CHARS);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.add_css_class("may-ellipsize");
        t.append(&s);
        root.append(&t);
        let action = gtk::Button::with_label("Get");
        action.add_css_class("get-button");
        action.set_valign(gtk::Align::Center);
        root.append(&action);
        let ring = progress_ring();
        ring.set_visible(false);
        root.append(&ring);
        AppTile { root, action, ring }
    }

    pub fn set_state(&self, s: TileState) {
        self.action.set_visible(s != TileState::Installing);
        self.ring.set_visible(s == TileState::Installing);
        self.action
            .set_label(if s == TileState::Open { "Open" } else { "Get" });
    }
}

/// Page hero: wide banner in the app's own two colours (white text), headline, subhead and
/// action on the left, the large app icon on the right.
pub fn hero_banner(
    colors: [&str; 2],
    eyebrow: &str,
    headline: &str,
    subhead: &str,
    icon: &gtk::Widget,
    action: &gtk::Button,
) -> gtk::Box {
    let b = hbox(S4);
    b.add_css_class("hero-banner");
    b.add_css_class(&gradient_class(colors));
    let t = vbox(S2);
    t.set_hexpand(true);
    t.set_valign(gtk::Align::Center);
    t.append(&label(&eyebrow.to_uppercase(), "hero-eyebrow"));
    let tl = label(headline, "large-title");
    tl.set_wrap(true);
    t.append(&tl);
    let s = label(subhead, "hero-subhead");
    s.set_wrap(true);
    t.append(&s);
    action.set_halign(gtk::Align::Start);
    action.set_margin_top(S2);
    t.append(action);
    b.append(&t);
    icon.set_valign(gtk::Align::Center);
    b.append(icon);
    b
}

thread_local! {
    static GRADIENTS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// CSS class painting a 135deg two-stop gradient; one display provider per colour pair.
fn gradient_class(colors: [&str; 2]) -> String {
    let [a, b] = colors.map(|c| c.trim_start_matches('#').to_ascii_lowercase());
    let class = format!("gradient-{a}-{b}");
    let new = GRADIENTS.with_borrow_mut(|g| g.insert(class.clone()));
    if new && let Some(d) = gtk::gdk::Display::default() {
        let p = gtk::CssProvider::new();
        p.load_from_string(&format!(
            ".{class} {{ background-image: linear-gradient(135deg, #{a}, #{b}); }}"
        ));
        gtk::style_context_add_provider_for_display(
            &d,
            &p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    }
    class
}

impl AppWindow {
    /// Search field at the top of the sidebar (App Store, Settings-style apps).
    pub fn sidebar_search(&self, placeholder: &str) -> gtk::SearchEntry {
        let s = search_field(placeholder);
        let r = gtk::ListBoxRow::builder()
            .child(&s)
            .selectable(false)
            .activatable(false)
            .build();
        r.add_css_class("sidebar-search");
        self.sidebar.prepend(&r);
        s
    }

    /// Content page with no sidebar row (search results); show it with `content.set_visible_child_name`.
    pub fn hidden_page(&self, name: &str, page: &impl IsA<gtk::Widget>) {
        let scroll = gtk::ScrolledWindow::builder()
            .child(page)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        self.content.add_named(&scroll, Some(name));
    }
}

impl AppWindow {
    /// Space between sidebar groups (System Settings-style, no header).
    pub fn source_gap(&self) {
        let r = gtk::ListBoxRow::builder()
            .child(&vbox(0))
            .selectable(false)
            .activatable(false)
            .build();
        r.add_css_class("source-gap");
        self.sidebar.append(&r);
    }

    /// Shows the page `name`: its sidebar row when it has one, else a [`hidden_page`](Self::hidden_page)
    /// (subpages, search results) with the sidebar selection cleared.
    pub fn open(&self, name: &str) {
        let mut i = 0;
        while let Some(r) = self.sidebar.row_at_index(i) {
            if r.widget_name().eq_ignore_ascii_case(name) {
                self.sidebar.select_row(Some(&r));
                self.content.set_visible_child_name(&r.widget_name());
                return;
            }
            i += 1;
        }
        if self.content.child_by_name(name).is_some() {
            self.sidebar.unselect_all();
            self.content.set_visible_child_name(name);
        }
    }
}
