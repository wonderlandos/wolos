//! Install WolOS (`wol-installer`, ADR 0013): choose the disk, watch it install, restart. The same
//! cards as the Setup Assistant. The work is `install-run` as root; this app reads its progress
//! lines and answers bsdinstall's error box so a failed run ends.
use crate::install::{self, Check, Disk, Report, Stage};
use crate::kit::assistant::{self as a, Art, Card, Choice, ChoiceList};
use crate::kit::{self, reveal, widgets as w};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Choose,
    /// Secure Boot is on: the installed WolOS wouldn't start; say so before erasing.
    SecureBoot,
    Installing,
    Done,
    Failed,
}

impl Page {
    fn id(self) -> &'static str {
        match self {
            Page::Choose => "choose",
            Page::SecureBoot => "secure-boot",
            Page::Installing => "installing",
            Page::Done => "done",
            Page::Failed => "failed",
        }
    }
}

/// What the install process reports to the page.
enum Event {
    Report(Report),
    ErrorBox,
    Exit(bool),
}

/// Takes (true) or gives back (false) what keeps the machine awake.
type KeepAwake = Box<dyn Fn(bool)>;

pub struct Installer {
    pub root: gtk::Widget,
    pager: gtk::Stack,
    disks: Vec<Disk>,
    /// What `install-check` found on each disk (other systems, Secure Boot, refusals).
    checks: RefCell<Vec<Check>>,
    /// The picked disk; the list writes it, Return and Erase and Install read it.
    chosen: Rc<Cell<Option<usize>>>,
    list: Option<ChoiceList>,
    choose: Card,
    warning: gtk::Label,
    progress: a::Progress,
    page: Cell<Page>,
    /// Last report and when its stage began; first package sample for the time estimate.
    report: RefCell<(Report, Instant)>,
    sample: Cell<Option<(f64, Instant)>>,
    /// The failed page's subtitle says where it stopped.
    failed: Card,
    stdin: RefCell<Option<std::process::ChildStdin>>,
    /// Idle sleep and screen-off held off while installing (`run` sets how).
    awake: RefCell<install::Awake>,
    keep_awake: RefCell<Option<KeepAwake>>,
}

const ERASE_FIRST: &str = "Choose a disk. Everything on it will be erased.";

