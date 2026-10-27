//! Helper approvals (archicture/modules/automation.md): a helper (a `wol mcp` client, a
//! `WOL_HELPER` script) asks before a change (`approve <json>` on the shell socket). One glass
//! sheet names the helper and says what will change, in plain words. User-level changes: Deny,
//! Always Allow, Allow Once; the helper runs them. Administrator changes: the password, then they
//! run here (wol-admind), never in the helper. One sheet on screen, [`MAX_WAITING`] behind it,
//! more get Busy; each request is denied [`APPROVAL_TIMEOUT_SECS`] after it arrived. The answer
//! goes back on the request's connection and the requester writes the audit record (this side
//! only logs an administrator change it ran when the requester is gone).
use crate::kit::{surface, widgets as w};
use crate::system::{ActionError, System};
use gtk::{gio, glib, prelude::*};
use shared::shell::{APPROVAL_TIMEOUT_SECS, Answer, Approval, ApprovalRequest};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use wol_core::automation::{Outcome, Record, audit, policy};

/// Requests waiting beyond the one on screen; more are refused (a helper cannot flood the screen).
pub const MAX_WAITING: usize = 4;

/// Requests in arrival order; the front one is on screen. `T` is how to answer it.
pub struct Queue<T> {
    items: VecDeque<(u64, Approval, T)>,
    next: u64,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            next: 0,
        }
    }
}

impl<T> Queue<T> {
    /// Its id, or the request back when [`MAX_WAITING`] already wait.
    pub fn push(&mut self, a: Approval, t: T) -> Result<u64, (Approval, T)> {
        if self.items.len() > MAX_WAITING {
            return Err((a, t));
        }
        self.next += 1;
        self.items.push_back((self.next, a, t));
        Ok(self.next)
    }
    pub fn front(&self) -> Option<&Approval> {
        self.items.front().map(|i| &i.1)
    }
    pub fn front_id(&self) -> Option<u64> {
        self.items.front().map(|i| i.0)
    }
    /// Requests behind the one on screen.
    pub fn waiting(&self) -> usize {
        self.items.len().saturating_sub(1)
    }
    pub fn pop_front(&mut self) -> Option<(Approval, T)> {
        self.items.pop_front().map(|i| (i.1, i.2))
    }
    /// Removes request `id`: (was on screen, request, answerer).
    pub fn remove(&mut self, id: u64) -> Option<(bool, Approval, T)> {
        let at = self.items.iter().position(|i| i.0 == id)?;
        self.items.remove(at).map(|i| (at == 0, i.1, i.2))
    }
}

/// `script:backup` -> `backup`; MCP clients by their own name.
fn display_name(helper: &str) -> &str {
    helper.strip_prefix("script:").unwrap_or(helper)
}

/// The sheet's title: who asks, for what.
pub fn title(a: &Approval) -> String {
    format!(
        "“{}” wants to {}.",
        display_name(&a.helper),
        a.request.describe()
    )
}

/// The line under the title: what the choices mean, and how many more wait.
pub fn detail(a: &Approval, waiting: usize) -> String {
    let mut d = match &a.request {
        ApprovalRequest::Change { capability, .. } => match policy::label(capability) {
            Some(l) => {
                let mut c = l.chars();
                let first = c.next().map(|f| f.to_lowercase().collect::<String>());
                format!(
                    "Always Allow lets it {}{} without asking again.",
                    first.unwrap_or_default(),
                    c.as_str()
                )
            }
            None => "Allow Once lets it do this one time.".into(),
        },
        _ => "Enter your password to allow this.".into(),
    };
    match waiting {
        0 => {}
        1 => d.push_str(" 1 more request is waiting."),
        n => d.push_str(&format!(" {n} more requests are waiting.")),
    }
    d
}

/// Shows (true) or hides the surface holding the approvals.
type ShowFn = Box<dyn Fn(bool)>;

pub struct Approvals {
    pub root: gtk::Box,
    sheet: w::AuthSheet,
    sys: Rc<dyn System>,
    queue: RefCell<Queue<async_channel::Sender<Answer>>>,
    /// The administrator change on screen is running (wol-admind): its result answers it, not the
    /// timeout.
    running: Cell<bool>,
    /// Id of the request the sheet shows: a new arrival behind it never resets the password field.
    shown: Cell<Option<u64>>,
    visible: RefCell<Option<ShowFn>>,
}

impl Approvals {
    pub fn new(sys: Rc<dyn System>) -> Rc<Self> {
        let root = w::glass_panel(0);
        root.add_css_class("approval");
        root.set_width_request(w::CONTROL_CENTER_WIDTH + w::S8);
        let sheet = w::authenticate_sheet("A helper");
        sheet.revealer.set_reveal_child(true);
        root.append(&sheet.revealer);
        let a = Rc::new(Self {
            root,
            sheet,
            sys,
            queue: Default::default(),
            running: Cell::new(false),
            shown: Cell::new(None),
            visible: Default::default(),
        });
        let weak = Rc::downgrade(&a);
        let on = move |f: fn(&Rc<Self>)| {
            let weak = weak.clone();
            move || {
                if let Some(a) = weak.upgrade() {
                    f(&a)
                }
            }
        };
        let deny = on(|a| a.deny());
        a.sheet.cancel.connect_clicked(move |_| deny());
        let allow = on(|a| a.allow());
        a.sheet.allow.connect_clicked(move |_| allow());
        let always = on(|a| a.always());
        a.sheet.always.connect_clicked(move |_| always());
        let submit = on(|a| a.allow());
        a.sheet.field.connect_activate(move |_| submit());
        a
    }

