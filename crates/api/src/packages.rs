// SPDX-License-Identifier: GPL-2.0-or-later
//! wold system package service (ADR 0021): wopkg asks, wold decides with the caller's executable
//! (from the kernel) and the consent grant the call names, then runs the sealed-system pacman
//! (`wol pkg --`, `controller::pkg`: live layer, pending slot for kernel/initramfs/init) or the A/B
//! system update. wopkg itself never runs as root.
//!
//! Invariant (audit A1/B2/S1/S2): a grant means "may ask", never "may act". Every change is first
//! shown to the caller's own session as an approval sheet that names the caller by the executable
//! the kernel reported (A23), and runs only after the person at the computer says yes. The ask goes
//! out on the shower's own `subscribe account` connection (`Event::SessionAsk`, admitted by its
//! kernel-verified executable) and the answer comes back only as `session.answer` from that same
//! kind of peer: wold never dials a socket that anything of the account could have bound. Every
//! pacman transaction from the repos (install, remove, update with or without names, update with
//! holds) is an administrator sheet the shell runs through wol-admind after the password; wold
//! itself runs only the Allow-sheet changes (a staged local file, a signed release, the channel,
//! undo). No session, no answer, Deny or a timeout: nothing runs. wold never remembers a yes.
use crate::bus::Bus;
use crate::portal::Asks;
use shared::packages::{PackageCall, Verb};
use shared::portal::{Answer, Ask, Code};
use shared::shell::{Answer as Sheet, Approval, ApprovalRequest};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub const WOL: &str = "/usr/bin/wol";
/// Root-only copies of locally built packages, one fresh directory per call, so the caller cannot
/// swap the file after the check and two installs never share a name.
pub const STAGING: &str = "/var/cache/wolos/install";
pub const CHANNEL_FILE: &str = "/etc/wolos/channel";

/// What wold does on the machine (fake in tests).
pub trait System {
    /// Run `argv` as root; exit code.
    fn run(&self, argv: &[String]) -> i32;
    /// Copy `path` into a fresh directory under [`STAGING`] if it is a regular file owned by
    /// `uid`; the copy's path. None when the file is not the caller's or cannot be copied.
    fn stage(&self, path: &str, uid: u32) -> Option<String>;
    /// Forget a copy [`System::stage`] made (after the install, or when the person said no).
    fn discard(&self, staged: &str);
    fn write(&self, path: &str, text: &str) -> bool;
    /// Trusted release keys and the running release manifest's text.
    fn keys(&self) -> Vec<[u8; 32]>;
    fn running_release(&self) -> Option<String>;
    /// Show `approval` on the screen of account `uid` (its wol-shell approval sheet) and wait for
    /// the answer; None when no session of that account answers.
    fn ask(&self, uid: u32, approval: &Approval) -> Option<Sheet>;
    /// The staged installs in flight (R2C-8); this wold's [`STAGED`].
    fn staging(&self) -> &Staging {
        &STAGED
    }
}

/// The real machine (wold runs as root). Asks go to the account's shower over `bus`; `asks` holds
/// the calls waiting for their `session.answer`.
pub struct Host<'a> {
    pub bus: &'a Bus<UnixStream>,
    pub asks: &'a Asks,
}

/// Show `approval` to the shower of account `uid` (`Event::SessionAsk` on its `subscribe account`
/// connection) and wait at most `wait` for its `session.answer`, whose data is the sheet's answer.
/// None when no shower runs, it did not answer in time, or the answer is not a sheet answer.
pub fn ask_shower<W: std::io::Write + Send>(
    bus: &Bus<W>,
    asks: &Asks,
    uid: u32,
    approval: &Approval,
    wait: Duration,
) -> Option<Sheet> {
    let ask = Ask::Approval {
        approval: approval.clone(),
    };
    let a = asks.ask(bus, uid, ask, wait);
    if !a.ok {
        // R2C-9: a full table and a shower that did not answer say so, not "no session".
        return match a.message.as_deref() {
            Some(crate::portal::BUSY) => Some(Sheet::Busy),
            Some(crate::portal::NO_ANSWER) => Some(Sheet::TimedOut),
            _ => None,
        };
    }
    serde_json::from_value(a.data?).ok()
}