impl Installer {
    pub fn new(disks: Vec<Disk>, checks: Vec<Check>) -> Rc<Installer> {
        let pager = a::pager();
        let body = w::vbox(w::S4);
        let (warn_box, warning) = a::warning(ERASE_FIRST);
        let one = (disks.len() == 1).then_some(0);
        let me: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(one));
        let mut list = None;
        if disks.is_empty() {
            body.append(&w::empty_state(
                "drive-harddisk-symbolic",
                "No disk found",
                "Connect the disk you want to install on, then open Install WolOS again.",
                None,
            ));
        } else {
            let rows = disks
                .iter()
                .map(|d| Choice {
                    icon: Some("drive-harddisk-symbolic".into()),
                    title: d.title(),
                    detail: install::size_text(d.bytes),
                    trail: None,
                })
                .collect();
            let (chosen, warn, ds, cs) =
                (me.clone(), warning.clone(), disks.clone(), checks.clone());
            let l = ChoiceList::new(rows, one, move |i| {
                chosen.set(Some(i));
                warn.set_label(&erase_warning(&ds[i], cs.get(i)));
            });
            body.append(&l.root);
            list = Some(l);
            if let Some(i) = one {
                warning.set_label(&erase_warning(&disks[i], checks.get(i)));
            }
        }
        body.append(&warn_box);
        let choose = Card::new(
            Art::Illustration("white-rabbit"),
            "Install WolOS",
            "Choose the disk to install on.",
            &body,
        );
        choose.next.set_label("Erase and Install");
        choose.back.set_label("Cancel");
        let secure = Card::moment(
            "keyhole",
            install::SECURE_BOOT_TITLE,
            install::SECURE_BOOT_TEXT,
            &a::blank(),
        );
        secure.next.set_label("Restart");
        let anyway = w::button("Install Anyway", w::Kind::Default, || {});
        secure.add_action(&anyway);
        let progress = a::Progress::new();
        let installing = Card::moment(
            "pocket-watch",
            "Installing WolOS",
            "This takes a few minutes. Keep your computer plugged in.",
            &progress.root,
        );
        installing.without_foot();
        let done = Card::moment(
            "keyhole",
            "WolOS Is Installed",
            "Restart to start using WolOS. Remove the installer stick when the screen goes black.",
            &a::blank(),
        );
        done.back.set_visible(false);
        done.next.set_label("Restart");
        let failed = Card::moment(
            "teacup",
            "WolOS Couldn't Be Installed",
            install::stopped_text(Stage::Finish),
            &a::blank(),
        );
        failed.back.set_visible(false);
        failed.next.set_label("Try Again");
        failed.add_action(&w::button("Restart", w::Kind::Default, || {
            kit::spawn(&install::restart_command())
        }));
        for (p, c) in [
            (Page::Choose, &choose),
            (Page::SecureBoot, &secure),
            (Page::Installing, &installing),
            (Page::Done, &done),
            (Page::Failed, &failed),
        ] {
            pager.add_named(&c.root, Some(p.id()));
        }
        let s = Rc::new(Installer {
            root: super::lock::screen(&pager),
            pager,
            disks,
            checks: RefCell::new(checks),
            chosen: me,
            list,
            choose,
            warning,
            progress,
            page: Cell::new(Page::Choose),
            report: RefCell::new((Report::new(Stage::Prepare, 0), Instant::now())),
            sample: Cell::new(None),
            failed,
            stdin: RefCell::new(None),
            awake: RefCell::new(install::Awake::default()),
            keep_awake: RefCell::new(None),
        });
        let s2 = Rc::downgrade(&s);
        s.choose.next.connect_clicked(move |_| {
            if let Some(s) = s2.upgrade() {
                s.next();
            }
        });
        s.choose.back.connect_clicked(|_| std::process::exit(0));
        let s4 = Rc::downgrade(&s);
        secure.back.connect_clicked(move |_| {
            if let Some(s) = s4.upgrade() {
                s.show(Page::Choose);
            }
        });
        secure
            .next
            .connect_clicked(|_| kit::spawn(&install::restart_command()));
        let s5 = Rc::downgrade(&s);
        anyway.connect_clicked(move |_| {
            if let Some(s) = s5.upgrade() {
                s.erase_chosen();
            }
        });
        done.next
            .connect_clicked(|_| kit::spawn(&install::restart_command()));
        let s3 = Rc::downgrade(&s);
        s.failed.next.connect_clicked(move |_| {
            if let Some(s) = s3.upgrade() {
                s.show(Page::Choose);
            }
        });
        s
    }

    pub fn show(&self, p: Page) {
        self.page.set(p);
        self.pager.set_visible_child_name(p.id());
        let change = self.awake.borrow_mut().set(p == Page::Installing);
        if let (Some(on), Some(f)) = (change, &*self.keep_awake.borrow()) {
            f(on);
        }
    }

    /// How to keep the machine awake while installing (the app window's inhibitors).
    pub fn set_keep_awake(&self, f: impl Fn(bool) + 'static) {
        *self.keep_awake.borrow_mut() = Some(Box::new(f));
    }

    /// Picks disk `i` as a click would (goldens).
    pub fn pick(&self, i: usize) {
        if let Some(d) = self.disks.get(i) {
            self.chosen.set(Some(i));
            self.warning
                .set_label(&erase_warning(d, self.checks.borrow().get(i)));
            if let Some(l) = &self.list {
                l.select(i);
            }
        }
    }

    /// Progress page at `r`, `left` seconds remaining (goldens).
    pub fn show_progress(&self, r: &Report, left: Option<f64>) {
        self.show(Page::Installing);
        self.progress.set(
            install::fraction(r, 0.0),
            &format!(
                "{} {}",
                install::doing(r.stage),
                install::remaining_text(left)
            ),
        );
    }

    /// Failed page, stopped during `at`.
    pub fn show_failed(&self, at: Stage) {
        self.failed.subtitle.set_label(install::stopped_text(at));
        self.show(Page::Failed);
    }

    pub fn installing(&self) -> bool {
        self.page.get() == Page::Installing
    }

    /// Return: Erase and Install on the first page, Restart or Try Again later.
    pub fn next(self: &Rc<Self>) {
        match self.page.get() {
            Page::Choose => {
                let Some(i) = self.chosen.get().filter(|i| *i < self.disks.len()) else {
                    self.choose.say("Choose a disk to install on.", true);
                    return;
                };
                let d = &self.disks[i];
                if !d.big_enough() {
                    let t = install::refusal_text("too-small", &d.title()).unwrap_or_default();
                    self.choose.say(&t, true);
                    return;
                }
                // Fresh facts just before erasing (a stick plugged in, a disk mounted since).
                let c = check_disk(&d.name);
                if let Some(t) = c
                    .refused
                    .as_deref()
                    .and_then(|w| install::refusal_text(w, &d.title()))
                {
                    self.choose.say(&t, true);
                    return;
                }
                let secure_boot = c.secure_boot;
                if let Some(slot) = self.checks.borrow_mut().get_mut(i) {
                    *slot = c;
                }
                self.choose.say("", false);
                if secure_boot {
                    self.show(Page::SecureBoot);
                } else {
                    self.erase_chosen();
                }
            }
            Page::SecureBoot => kit::spawn(&install::restart_command()),
            Page::Installing => {}
            Page::Done => kit::spawn(&install::restart_command()),
            Page::Failed => self.show(Page::Choose),
        }
    }

    /// Escape: from the Secure Boot page back to the disks.
    pub fn back(&self) {
        if self.page.get() == Page::SecureBoot {
            self.show(Page::Choose);
        }
    }

    /// Erase and Install on the picked disk (checked already).
    fn erase_chosen(self: &Rc<Self>) {
        if let Some(d) = self.chosen.get().and_then(|i| self.disks.get(i)) {
            let disk = d.name.clone();
            self.start(&disk);
        }
    }

    /// Runs `install-run <disk>`; progress lines come back through a channel polled on the UI thread.
    fn start(self: &Rc<Self>, disk: &str) {
        let argv = install::install_command(disk, has_sudo());
        eprintln!("wol-installer: {}", argv.join(" "));
        let child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn();
        let Ok(mut child) = child else {
            self.show_failed(Stage::Prepare);
            return;
        };
        *self.stdin.borrow_mut() = child.stdin.take();
        let out = child.stdout.take();
        let (tx, rx) = std::sync::mpsc::channel::<Event>();
        std::thread::spawn(move || {
            if let Some(out) = out {
                let mut r = std::io::BufReader::new(out);
                let mut buf = Vec::new();
                // bsdinstall redraws with '\r'; split on both.
                while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
                    for part in String::from_utf8_lossy(&buf).split('\r') {
                        if part.contains(install::BSDINSTALL_ERROR) {
                            let _ = tx.send(Event::ErrorBox);
                        }
                        if let Some(r) = install::parse_line(part) {
                            eprintln!("wol-installer: {:?} {}% {}", r.stage, r.percent, r.message);
                            let _ = tx.send(Event::Report(r));
                        }
                    }
                    buf.clear();
                }
            }
            let ok = child.wait().is_ok_and(|s| s.success());
            let _ = tx.send(Event::Exit(ok));
        });
        *self.report.borrow_mut() = (Report::new(Stage::Prepare, 0), Instant::now());
        self.sample.set(None);
        self.show_progress(&Report::new(Stage::Prepare, 0), None);
        let me = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(s) = me.upgrade() else {
                return glib::ControlFlow::Break;
            };
            while let Ok(e) = rx.try_recv() {
                match e {
                    Event::Report(r) if r.stage == Stage::Failed => {
                        let at = s.report.borrow().0.stage;
                        s.show_failed(at);
                    }
                    Event::Report(r) => {
                        // The bar's clock runs per stage (Files creeps from when it began).
                        let mut cur = s.report.borrow_mut();
                        let since = if r.stage == cur.0.stage {
                            cur.1
                        } else {
                            Instant::now()
                        };
                        *cur = (r, since);
                    }
                    Event::ErrorBox => {
                        if let Some(i) = &mut *s.stdin.borrow_mut() {
                            let _ = i.write_all(b"\r");
                        }
                    }
                    Event::Exit(ok) => {
                        s.stdin.borrow_mut().take();
                        if ok {
                            s.show(Page::Done);
                        } else if s.page.get() != Page::Failed {
                            let at = s.report.borrow().0.stage;
                            s.show_failed(at);
                        }
                        return glib::ControlFlow::Break;
                    }
                }
            }
            s.tick();
            glib::ControlFlow::Continue
        });
    }

    /// Bar and time left from the last report.
    fn tick(&self) {
        if self.page.get() != Page::Installing {
            return;
        }
        let (r, since) = self.report.borrow().clone();
        let f = install::fraction(&r, since.elapsed().as_secs_f64());
        let now = Instant::now();
        // Apps are most of the time and report steadily: the estimate starts with them.
        if r.stage == Stage::Packages && self.sample.get().is_none() {
            self.sample.set(Some((f, now)));
        }
        let left = self.sample.get().and_then(|(f0, t0)| {
            install::seconds_left((f0, 0.0), (f, now.duration_since(t0).as_secs_f64()))
        });
        self.progress.set(
            f,
            &format!(
                "{} {}",
                install::doing(r.stage),
                install::remaining_text(left)
            ),
        );
    }
}

