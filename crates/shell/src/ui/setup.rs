//! Setup Assistant (`wol-setup`, ADR 0004, ADR 0013): the welcome art, then one card per question
//! over the blurred wallpaper, the same glass as the login window and the lock screen. Runs as
//! `_wollogin` on first boot; Wi-Fi and the account go to wol-login over its socket. Return
//! continues, Escape goes back; Get Started fades to black into the desktop.
use crate::kit::assistant::{self as a, Art, Card, Choice, ChoiceList};
use crate::kit::{self, reveal, widgets as w};
use crate::login::{self, Verdict};
use crate::setup::{
    APPEARANCES, Appearance, Draft, Field, Flow, Hints, Job, KEYBOARDS, LANGUAGES, Next, STEPS,
    Step, THEMES, WifiPick, initials_or_placeholder,
};
use crate::system::{System, WifiNetwork};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const FIELDS: [(Field, &str, bool); 5] = [
    (Field::FullName, "Full name", false),
    (Field::Name, "Account name", false),
    (Field::Password, "Password", true),
    (Field::Verify, "Verify", true),
    (Field::Hint, "Hint", false),
];

/// The Wi-Fi page's parts that change with each scan.
#[derive(Clone)]
struct WifiUi {
    holder: gtk::Box,
    pw: gtk::Widget,
    pw_row: gtk::Box,
}

/// `holder` shows `child` only.
fn replace_child(holder: &gtk::Box, child: &impl IsA<gtk::Widget>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    holder.append(child);
}

pub struct Setup {
    pub root: gtk::Widget,
    pager: gtk::Stack,
    flow: RefCell<Flow>,
    cards: RefCell<Vec<(Step, Card)>>,
    editors: RefCell<Vec<(Field, gtk::Widget)>>,
    avatar: RefCell<Option<gtk::Label>>,
    keyboard: RefCell<Option<gtk::DropDown>>,
    wifi_ui: RefCell<Option<WifiUi>>,
    scanning: Cell<bool>,
    busy: Cell<bool>,
}

impl Setup {
    /// Language and keyboard from this machine's hints (none in golden renders).
    pub fn new(sys: &dyn System) -> Rc<Setup> {
        let hints = if std::env::var_os("WOL_FAKE_NOW").is_some() {
            Hints::default()
        } else {
            Hints::read(std::path::Path::new("/"))
        };
        Setup::with(sys, &hints)
    }

    pub fn with(sys: &dyn System, hints: &Hints) -> Rc<Setup> {
        let net = sys.network();
        let pager = a::pager();
        let s = Rc::new(Setup {
            root: super::lock::screen(&pager),
            pager,
            flow: RefCell::new(Flow::new(Draft::detected(hints), net.online && !net.wifi)),
            cards: Default::default(),
            editors: Default::default(),
            avatar: Default::default(),
            keyboard: Default::default(),
            wifi_ui: Default::default(),
            scanning: Cell::new(false),
            busy: Cell::new(false),
        });
        s.pager.add_named(&s.welcome(), Some(Step::Welcome.id()));
        for st in &STEPS[1..] {
            let card = match st {
                Step::Language => s.language(),
                Step::Wifi => s.wifi(sys),
                Step::Account => s.account(),
                Step::Appearance => s.appearance(),
                _ => s.done(),
            };
            s.wire(*st, &card);
            s.pager.add_named(&card.root, Some(st.id()));
            s.cards.borrow_mut().push((*st, card));
        }
        s
    }

    /// Shows `step` at once (golden renders, tests).
    pub fn show(&self, step: Step) {
        self.flow.borrow_mut().step = step;
        self.pager
            .set_transition_type(gtk::StackTransitionType::None);
        self.pager.set_visible_child_name(step.id());
        self.pager
            .set_transition_type(gtk::StackTransitionType::Crossfade);
    }

    /// Types into an account field as the user would (goldens, tests).
    pub fn type_into(&self, f: Field, text: &str) {
        if let Some(e) = self.editor(f) {
            e.set_text(text);
        }
    }