impl System for Host<'_> {
    fn run(&self, argv: &[String]) -> i32 {
        let Some((prog, args)) = argv.split_first() else {
            return 127;
        };
        crate::reaper::run(crate::privileged::privileged_command(prog).args(args))
    }

    fn stage(&self, path: &str, uid: u32) -> Option<String> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        // O_NOFOLLOW + fstat on the opened file: what is checked is what is copied.
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        let md = f.metadata().ok()?;
        if !md.is_file() || md.uid() != uid {
            return None;
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(STAGING)
            .ok()?;
        // Audit S16: a fresh directory per call (mkdir fails when the name exists), so two
        // installs of the same file name never overwrite each other between staging and pacman.
        let dir = std::path::Path::new(STAGING).join(shared::jobs::new_nonce().ok()?);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;
        // Audit S15: the caller's file name is joined under the fresh directory through
        // safe_join (no `..`, no separator, nothing resolving outside it).
        let name = path.rsplit('/').next()?;
        let dest = shared::safe_path::safe_join_str(&dir, name).ok()?;
        let copied = (|| {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&dest)
                .ok()?;
            // Audit A24: bounded; one byte past the cap means the file grew after the check.
            use std::io::Read;
            let n = std::io::copy(&mut (&mut f).take(MAX_STAGED + 1), &mut out).ok()?;
            (n <= MAX_STAGED).then_some(())?;
            out.sync_all().ok()
        })();
        match (copied, dest.to_str()) {
            (Some(()), Some(d)) => Some(d.to_owned()),
            _ => {
                let _ = std::fs::remove_dir_all(&dir);
                None
            }
        }
    }

    fn discard(&self, staged: &str) {
        let dir = std::path::Path::new(staged).parent();
        if let Some(d) = dir.filter(|d| d.parent() == Some(std::path::Path::new(STAGING))) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    fn write(&self, path: &str, text: &str) -> bool {
        // R2C-23: a unique temp name per call, then rename (never a shared `{path}.tmp`).
        crate::admind::atomic_write(std::path::Path::new(path), 0o644, text.as_bytes()).is_ok()
    }

    fn keys(&self) -> Vec<[u8; 32]> {
        wol_core::package::read_keys(std::path::Path::new(shared::package::KEYS_DIR))
    }

    fn running_release(&self) -> Option<String> {
        std::fs::read_to_string(wol_core::update::check::RELEASE_FILE).ok()
    }

    fn ask(&self, uid: u32, approval: &Approval) -> Option<Sheet> {
        let wait = Duration::from_secs(shared::shell::APPROVAL_WAIT_SECS.into());
        ask_shower(self.bus, self.asks, uid, approval, wait)
    }
}

/// Largest locally built package wold copies into [`STAGING`] (bytes): 4 GiB.
pub const MAX_STAGED: u64 = 4 << 30;
/// Staged installs of one account in flight at once (R2C-8): with [`MAX_STAGED`] per copy, one
/// account holds at most 4 GiB of copies, however many calls it makes.
pub const STAGED_PER_UID: usize = 1;
/// Staged installs of all accounts in flight at once: at most 8 GiB of copies under [`STAGING`].
pub const STAGED_TOTAL: usize = 2;

/// Accounts with a staged install in flight (R2C-8). Taken before the copy is made, given back
/// when the call ends (installed, refused or failed).
#[derive(Default)]
pub struct Staging(std::sync::Mutex<Vec<u32>>);

/// One staged install in flight; dropping it frees the place.
pub struct StagingSlot<'a>(&'a Staging, u32);

impl Staging {
    pub const fn new() -> Self {
        Self(std::sync::Mutex::new(Vec::new()))
    }

    /// A place for `uid`, or None when it already has [`STAGED_PER_UID`] or all
    /// [`STAGED_TOTAL`] are taken.
    pub fn enter(&self, uid: u32) -> Option<StagingSlot<'_>> {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let mine = g.iter().filter(|u| **u == uid).count();
        if mine >= STAGED_PER_UID || g.len() >= STAGED_TOTAL {
            return None;
        }
        g.push(uid);
        Some(StagingSlot(self, uid))
    }
}

impl Drop for StagingSlot<'_> {
    fn drop(&mut self) {
        let mut g = self.0.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(i) = g.iter().position(|u| *u == self.1) {
            g.swap_remove(i);
        }
    }
}

/// The places of this wold.
pub static STAGED: Staging = Staging::new();