    /// Queues `a`; it shows now when nothing else is on screen. The receiver yields the answer.
    pub fn ask(self: &Rc<Self>, a: Approval) -> async_channel::Receiver<Answer> {
        let (tx, rx) = async_channel::bounded(1);
        let pushed = self.queue.borrow_mut().push(a, tx);
        match pushed {
            Err((_, tx)) => {
                let _ = tx.try_send(Answer::Busy);
            }
            Ok(id) => {
                let weak = Rc::downgrade(self);
                glib::timeout_add_seconds_local_once(APPROVAL_TIMEOUT_SECS, move || {
                    if let Some(a) = weak.upgrade() {
                        a.expire(id);
                    }
                });
                self.show_front();
            }
        }
        rx
    }

    fn expire(&self, id: u64) {
        if self.running.get() && self.queue.borrow().front_id() == Some(id) {
            return;
        }
        let gone = self.queue.borrow_mut().remove(id);
        if let Some((_, _, tx)) = gone {
            let _ = tx.try_send(Answer::TimedOut);
            self.show_front();
        }
    }

    /// Fills the sheet for the request on screen (or hides it).
    fn show_front(&self) {
        let q = self.queue.borrow();
        let (Some(a), id) = (q.front().cloned(), q.front_id()) else {
            drop(q);
            self.shown.set(None);
            self.set_visible(false);
            return;
        };
        let waiting = q.waiting();
        drop(q);
        let fresh = self.shown.replace(id) != id;
        let s = &self.sheet;
        let admin = a.request.is_admin();
        s.title.set_label(&title(&a));
        s.why.set_label(&detail(&a, waiting));
        s.icon.set_icon_name(Some(if admin {
            "system-lock-screen-symbolic"
        } else {
            "security-high-symbolic"
        }));
        s.field.set_visible(admin);
        s.always.set_visible(!admin);
        s.cancel.set_label("Deny");
        s.allow
            .set_label(if admin { "Allow" } else { "Allow Once" });
        self.set_visible(true);
        if !fresh {
            return;
        }
        self.set_message("");
        s.field.set_text("");
        s.allow.set_sensitive(true);
        // A stray Enter or Space while the sheet takes the keyboard must never say yes.
        if admin {
            s.field.grab_focus();
        } else {
            s.cancel.grab_focus();
        }
    }

    fn set_message(&self, text: &str) {
        self.sheet.message.set_label(text);
        self.sheet.message.set_visible(!text.is_empty());
    }

    fn set_visible(&self, on: bool) {
        if let Some(f) = &*self.visible.borrow() {
            f(on);
        }
    }

    /// Answers the request on screen, then shows the next one.
    fn answer(&self, ans: Answer) {
        self.running.set(false);
        let done = self.queue.borrow_mut().pop_front();
        if let Some((_, tx)) = done {
            let _ = tx.try_send(ans);
        }
        self.show_front();
    }

    fn front(&self) -> Option<Approval> {
        self.queue.borrow().front().cloned()
    }

    pub fn deny(&self) {
        if self.front().is_some() && !self.running.get() {
            self.answer(Answer::Deny);
        }
    }

    fn always(&self) {
        let Some(a) = self.front() else { return };
        let ApprovalRequest::Change { capability, .. } = &a.request else {
            return;
        };
        match self.sys.allow_helper_always(&a.helper, capability) {
            Ok(()) => self.answer(Answer::AllowAlways),
            // Not saved: allow this once; it asks again next time.
            Err(e) => {
                eprintln!("wol-shell: Always Allow not saved: {}", e.message());
                self.answer(Answer::AllowOnce)
            }
        }
    }

    fn allow(self: &Rc<Self>) {
        let Some(a) = self.front() else { return };
        if self.running.get() {
            return;
        }
        if !a.request.is_admin() {
            return self.answer(Answer::AllowOnce);
        }
        let pw = self.sheet.field.text().to_string();
        self.sheet.field.set_text("");
        if pw.is_empty() {
            self.set_message("Enter your password.");
            return;
        }
        self.running.set(true);
        self.sheet.allow.set_sensitive(false);
        let rx = self.sys.approve(a.request, pw);
        let me = self.clone();
        glib::spawn_future_local(async move {
            let r = rx.recv().await.unwrap_or(Err(ActionError::Failed));
            me.sheet.allow.set_sensitive(true);
            match r {
                Ok(()) => me.answer(Answer::Done),
                Err(e @ (ActionError::Denied | ActionError::NeedsAuth)) => {
                    me.running.set(false);
                    me.set_message(e.message());
                    me.sheet.field.grab_focus();
                }
                Err(e) => me.answer(Answer::Failed {
                    message: e.message().into(),
                }),
            }
        });
    }
}