    fn editor(&self, f: Field) -> Option<gtk::Editable> {
        self.editors
            .borrow()
            .iter()
            .find(|(g, _)| *g == f)
            .and_then(|(_, e)| e.clone().dynamic_cast::<gtk::Editable>().ok())
    }

    fn with_card(&self, step: Step, f: impl FnOnce(&Card)) {
        if let Some((_, c)) = self.cards.borrow().iter().find(|(s, _)| *s == step) {
            f(c)
        }
    }

    /// Back and Continue of a card.
    fn wire(self: &Rc<Self>, step: Step, card: &Card) {
        let me = Rc::downgrade(self);
        card.next.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.next()
            }
        });
        let me = Rc::downgrade(self);
        card.back.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.back()
            }
        });
        card.back.set_visible(step != Step::Done);
    }

    /// Continue (button or Return).
    pub fn next(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let n = self.flow.borrow_mut().advance();
        self.apply(n);
    }

    /// Back (button or Escape).
    pub fn back(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let from = self.flow.borrow().step;
        if let Some(p) = self.flow.borrow_mut().back() {
            self.with_card(from, |c| c.say("", false));
            self.pager.set_visible_child_name(p.id());
        }
    }

    fn apply(self: &Rc<Self>, n: Next) {
        let step = self.flow.borrow().step;
        self.clear_problems();
        match n {
            Next::Go(s) => {
                for (_, c) in self.cards.borrow().iter() {
                    c.say("", false);
                }
                self.pager.set_visible_child_name(s.id());
                if s == Step::Wifi {
                    self.scan();
                }
                if s == Step::Account
                    && let Some(e) = self.editor(Field::FullName)
                {
                    e.grab_focus();
                }
            }
            Next::Stay(field, why) => {
                self.with_card(step, |c| c.say(&why, true));
                if let Some(f) = field {
                    self.mark(f);
                }
            }
            Next::Run(job) => self.run(job),
            Next::Finish => reveal::exit_faded(),
        }
    }

    /// Field with the problem: outlined and focused.
    fn mark(&self, f: Field) {
        if let Some((_, e)) = self.editors.borrow().iter().find(|(g, _)| *g == f) {
            e.add_css_class("problem");
            e.grab_focus();
        }
    }

    fn clear_problems(&self) {
        for (_, e) in self.editors.borrow().iter() {
            e.remove_css_class("problem");
        }
    }

    /// Runs `job` through wol-login off the main thread, then lets the flow decide.
    fn run(self: &Rc<Self>, job: Job) {
        let step = self.flow.borrow().step;
        let (req, doing) = match &job {
            Job::JoinWifi { ssid, password } => (
                shared::login::encode_wifi(ssid, password),
                format!("Joining “{ssid}”…"),
            ),
            Job::CreateAccount(acc) => (acc.encode(), "Setting up your account…".to_string()),
        };
        self.busy.set(true);
        self.with_card(step, |c| {
            c.say(&doing, false);
            c.set_busy(true);
        });
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let v = gtk::gio::spawn_blocking(move || login::request(login::LOGIN_SOCKET, &req))
                .await
                .unwrap_or(Verdict::Error);
            let Some(s) = me.upgrade() else {
                return;
            };
            s.busy.set(false);
            s.with_card(step, |c| c.set_busy(false));
            let n = s.flow.borrow_mut().job_done(&job, v);
            s.apply(n);
        });
    }

    fn welcome(self: &Rc<Self>) -> gtk::Widget {
        let me = Rc::downgrade(self);
        let go = w::button("Continue", w::Kind::Primary, move || {
            if let Some(s) = me.upgrade() {
                s.next()
            }
        });
        a::welcome(
            "setup-welcome",
            "Welcome to\nWolOS",
            Step::Welcome.subtitle(),
            &go,
        )
    }

    fn language(self: &Rc<Self>) -> Card {
        let d = self.flow.borrow().draft.clone();
        let body = w::vbox(w::S4);
        let rows = LANGUAGES
            .iter()
            .map(|l| Choice {
                icon: None,
                title: l.name.into(),
                detail: l.region.into(),
                trail: None,
            })
            .collect();
        let me = Rc::downgrade(self);
        let list = ChoiceList::new(rows, Some(d.language), move |i| {
            if let Some(s) = me.upgrade() {
                s.flow.borrow_mut().draft.set_language(i);
                let k = s.flow.borrow().draft.keyboard;
                if let Some(dd) = &*s.keyboard.borrow() {
                    dd.set_selected(k as u32);
                }
            }
        });
        body.append(&list.root);
        let names: Vec<&str> = KEYBOARDS.iter().map(|k| k.1).collect();
        let kb = w::dropdown(&names, d.keyboard as u32);
        let me = Rc::downgrade(self);
        kb.connect_selected_notify(move |dd| {
            let Some(s) = me.upgrade() else {
                return;
            };
            let i = (dd.selected() as usize).min(KEYBOARDS.len() - 1);
            s.flow.borrow_mut().draft.keyboard = i;
            if std::env::var_os("WOL_FAKE_NOW").is_none() {
                kit::spawn(&crate::setup::keyboard_command(KEYBOARDS[i].0));
            }
        });
        body.append(&a::form_row("Keyboard", &kb));
        *self.keyboard.borrow_mut() = Some(kb);
        Card::new(
            Art::Icon("preferences-desktop-locale-symbolic"),
            Step::Language.title(),
            Step::Language.subtitle(),
            &body,
        )
    }

    fn wifi(self: &Rc<Self>, sys: &dyn System) -> Card {
        let body = w::vbox(w::S4);
        let holder = w::vbox(0);
        holder.set_vexpand(true);
        body.append(&holder);
        let pw = w::password_field("Network password");
        let pw_row = a::form_row("Password", &pw);
        pw_row.set_visible(false);
        body.append(&pw_row);
        let me = Rc::downgrade(self);
        pw.connect_changed(move |e| {
            if let Some(s) = me.upgrade()
                && let Some(wp) = &mut s.flow.borrow_mut().wifi
            {
                wp.password = e.text().to_string();
            }
        });
        *self.wifi_ui.borrow_mut() = Some(WifiUi {
            holder,
            pw: pw.upcast(),
            pw_row,
        });
        // What the system already knows; entering the page scans again (wol-login).
        self.show_networks(sys.wifi_networks());
        let card = Card::new(
            Art::Icon("network-wireless-symbolic"),
            Step::Wifi.title(),
            Step::Wifi.subtitle(),
            &body,
        );
        let me = Rc::downgrade(self);
        card.add_action(&w::button("Not Now", w::Kind::Plain, move || {
            if let Some(s) = me.upgrade()
                && !s.busy.get()
            {
                let n = s.flow.borrow_mut().skip_wifi();
                s.apply(n);
            }
        }));
        card
    }

    /// Scans through wol-login (a few seconds, spinner meanwhile), then lists what it heard.
    fn scan(self: &Rc<Self>) {
        if self.scanning.replace(true) {
            return;
        }
        if let Some(ui) = &*self.wifi_ui.borrow() {
            replace_child(&ui.holder, &a::waiting("Looking for networks…"));
            ui.pw_row.set_visible(false);
        }
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gtk::gio::spawn_blocking(|| login::wifi_scan(login::LOGIN_SOCKET))
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let Some(s) = me.upgrade() else {
                return;
            };
            s.scanning.set(false);
            s.show_networks(
                found
                    .into_iter()
                    .map(|n| WifiNetwork {
                        ssid: n.ssid,
                        signal: n.signal,
                        secure: n.secure,
                        connected: n.connected,
                    })
                    .collect(),
            );
        });
    }

    /// The network list (or how to find one); the joined network is the pick.
    fn show_networks(self: &Rc<Self>, nets: Vec<WifiNetwork>) {
        let Some(ui) = self.wifi_ui.borrow().as_ref().map(WifiUi::clone) else {
            return;
        };
        self.flow.borrow_mut().wifi = nets.iter().find(|n| n.connected).map(|n| WifiPick {
            ssid: n.ssid.clone(),
            secure: n.secure,
            connected: true,
            password: String::new(),
        });
        ui.pw_row.set_visible(false);
        if nets.is_empty() {
            let me = Rc::downgrade(self);
            let again = w::button("Look Again", w::Kind::Default, move || {
                if let Some(s) = me.upgrade() {
                    s.scan()
                }
            });
            replace_child(
                &ui.holder,
                &w::empty_state(
                    "network-wireless-offline-symbolic",
                    "No networks found",
                    "Move closer to your Wi-Fi and look again, or choose Not Now.",
                    Some(again),
                ),
            );
            return;
        }
        let rows = nets
            .iter()
            .map(|n| Choice {
                icon: Some(
                    crate::status::network(crate::status::Network {
                        online: true,
                        wifi: true,
                        signal: Some(n.signal),
                    })
                    .into(),
                ),
                title: n.ssid.clone(),
                detail: String::new(),
                trail: n
                    .secure
                    .then(|| "network-wireless-encrypted-symbolic".into()),
            })
            .collect();
        let selected = nets.iter().position(|n| n.connected);
        let me = Rc::downgrade(self);
        let list = ChoiceList::new(rows, selected, move |i| {
            let Some(s) = me.upgrade() else {
                return;
            };
            let n = &nets[i];
            let pw = ui.pw.clone().dynamic_cast::<gtk::Editable>().ok();
            s.flow.borrow_mut().wifi = Some(WifiPick {
                ssid: n.ssid.clone(),
                secure: n.secure,
                connected: n.connected,
                password: pw
                    .as_ref()
                    .map(|p| p.text().to_string())
                    .unwrap_or_default(),
            });
            ui.pw_row.set_visible(n.secure && !n.connected);
            if ui.pw_row.is_visible() {
                ui.pw.grab_focus();
            }
        });
        if let Some(ui) = &*self.wifi_ui.borrow() {
            replace_child(&ui.holder, &list.root);
        }
    }

    fn account(self: &Rc<Self>) -> Card {
        let me = Rc::downgrade(self);
        let picker = a::AvatarPicker::new(0, move |i| {
            if let Some(s) = me.upgrade() {
                s.flow.borrow_mut().draft.avatar = i;
            }
        });
        picker.avatar.set_label(&initials_or_placeholder(""));
        *self.avatar.borrow_mut() = Some(picker.avatar.clone());
        let form = w::vbox(w::S2);
        for (f, title, secret) in FIELDS {
            let e: gtk::Widget = if secret {
                w::password_field("").upcast()
            } else {
                w::text_field("").upcast()
            };
            if f == Field::Hint {
                e.set_property(
                    "placeholder-text",
                    "Optional, shown after three wrong passwords",
                );
            }
            e.add_css_class("assistant-field");
            form.append(&a::form_row(title, &e));
            self.editors.borrow_mut().push((f, e));
        }
        let note = a::caption(
            "This account is the administrator: it can install apps and change any setting.",
        );
        note.set_wrap(true);
        form.append(&note);
        for (f, e) in self.editors.borrow().iter() {
            let Ok(e) = e.clone().dynamic_cast::<gtk::Editable>() else {
                continue;
            };
            let (f, me) = (*f, Rc::downgrade(self));
            e.connect_changed(move |e| {
                let Some(s) = me.upgrade() else {
                    return;
                };
                let t = e.text().to_string();
                let derived = {
                    let d = &mut s.flow.borrow_mut().draft;
                    match f {
                        Field::FullName => d.set_full_name(&t),
                        Field::Name => d.set_name(&t),
                        Field::Password => d.password = t.clone(),
                        Field::Verify => d.verify = t.clone(),
                        Field::Hint => d.hint = t.clone(),
                    }
                    d.name.clone()
                };
                if let Some((_, w)) = s.editors.borrow().iter().find(|(g, _)| *g == f) {
                    w.remove_css_class("problem");
                }
                if f == Field::FullName {
                    if let Some(av) = &*s.avatar.borrow() {
                        av.set_label(&initials_or_placeholder(&t));
                    }
                    if let Some(nf) = s.editor(Field::Name)
                        && nf.text() != derived
                    {
                        nf.set_text(&derived);
                    }
                }
            });
        }
        Card::new(
            Art::Widget(picker.root.upcast()),
            Step::Account.title(),
            Step::Account.subtitle(),
            &form,
        )
    }

    fn appearance(self: &Rc<Self>) -> Card {
        let body = w::vbox(w::S4);
        let light = crate::theme::resolve(Some("wol-light"), false);
        let dark = crate::theme::resolve(Some("wol-dark"), true);
        let selected = self.flow.borrow().draft.appearance;
        let modes: Vec<gtk::ToggleButton> = APPEARANCES
            .iter()
            .map(|ap| {
                let slices = match ap {
                    Appearance::Light => vec![w::Palette::of(&light)],
                    Appearance::Dark => vec![w::Palette::of(&dark)],
                    Appearance::Auto => vec![w::Palette::of(&light), w::Palette::of(&dark)],
                };
                let t = w::theme_preview(slices, ap.title(), *ap == selected);
                let (me, ap) = (Rc::downgrade(self), *ap);
                t.connect_toggled(move |t| {
                    if let Some(s) = me.upgrade()
                        && t.is_active()
                    {
                        s.flow.borrow_mut().draft.appearance = ap;
                        s.preview_look();
                    }
                });
                t
            })
            .collect();
        body.append(&a::tile_row(&modes));
        let dark_now = super::lock::prefers_dark();
        let looks: Vec<gtk::ToggleButton> = THEMES
            .iter()
            .enumerate()
            .map(|(i, (accent, title))| {
                let id = wol_core::theme::accent::theme_id(
                    Some(if dark_now {
                        shared::theme::Variant::Dark
                    } else {
                        shared::theme::Variant::Light
                    }),
                    accent,
                );
                let t = crate::theme::resolve(id.as_deref(), dark_now);
                let tile = w::theme_preview(vec![w::Palette::of(&t)], title, i == 0);
                let me = Rc::downgrade(self);
                tile.connect_toggled(move |t| {
                    if let Some(s) = me.upgrade()
                        && t.is_active()
                    {
                        s.flow.borrow_mut().draft.theme = i;
                        s.preview_look();
                    }
                });
                tile
            })
            .collect();
        body.append(&a::tile_row(&looks));
        Card::new(
            Art::Icon("preferences-desktop-appearance-symbolic"),
            Step::Appearance.title(),
            Step::Appearance.subtitle(),
            &body,
        )
    }

    /// The assistant itself takes the chosen look.
    fn preview_look(&self) {
        let d = self.flow.borrow().draft.clone();
        let variant = match d.appearance {
            Appearance::Light => Some(shared::theme::Variant::Light),
            Appearance::Dark => Some(shared::theme::Variant::Dark),
            Appearance::Auto => None,
        };
        let dark = match d.appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::Auto => super::lock::prefers_dark(),
        };
        let id = wol_core::theme::accent::theme_id(variant, d.accent());
        kit::style::apply(&crate::theme::resolve(id.as_deref(), dark), "");
    }

    /// No Back: the account exists once this page shows.
    fn done(self: &Rc<Self>) -> Card {
        let c = Card::moment(
            "keyhole",
            Step::Done.title(),
            Step::Done.subtitle(),
            &a::blank(),
        );
        c.next.set_label("Get Started");
        c
    }
}

pub fn run() -> glib::ExitCode {
    a::run("org.wolos.setup", "setup", |win| {
        let s = Setup::new(&*crate::system::default_system());
        // Fades in from the compositor's black; Get Started fades out to it (ADR 0012).
        let shown = reveal::Reveal::new(&s.root, true);
        win.set_child(Some(&shown.root));
        shown.show_when(super::lock::backdrop_ready, || {
            reveal::show_pointer();
            reveal::boot_mark("setup shown");
        });
        let k = s.flow.borrow().draft.keyboard;
        if KEYBOARDS[k].0 != KEYBOARDS[0].0 {
            kit::spawn(&crate::setup::keyboard_command(KEYBOARDS[k].0));
        }
        // The key controller keeps the assistant alive with its window.
        let s2 = s.clone();
        a::keys(win, move || s.next(), move || s2.back());
    })
}