/// Is `name` one of [`Host::stage`]'s directories (a 32-digit hex nonce)? wol-admind's own
/// staging directories under the same root carry dashes and are never matched.
fn is_wold_stage(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Remove the copies a previous wold left under `root` (R2C-8: a crash or a stop while a sheet
/// was open). wold is the only writer of these names, so at start every one is stale. Returns
/// how many were removed.
pub fn sweep_staging(root: &std::path::Path) -> usize {
    let mut n = 0;
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = e.file_name();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if is_dir
            && name.to_str().is_some_and(is_wold_stage)
            && std::fs::remove_dir_all(e.path()).is_ok()
        {
            n += 1;
        }
    }
    n
}

/// The sentence end of the sheet ("<app> wants to …") for one call.
pub fn what(call: &PackageCall) -> String {
    let list = |n: &[String]| n.join(", ");
    match call {
        PackageCall::Change {
            verb: Verb::Install,
            names,
        } => format!("install {} for every account", list(names)),
        PackageCall::Change {
            verb: Verb::Remove,
            names,
        } => format!("remove {} from this computer", list(names)),
        PackageCall::Change {
            verb: Verb::Update,
            names,
        } if names.is_empty() => "update all system apps".into(),
        PackageCall::Change { names, .. } => format!("update {}", list(names)),
        PackageCall::UpdateHolding { .. } => "update all system apps".into(),
        PackageCall::InstallFile { path } => format!(
            "install the package “{}”, which WolOS has not reviewed",
            path.rsplit('/').next().unwrap_or(path)
        ),
        PackageCall::SystemUpdate { .. } => "install a WolOS system update".into(),
        PackageCall::SystemChannel { channel } => {
            format!("switch system updates to the “{channel}” channel")
        }
        PackageCall::SystemRollback {} => "undo the last system update".into(),
    }
}

/// The sheet for one call. Every pacman transaction from the repos is an administrator sheet the
/// shell runs through wol-admind after the password ([`ApprovalRequest::Packages`]; audit S2: an
/// "update" with names installs what is not installed yet, so it is an install). The rest (a local
/// file, a system release, the channel, undo) are Allow sheets after which wold runs the change.
pub fn request(call: &PackageCall) -> ApprovalRequest {
    let packages = |verb: &str, names: &[String]| ApprovalRequest::Packages {
        verb: verb.into(),
        names: names.to_vec(),
    };
    match call {
        PackageCall::Change {
            verb: Verb::Install,
            names,
        } => packages("install", names),
        PackageCall::Change {
            verb: Verb::Remove,
            names,
        } => packages("remove", names),
        PackageCall::Change {
            verb: Verb::Update,
            names,
        } => packages("update", names),
        PackageCall::UpdateHolding { hold } if hold.is_empty() => packages("update", hold),
        PackageCall::UpdateHolding { hold } => packages("update-holding", hold),
        _ => ApprovalRequest::Change {
            capability: call.grant().to_owned(),
            what: what(call),
        },
    }
}

/// What the answer lets wold do.
#[derive(Debug, PartialEq, Eq)]
enum Approved {
    /// The person allowed it; wold runs the change.
    RunHere,
    /// The shell ran it through wol-admind after the password.
    Done,
}

/// Asks the person at the computer. Always Allow counts once: wold keeps no memory of it (a
/// grant is permission to ask). An answer that does not fit the sheet kind is a no.
fn approved(sys: &dyn System, uid: u32, app: &str, call: &PackageCall) -> Result<Approved, Answer> {
    let request = request(call);
    let admin = request.is_admin();
    let approval = Approval {
        // A23: the kernel's executable path, never a name the app chose.
        helper: app.to_owned(),
        tool: format!("wopkg {}", call.grant()),
        request,
    };
    let no = |m: &str| Err(Answer::err(Code::PermissionDenied, m.to_owned()));
    match sys.ask(uid, &approval) {
        Some(Sheet::AllowOnce | Sheet::AllowAlways) if !admin => Ok(Approved::RunHere),
        Some(Sheet::Done) if admin => Ok(Approved::Done),
        Some(Sheet::Failed { .. }) if admin => Err(Answer::err(
            Code::Internal,
            "The change did not finish. Nothing was changed; try again.",
        )),
        Some(Sheet::Deny) => no("You said no on screen, so nothing was changed."),
        Some(Sheet::Busy) => no("Too many requests are waiting on screen. Try again in a moment."),
        Some(Sheet::TimedOut) => {
            no("Nobody answered on screen in time, so nothing was changed. Try again.")
        }
        Some(_) | None => no(
            "Nobody could approve this on screen (no WolOS session is showing), so nothing was changed.",
        ),
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

/// Is this call line one of wopkg's (`package.*`, `system.*`)? Everything else is a libwol call.
pub fn is_package_call(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json.trim())
        .ok()
        .and_then(|v| v.get("call")?.as_str().map(str::to_owned))
        .is_some_and(|c| c.starts_with("package.") || c.starts_with("system."))
}

fn denied(grant: &str, app: &str) -> Answer {
    let what = match grant {
        "packages-install" => "install system apps",
        "packages-remove" => "remove system apps",
        "packages-update" => "update the system",
        _ => "install apps built on this computer",
    };
    let name = app.rsplit('/').next().unwrap_or(app);
    Answer::err(
        Code::PermissionDenied,
        format!(
            "{name} is not allowed to {what}. Allow it in System Settings > Privacy & Security > Software."
        ),
    )
}

/// One call line's JSON -> answer. `caller` = (uid, canonical executable), from the kernel.
pub fn handle(
    json: &str,
    caller: Option<(u32, &str)>,
    granted: &dyn Fn(&str, &str) -> bool,
    sys: &dyn System,
) -> Answer {
    let call = match PackageCall::decode(json.as_bytes()) {
        Ok(c) => c,
        Err(m) => return Answer::err(Code::InvalidArgument, m),
    };
    let Some((uid, app)) = caller else {
        return Answer::err(
            Code::PermissionDenied,
            "WolOS could not tell which app asked, so it did nothing.",
        );
    };
    if !granted(app, call.grant()) {
        return denied(call.grant(), app);
    }
    // Audit S16: a local file is copied before the sheet shows, so what the person allows is the
    // copy that will be installed, whatever happens to the original afterwards.
    // R2C-8: one staged install per account in flight, and a small cap for all of them, taken
    // before anything is copied and held until this call ends.
    let _slot = match &call {
        PackageCall::InstallFile { .. } => match sys.staging().enter(uid) {
            Some(slot) => Some(slot),
            None => {
                return Answer::err(
                    Code::Unavailable,
                    "Another package install is still waiting. Try again when it is done.",
                );
            }
        },
        _ => None,
    };
    let staged = match &call {
        PackageCall::InstallFile { path } => match sys.stage(path, uid) {
            Some(copy) => Some(copy),
            None => {
                return Answer::err(
                    Code::PermissionDenied,
                    "That package file is not yours, so it was not installed.",
                );
            }
        },
        _ => None,
    };
    let yes = approved(sys, uid, app, &call);
    let run_here = matches!(yes, Ok(Approved::RunHere));
    if !run_here && let Some(copy) = &staged {
        sys.discard(copy);
    }
    match yes {
        Ok(Approved::RunHere) => {}
        Ok(Approved::Done) => return Answer::ok(),
        Err(no) => return no,
    }
    let ran = |argv: Vec<String>, what: &str| {
        if sys.run(&argv) == 0 {
            Answer::ok()
        } else {
            Answer::err(
                Code::Internal,
                format!("{what} did not finish. Nothing was changed; try again."),
            )
        }
    };
    match call {
        // Administrator sheets: the shell ran them (Done above). Never here, whatever the answer.
        PackageCall::Change { .. } | PackageCall::UpdateHolding { .. } => Answer::err(
            Code::PermissionDenied,
            "This change needs the administrator password, so nothing was changed.",
        ),
        PackageCall::InstallFile { .. } => {
            let Some(copy) = staged else {
                return Answer::err(Code::Internal, "The install did not finish. Try again.");
            };
            let a = ran(
                s(&[WOL, "pkg", "--", "-U", "--noconfirm", "--", &copy]),
                "The install",
            );
            sys.discard(&copy);
            a
        }
        PackageCall::SystemUpdate {
            manifest,
            signature,
        } => {
            // CORE-7: never slices; a malformed signature decodes to nothing and fails verification.
            let sig = wol_core::update::check::unhex(&signature).unwrap_or_default();
            let m = match wol_core::update::check::Manifest::verify(
                manifest.as_bytes(),
                &sig,
                &sys.keys(),
            ) {
                Ok(m) => m,
                Err(_) => {
                    return Answer::err(
                        Code::PermissionDenied,
                        "That release is not signed by WolOS, so it was not installed.",
                    );
                }
            };
            let running = sys
                .running_release()
                .and_then(|t| wol_core::update::check::Manifest::parse(&t));
            if running.is_some_and(|r| r.snapshot >= m.snapshot) {
                return Answer::err(
                    Code::InvalidArgument,
                    "This computer already runs that release or a newer one. To go back, use Undo Update.",
                );
            }
            ran(
                s(&[WOL, "update", "apply", "--release", &m.snapshot, "--yes"]),
                "The system update",
            )
        }
        PackageCall::SystemChannel { channel } => {
            if sys.write(CHANNEL_FILE, &format!("{channel}\n")) {
                Answer::ok()
            } else {
                Answer::err(Code::Internal, "The update channel could not be changed.")
            }
        }
        PackageCall::SystemRollback {} => ran(s(&[WOL, "update", "rollback"]), "Undo Update"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_package_is_held_through_a_full_update() {
        // Audit S2: holds are an administrator sheet; wol-admind runs `-Syu --ignore=…`.
        let call = PackageCall::UpdateHolding {
            hold: vec!["firefox".into(), "gimp".into()],
        };
        assert_eq!(
            request(&call),
            ApprovalRequest::Packages {
                verb: "update-holding".into(),
                names: vec!["firefox".into(), "gimp".into()]
            }
        );
        let none = PackageCall::UpdateHolding { hold: vec![] };
        assert_eq!(
            request(&none),
            ApprovalRequest::Packages {
                verb: "update".into(),
                names: vec![]
            }
        );
        let bad = PackageCall::UpdateHolding {
            hold: vec!["--root=/x".into()],
        };
        assert!(bad.validate().is_err());
        assert_eq!(bad.grant(), "packages-update");
    }
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        ran: RefCell<Vec<Vec<String>>>,
        wrote: RefCell<Vec<(String, String)>>,
        seed: [u8; 32],
        running: Option<String>,
        /// The sheet's answer; None = no session answered.
        sheet: Option<Sheet>,
        asked: RefCell<Vec<Approval>>,
        discarded: RefCell<Vec<String>>,
        staging: Staging,
    }

    fn yes(seed: [u8; 32]) -> Fake {
        Fake {
            seed,
            sheet: Some(Sheet::AllowOnce),
            ..Default::default()
        }
    }
    impl System for Fake {
        fn run(&self, a: &[String]) -> i32 {
            self.ran.borrow_mut().push(a.to_vec());
            0
        }
        fn stage(&self, p: &str, uid: u32) -> Option<String> {
            (uid == 1000).then(|| format!("{STAGING}/n/{}", p.rsplit('/').next().unwrap()))
        }
        fn discard(&self, staged: &str) {
            self.discarded.borrow_mut().push(staged.into());
        }
        fn write(&self, p: &str, t: &str) -> bool {
            self.wrote.borrow_mut().push((p.into(), t.into()));
            true
        }
        fn keys(&self) -> Vec<[u8; 32]> {
            vec![wol_core::package::public_of(&self.seed)]
        }
        fn running_release(&self) -> Option<String> {
            self.running.clone()
        }
        fn staging(&self) -> &Staging {
            &self.staging
        }
        fn ask(&self, _uid: u32, a: &Approval) -> Option<Sheet> {
            self.asked.borrow_mut().push(a.clone());
            // "Yes" on an administrator sheet: the shell ran it and answers Done.
            match (&self.sheet, a.request.is_admin()) {
                (Some(Sheet::AllowOnce), true) => Some(Sheet::Done),
                (s, _) => s.clone(),
            }
        }
    }

    /// R2C-8: a second staged install of the same account is refused while the first waits;
    /// the cap for all accounts holds; the place is given back when the call ends.
    #[test]
    fn staged_installs_are_limited_per_account() {
        let t = Staging::new();
        let a = t.enter(1000).unwrap();
        assert!(t.enter(1000).is_none(), "one per account");
        let b = t.enter(1001).unwrap();
        assert!(t.enter(1002).is_none(), "the cap for all accounts");
        drop(a);
        assert!(t.enter(1000).is_some());
        drop(b);

        // Through handle: uid 1000 already has one in flight.
        let f = yes([0; 32]);
        let held = f.staging.enter(1000).unwrap();
        let call = r#"{"call":"package.install-file","path":"/home/a/x.pkg.tar.zst"}"#;
        let all = |_: &str, _: &str| true;
        let a = handle(call, Some((1000, APP)), &all, &f);
        assert_eq!(a.code, Some(Code::Unavailable), "{a:?}");
        assert!(f.asked.borrow().is_empty() && f.ran.borrow().is_empty());
        drop(held);
        assert!(handle(call, Some((1000, APP)), &all, &f).ok, "free again");
    }

    /// R2C-8: at start wold removes the copies a previous run left, never wol-admind's.
    #[test]
    fn stale_staging_directories_are_swept_at_start() {
        let root = std::env::temp_dir().join(format!("wol-staging-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mine = root.join("0123456789abcdef0123456789abcdef");
        let admind = root.join("77-123-0123456789abcdef0123456789abcdef");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("x.pkg.tar.zst"), b"x").unwrap();
        std::fs::create_dir_all(&admind).unwrap();
        std::fs::write(root.join("0123456789abcdef0123456789abcdee"), b"a file").unwrap();
        assert_eq!(sweep_staging(&root), 1);
        assert!(!mine.exists());
        assert!(admind.exists());
        assert_eq!(sweep_staging(&root.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_grant_only_lets_the_app_ask_and_the_person_decides() {
        let inst = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
        let all = |_: &str, _: &str| true;
        for sheet in [
            None,
            Some(Sheet::Deny),
            Some(Sheet::TimedOut),
            Some(Sheet::Busy),
            Some(Sheet::AllowAlways),
            Some(Sheet::Failed {
                message: "x".into(),
            }),
        ] {
            let f = Fake {
                sheet: sheet.clone(),
                ..Default::default()
            };
            let a = handle(inst, Some((1000, APP)), &all, &f);
            assert!(!a.ok, "{sheet:?}");
            assert!(f.ran.borrow().is_empty(), "{sheet:?}: nothing ran");
            assert_eq!(f.asked.borrow().len(), 1);
        }
        let f = yes([0; 32]);
        assert!(handle(inst, Some((1000, APP)), &all, &f).ok);
        assert!(
            f.ran.borrow().is_empty(),
            "the shell ran it through wol-admind"
        );
        let asked = f.asked.borrow();
        assert_eq!(
            asked[0].helper, APP,
            "the sheet names the kernel's executable"
        );
        assert_eq!(
            asked[0].request,
            ApprovalRequest::Packages {
                verb: "install".into(),
                names: vec!["firefox".into()]
            }
        );
        drop(asked);
        // A local file is an Allow sheet; wold runs it after the yes.
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        assert!(handle(file, Some((1000, APP)), &all, &f).ok);
        assert_eq!(f.ran.borrow().len(), 1);
        assert!(matches!(
            f.asked.borrow()[1].request,
            ApprovalRequest::Change { .. }
        ));
        // Not granted: refused without bothering the person.
        let f = yes([0; 32]);
        let none = |_: &str, _: &str| false;
        assert!(!handle(inst, Some((1000, APP)), &none, &f).ok);
        assert!(f.asked.borrow().is_empty());
    }

    /// Audit S2: an "update" with names, or with holds, installs as root, so it is an
    /// administrator sheet the shell runs through wol-admind. Allow Once is not enough: wold runs
    /// nothing itself.
    #[test]
    fn updates_with_names_or_holds_need_the_administrator_sheet() {
        let all = |_: &str, _: &str| true;
        for (line, verb, names) in [
            (
                r#"{"call":"package.change","verb":"update","names":["firefox"]}"#,
                "update",
                vec!["firefox".to_string()],
            ),
            (
                r#"{"call":"package.update-holding","hold":["gimp"]}"#,
                "update-holding",
                vec!["gimp".to_string()],
            ),
        ] {
            // A forged or confused Allow Once on an administrator sheet: refused, nothing ran.
            let f = Fake {
                sheet: Some(Sheet::AllowOnce),
                ..Default::default()
            };
            struct Plain<'a>(&'a Fake);
            impl System for Plain<'_> {
                fn run(&self, a: &[String]) -> i32 {
                    self.0.run(a)
                }
                fn stage(&self, p: &str, u: u32) -> Option<String> {
                    self.0.stage(p, u)
                }
                fn discard(&self, s: &str) {
                    self.0.discard(s)
                }
                fn write(&self, p: &str, t: &str) -> bool {
                    self.0.write(p, t)
                }
                fn keys(&self) -> Vec<[u8; 32]> {
                    self.0.keys()
                }
                fn running_release(&self) -> Option<String> {
                    self.0.running_release()
                }
                fn ask(&self, _: u32, a: &Approval) -> Option<Sheet> {
                    self.0.asked.borrow_mut().push(a.clone());
                    Some(Sheet::AllowOnce)
                }
                fn staging(&self) -> &Staging {
                    &self.0.staging
                }
            }
            let a = handle(line, Some((1000, APP)), &all, &Plain(&f));
            assert!(!a.ok, "{line}: Allow Once does not run pacman");
            assert!(f.ran.borrow().is_empty(), "{line}");
            assert_eq!(
                f.asked.borrow()[0].request,
                ApprovalRequest::Packages {
                    verb: verb.into(),
                    names: names.clone()
                },
                "{line}"
            );
            // The password sheet: the shell ran it (Done); wold still runs nothing.
            let f = yes([0; 32]);
            assert!(handle(line, Some((1000, APP)), &all, &f).ok, "{line}");
            assert!(f.ran.borrow().is_empty(), "{line}");
        }
    }

    /// Audit S1: the sheet goes to the account's shower over the bus and only a
    /// `session.answer` releases it. A socket somebody of the account bound where the shell's
    /// socket lives is never dialled, so its "allow-once" counts for nothing.
    #[test]
    fn a_fake_shell_socket_cannot_approve() {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("wol-fake-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = UnixListener::bind(dir.join("wol-shell.sock")).unwrap();
        fake.set_nonblocking(true).unwrap();
        let bus: Bus<UnixStream> = Bus::default();
        let asks = Asks::default();
        let (shower, mine) = UnixStream::pair().unwrap();
        assert!(bus.subscribe(Some(1000), Some((1000, 1)), shower));
        struct ViaBus<'a> {
            fake: Fake,
            bus: &'a Bus<UnixStream>,
            asks: &'a Asks,
        }
        impl System for ViaBus<'_> {
            fn run(&self, a: &[String]) -> i32 {
                self.fake.run(a)
            }
            fn stage(&self, p: &str, u: u32) -> Option<String> {
                self.fake.stage(p, u)
            }
            fn discard(&self, s: &str) {
                self.fake.discard(s)
            }
            fn write(&self, p: &str, t: &str) -> bool {
                self.fake.write(p, t)
            }
            fn keys(&self) -> Vec<[u8; 32]> {
                vec![]
            }
            fn running_release(&self) -> Option<String> {
                None
            }
            fn ask(&self, uid: u32, a: &Approval) -> Option<Sheet> {
                ask_shower(self.bus, self.asks, uid, a, Duration::from_millis(300))
            }
            fn staging(&self) -> &Staging {
                &self.fake.staging
            }
        }
        let sys = ViaBus {
            fake: Fake::default(),
            bus: &bus,
            asks: &asks,
        };
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        let all = |_: &str, _: &str| true;
        let a = handle(file, Some((1000, APP)), &all, &sys);
        assert!(!a.ok, "nobody answered on the bus: nothing runs");
        assert!(sys.fake.ran.borrow().is_empty());
        assert_eq!(sys.fake.discarded.borrow().len(), 1, "the copy is dropped");
        assert!(
            fake.accept().is_err(),
            "the fake shell socket was never dialled"
        );
        // The ask reached the shower, naming the kernel's executable.
        let mut line = String::new();
        std::io::BufReader::new(&mine).read_line(&mut line).unwrap();
        let Some(shared::events::Event::SessionAsk {
            uid: 1000,
            ask: Ask::Approval { approval },
            ..
        }) = shared::events::Event::parse(&line)
        else {
            panic!("{line}")
        };
        assert_eq!(approval.helper, APP);
        assert!(matches!(approval.request, ApprovalRequest::Change { .. }));
        let _ = (&mine).flush();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn manifest(snapshot: &str) -> String {
        format!(
            r#"{{"version":"1.1.0","snapshot":"{snapshot}","wolos_repo":"r","kernel":"k","image":"i","sha256":"{}"}}"#,
            "0".repeat(64)
        )
    }

    const APP: &str = "/usr/bin/wopkg";

    #[test]
    fn only_package_and_system_calls_are_ours() {
        for (line, ours) in [
            (
                r#"{"call":"package.change","verb":"install","names":["a"]}"#,
                true,
            ),
            (r#"{"call":"system.rollback"}"#, true),
            (r#"{"call":"notification.post","title":"x"}"#, false),
            ("not json", false),
        ] {
            assert_eq!(is_package_call(line), ours, "{line}");
        }
    }

    #[test]
    fn decisions() {
        let f = yes([7; 32]);
        let all = |a: &str, _: &str| a == APP;
        let only_install = |a: &str, g: &str| a == APP && g == "packages-install";
        let inst = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
        assert!(handle(inst, Some((1000, APP)), &only_install, &f).ok);
        assert!(
            f.ran.borrow().is_empty(),
            "installs run in the shell (wol-admind)"
        );
        let rm = r#"{"call":"package.change","verb":"remove","names":["vim"]}"#;
        let a = handle(rm, Some((1000, APP)), &only_install, &f);
        assert_eq!(
            a.code,
            Some(Code::PermissionDenied),
            "each verb has its own grant"
        );
        assert!(a.message.unwrap().contains("Privacy & Security"));
        assert_eq!(
            handle(inst, None, &all, &f).code,
            Some(Code::PermissionDenied)
        );
        assert_eq!(
            handle(inst, Some((1000, "/usr/bin/other")), &all, &f).code,
            Some(Code::PermissionDenied)
        );
        let bad = r#"{"call":"package.change","verb":"install","names":["-Syu"]}"#;
        assert_eq!(
            handle(bad, Some((1000, APP)), &all, &f).code,
            Some(Code::InvalidArgument)
        );
        assert_eq!(f.ran.borrow().len(), 0, "nothing else ran");
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        assert!(handle(file, Some((1000, APP)), &all, &f).ok);
        assert_eq!(
            f.ran.borrow()[0].last().unwrap(),
            &format!("{STAGING}/n/x-1-1-x86_64.pkg.tar.zst")
        );
        assert_eq!(
            f.discarded.borrow().len(),
            1,
            "the copy is forgotten after the install"
        );
        assert_eq!(
            handle(file, Some((1001, APP)), &all, &f).code,
            Some(Code::PermissionDenied),
            "not the caller's file"
        );
        assert!(
            handle(
                r#"{"call":"system.channel","channel":"beta"}"#,
                Some((1000, APP)),
                &all,
                &f
            )
            .ok
        );
        assert_eq!(f.wrote.borrow()[0], (CHANNEL_FILE.into(), "beta\n".into()));
    }

    #[test]
    fn system_update_needs_a_signed_newer_release() {
        let seed = [9u8; 32];
        let f = Fake {
            running: Some(manifest("2027/01/10")),
            ..yes(seed)
        };
        let all = |_: &str, _: &str| true;
        let call = |m: &str, sig: &[u8]| {
            let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
            PackageCall::SystemUpdate {
                manifest: m.into(),
                signature: hex,
            }
            .encode()
        };
        let newer = manifest("2027/03/01");
        let good = call(&newer, &wol_core::package::sign(&seed, newer.as_bytes()));
        assert!(handle(&good, Some((1000, APP)), &all, &f).ok);
        assert_eq!(
            f.ran.borrow()[0],
            s(&[WOL, "update", "apply", "--release", "2027/03/01", "--yes"])
        );
        let forged = call(&newer, &wol_core::package::sign(&[1; 32], newer.as_bytes()));
        assert_eq!(
            handle(&forged, Some((1000, APP)), &all, &f).code,
            Some(Code::PermissionDenied)
        );
        // CORE-7: a signature that is not hex (multi-byte text) is refused, never a panic.
        let odd = PackageCall::SystemUpdate {
            manifest: newer.clone(),
            signature: "aé".into(),
        }
        .encode();
        assert!(!handle(&odd, Some((1000, APP)), &all, &f).ok);
        let older = manifest("2026/12/01");
        let down = call(&older, &wol_core::package::sign(&seed, older.as_bytes()));
        assert_eq!(
            handle(&down, Some((1000, APP)), &all, &f).code,
            Some(Code::InvalidArgument),
            "no downgrades"
        );
        assert_eq!(f.ran.borrow().len(), 1);
    }
}