/// The warning under the list: what is lost, other systems by name (`install-check`).
fn erase_warning(d: &Disk, c: Option<&Check>) -> String {
    match c.map(|c| c.others.as_slice()).unwrap_or_default() {
        [] => format!(
            "Everything on “{}” ({}) will be erased. This can't be undone.",
            d.title(),
            install::size_text(d.bytes)
        ),
        others => format!("{} This can't be undone.", install::erase_warning(others)),
    }
}

fn output(argv: &[String]) -> String {
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn has_sudo() -> bool {
    std::path::Path::new("/usr/local/bin/sudo").exists()
}

/// `install-check <disk>` (read-only, a few milliseconds).
fn check_disk(disk: &str) -> Check {
    install::parse_check(&output(&install::check_command(disk, has_sudo())))
}

/// Disks `install-disks` allows, described by `geom disk list`, with what `install-check` found.
pub fn system_disks() -> (Vec<Disk>, Vec<Check>) {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let disks = install::parse_disks(
        &output(&s(&["geom", "disk", "list"])),
        &output(&s(&[install::DISKS])),
    );
    let checks = disks.iter().map(|d| check_disk(&d.name)).collect();
    (disks, checks)
}

pub fn run() -> glib::ExitCode {
    a::run("org.wolos.installer", "installer", |win| {
        let (disks, checks) = system_disks();
        let s = Installer::new(disks, checks);
        // Over the live desktop: shown at once, no fade from black.
        let shown = reveal::Reveal::new(&s.root, false);
        win.set_child(Some(&shown.root));
        // Closing mid-install would cut the installer's output and stop it: not while installing.
        let s2 = s.clone();
        win.connect_close_request(move |_| {
            if s2.installing() {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        // Installing: no idle screen-off, lock or sleep. The compositor's idle inhibitor for the
        // window, and the session's ScreenSaver inhibit, which hypridle honours whichever
        // workspace is showing. install-run holds the lid. Released on done or stopped.
        let (app, w) = (win.application(), win.clone());
        let held: RefCell<Option<(u32, std::sync::mpsc::Sender<()>)>> = RefCell::new(None);
        s.set_keep_awake(move |on| {
            if on {
                // IDLE only: on Wayland GTK maps it to the compositor's idle inhibitor (no D-Bus
                // round trip on the UI thread); sleep from idle is hypridle's, held below.
                let flags = gtk::ApplicationInhibitFlags::IDLE;
                let cookie = app.as_ref().map_or(0, |a| {
                    a.inhibit(Some(&w), flags, Some(install::AWAKE_REASON))
                });
                *held.borrow_mut() = Some((cookie, screensaver_inhibit()));
            } else if let Some((cookie, release)) = held.borrow_mut().take() {
                if let (Some(a), true) = (&app, cookie != 0) {
                    a.uninhibit(cookie);
                }
                let _ = release.send(());
            }
        });
        let s3 = s.clone();
        a::keys(win, move || s.next(), move || s3.back());
    })
}

/// `org.freedesktop.ScreenSaver.Inhibit` on the session bus (hypridle serves it) until the sender
/// is used or dropped; the connection lives on its own thread, so a slow bus never blocks the UI.
fn screensaver_inhibit() -> std::sync::mpsc::Sender<()> {
    const SS: &str = "org.freedesktop.ScreenSaver";
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let Ok(c) = zbus::blocking::Connection::session() else {
            return;
        };
        let cookie = c
            .call_method(
                Some(SS),
                "/org/freedesktop/ScreenSaver",
                Some(SS),
                "Inhibit",
                &("Install WolOS", install::AWAKE_REASON),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<u32>().ok());
        let _ = rx.recv();
        if let Some(k) = cookie {
            let _ = c.call_method(
                Some(SS),
                "/org/freedesktop/ScreenSaver",
                Some(SS),
                "UnInhibit",
                &(k,),
            );
        }
    });
    tx
}