/// Audit record the shell writes itself: only an administrator change it ran for a requester
/// that is gone (the requester logs everything else). Never the Wi-Fi password.
fn log(a: &Approval, o: Outcome, detail: &str) {
    let r = Record {
        t: wol_core::journal::now(),
        helper: a.helper.clone(),
        via: "shell".into(),
        tool: a.tool.clone(),
        args: Vec::new(),
        result: o,
        detail: format!("{}: {detail}", a.request.describe()),
    };
    if let Err(e) = audit::append(&wol_core::journal::state_home(), &r) {
        eprintln!("wol-shell: helper audit log: {e}");
    }
}

/// wol-shell: the approval surface (centred, takes the keyboard while it asks) and the socket's
/// `approve` handler: answers on the request's connection, then closes it.
pub fn handler(app: &gtk::Application, sys: Rc<dyn System>) -> super::ipc::Approver {
    let ap = Approvals::new(sys);
    let win = super::window(
        app,
        "panel",
        surface::Spec::panel(
            "wol-panel-approval",
            [false; 4],
            surface::Keyboard::Exclusive,
        ),
    );
    win.set_child(Some(&ap.root));
    win.set_hide_on_close(true);
    {
        let a = ap.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                a.deny();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(keys);
    }
    {
        let a = ap.clone();
        win.connect_close_request(move |_| {
            a.deny();
            glib::Propagation::Proceed
        });
    }
    *ap.visible.borrow_mut() = Some(Box::new(move |on| {
        if on {
            win.present();
        } else {
            win.set_visible(false);
        }
    }));
    Rc::new(move |json: String, conn: gio::SocketConnection| {
        let Some(a) = Approval::parse(&json) else {
            eprintln!("wol-shell: bad approval request");
            return;
        };
        let rx = ap.ask(a.clone());
        glib::spawn_future_local(async move {
            let ans = rx.recv().await.unwrap_or(Answer::Deny);
            let out = conn.output_stream();
            if let Err(e) = out
                .write_all_future(ans.line().into_bytes(), glib::Priority::DEFAULT)
                .await
            {
                eprintln!("wol-shell: approval answer: {}", e.1);
                if ans == Answer::Done {
                    log(&a, Outcome::Approved, "ran after the helper had gone");
                }
            }
            let _ = conn.close_future(glib::Priority::DEFAULT).await;
        });
    })
}

/// The sheet as it asks for `a` (goldens, previews).
pub fn content(sys: Rc<dyn System>, a: Approval) -> gtk::Widget {
    let ap = Approvals::new(sys);
    let _answer = ap.ask(a);
    // A still picture: no blinking caret.
    ap.sheet.field.set_can_focus(false);
    ap.root.clone().upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::shell::ApprovalRequest as R;

    fn change(helper: &str) -> Approval {
        Approval {
            helper: helper.into(),
            tool: "appearance set".into(),
            request: R::Change {
                capability: "appearance".into(),
                what: "switch to Dark appearance".into(),
            },
        }
    }

    /// One on screen, MAX_WAITING behind it, then Busy; expiry removes the right one.
    #[test]
    fn queue_lines_up_refuses_floods_and_expires() {
        let mut q: Queue<u8> = Queue::default();
        let ids: Vec<u64> = (0..=MAX_WAITING as u8)
            .map(|i| q.push(change("a"), i).unwrap())
            .collect();
        assert_eq!(q.waiting(), MAX_WAITING);
        assert!(q.push(change("flood"), 99).is_err(), "Busy");
        assert_eq!(q.front_id(), Some(ids[0]));
        assert_eq!(q.remove(ids[2]).map(|r| (r.0, r.2)), Some((false, 2)));
        assert_eq!(q.remove(ids[2]), None, "expired once");
        assert_eq!(q.remove(ids[0]).map(|r| (r.0, r.2)), Some((true, 0)));
        assert_eq!(q.front_id(), Some(ids[1]));
        assert_eq!(q.pop_front().map(|r| r.1), Some(1));
        assert_eq!(q.waiting(), MAX_WAITING - 3);
    }

    #[test]
    fn sheet_text_names_the_helper_and_the_change() {
        assert_eq!(
            title(&change("Claude Desktop")),
            "“Claude Desktop” wants to switch to Dark appearance."
        );
        assert_eq!(
            title(&change("script:backup")),
            "“backup” wants to switch to Dark appearance."
        );
        assert_eq!(
            detail(&change("Claude Desktop"), 0),
            "Always Allow lets it change appearance and wallpaper without asking again."
        );
        let admin = Approval {
            request: R::Wifi { on: false },
            ..change("Claude")
        };
        assert_eq!(title(&admin), "“Claude” wants to turn Wi-Fi off.");
        assert_eq!(detail(&admin, 0), "Enter your password to allow this.");
        assert_eq!(
            detail(&admin, 2),
            "Enter your password to allow this. 2 more requests are waiting."
        );
        assert_eq!(
            detail(&admin, 1),
            "Enter your password to allow this. 1 more request is waiting."
        );
    }
}
