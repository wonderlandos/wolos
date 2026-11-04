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
//! holds) is an administrator sheet the shell runs through wol-admind after the password, and so
//! are a locally built package (wold's own staged copy, ADR 0034), the update channel and Undo
//! Update (audit C7-1: a standard account's own shell answers its sheets, so only wol-admind's
//! administrator check counts). wold itself runs one Allow-sheet change: a signed, newer system
//! release. No session, no answer, Deny or a timeout: nothing runs. wold never remembers a yes.
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
pub const CHANNEL_FILE: &str = wol_core::admin::CHANNEL_FILE;
/// Longest the one Allow-sheet change (a signed system update) may run before wold stops it
/// (audit S4-11); the same bound wol-admind gives its long steps.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(1800);
/// Longest shell message wold passes on to the app (S4-13).
const MAX_SHELL_MESSAGE: usize = 200;

/// What wold does on the machine (fake in tests).
pub trait System {
    /// Run `argv` as root; exit code, [`crate::reaper::TIMED_OUT`] when it ran past
    /// [`RUN_TIMEOUT`] and was stopped.
    fn run(&self, argv: &[String]) -> i32;
    /// Copy `path` into a fresh directory under [`STAGING`] if it is a regular file owned by
    /// `uid` that `uid` can reach (every directory above it searchable,
    /// [`crate::admind::caller_may_reach`]); the copy's path. None when the file is not the
    /// caller's or cannot be copied.
    fn stage(&self, path: &str, uid: u32) -> Option<String>;
    /// Forget a copy [`System::stage`] made (after the install, or when the person said no).
    fn discard(&self, staged: &str);
    /// What the sheet says about a staged copy (ADR 0034): its `.PKGINFO` name and version and
    /// its SHA-256 ([`inspect_staged`]); Err with the reason when it cannot be read.
    fn inspect(&self, staged: &str) -> Result<PkgFacts, String>;
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
    /// Is there room under [`STAGING`] for the copy of `path` and a reserve left over (P6-5)?
    /// Err with what to tell the person. Checked before anything is copied.
    fn room(&self, _path: &str) -> Result<(), String> {
        Ok(())
    }
}

/// wold copies a caller's file only when the caller owns it and could walk down to it (QA-100
/// L1): every directory above it searchable by the caller, as wol-admind's copy requires
/// ([`crate::admind::caller_may_reach`]). Owning a file inside another account's private folder
/// (a hard link, a file left there) is not reaching it.
pub fn may_stage(file_uid: u32, dirs: &[(u32, u32)], uid: u32) -> bool {
    file_uid == uid && crate::admind::caller_may_reach(dirs, Some(uid))
}

/// Free space [`STAGING`]'s filesystem keeps after a copy (audit P6-5): a tenth of it, at least
/// 1 GiB. Pacman, the journal and the other accounts' files need it more than one more package.
pub fn staging_reserve(total: u64) -> u64 {
    (total / 10).max(1 << 30)
}

/// Would a copy of `size` bytes leave `free` of `total` bytes at or above [`staging_reserve`]?
pub fn enough_room(free: u64, total: u64, size: u64) -> bool {
    free.checked_sub(size)
        .is_some_and(|left| left >= staging_reserve(total))
}

/// The refusal shown when the copy of `size` would leave too little free space.
fn no_room_message() -> String {
    "There is not enough free space on this computer to install that file. Free some space and try again."
        .into()
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
        crate::reaper::run(
            crate::privileged::privileged_command(prog).args(args),
            RUN_TIMEOUT,
        )
    }

    fn room(&self, path: &str) -> Result<(), String> {
        // The size of the file as it is now; the copy itself stops at MAX_STAGED either way.
        let size = shared::sys::open_named(path)
            .and_then(|f| f.metadata())
            .map_or(MAX_STAGED, |m| m.len().min(MAX_STAGED));
        match crate::os::space_at(std::path::Path::new(STAGING)) {
            Some((free, total)) if enough_room(free, total, size) => Ok(()),
            _ => Err(no_room_message()),
        }
    }

    fn stage(&self, path: &str, uid: u32) -> Option<String> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        // O_NOFOLLOW + fstat on the opened file: what is checked is what is copied. O_NOCTTY
        // (S4-2): a pty the caller names never becomes wold's terminal, whose hang-up would end
        // wold and every session under it.
        let mut f = shared::sys::open_named(path).ok()?;
        let md = f.metadata().ok()?;
        let dirs = crate::admind::dirs_above(&f).ok()?;
        if !md.is_file() || !may_stage(md.uid(), &dirs, uid) {
            return None;
        }
        // api-security 5: as for wol-admind's copy, the caller could walk down to the file
        // (its own file kept inside another account's private directory is not the caller's).
        let dirs = crate::admind::dirs_above(&f).ok()?;
        if !crate::admind::caller_may_reach(&dirs, Some(uid)) {
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

    fn inspect(&self, staged: &str) -> Result<PkgFacts, String> {
        inspect_staged(staged)
    }

    fn discard(&self, staged: &str) {
        let dir = std::path::Path::new(staged).parent();
        if let Some(d) = dir.filter(|d| d.parent() == Some(std::path::Path::new(STAGING))) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    fn keys(&self) -> Vec<[u8; 32]> {
        wol_core::package::read_keys(std::path::Path::new(shared::package::KEYS_DIR))
    }

    fn running_release(&self) -> Option<String> {
        std::fs::read_to_string(wol_core::update::check::RELEASE_FILE).ok()
    }

    fn ask(&self, uid: u32, approval: &Approval) -> Option<Sheet> {
        ask_shower(
            self.bus,
            self.asks,
            uid,
            approval,
            ask_wait(&approval.request, crate::portal::patience_of(uid)),
        )
    }
}

/// How long wold waits for the answer to `request`. An Allow sheet answers when it is clicked; an
/// administrator sheet answers after wol-admind ran it, so the wait also covers its steps at
/// wol-admind's own deadlines (one request per package name). Otherwise wold would give up, and
/// drop a staged copy, while pacman still works on it.
pub fn ask_wait(request: &ApprovalRequest, patience: shared::patience::Patience) -> Duration {
    use wol_core::admin::{LONG_STEP_SECS, REPLY_MARGIN_SECS, STEP_SECS};
    let run: u64 = match request {
        ApprovalRequest::Change { .. } => 0,
        ApprovalRequest::Packages { names, .. } => {
            u64::from(LONG_STEP_SECS) * names.len().max(1) as u64
        }
        ApprovalRequest::PackageFile { .. } | ApprovalRequest::SystemRollback => {
            LONG_STEP_SECS.into()
        }
        // A Doctor fix may install packages: a long step each.
        ApprovalRequest::DoctorFix { steps, .. } => {
            u64::from(LONG_STEP_SECS) * steps.len().max(1) as u64
        }
        _ => STEP_SECS.into(),
    };
    let margin = if run > 0 { REPLY_MARGIN_SECS.into() } else { 0 };
    // The sheet's own time, stretched by the account's Timeouts setting, then the ten seconds
    // for its answer to arrive.
    let click = patience
        .secs(shared::shell::APPROVAL_TIMEOUT_SECS)
        .saturating_add(shared::shell::APPROVAL_WAIT_SECS - shared::shell::APPROVAL_TIMEOUT_SECS);
    Duration::from_secs(u64::from(click) + run + margin)
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
/// "update" with names installs what is not installed yet, so it is an install). So are a local
/// file (`staged`: wold's own copy, which is what wol-admind installs; ADR 0034), the channel and
/// Undo Update (audit C7-1). Only a signed newer release is an Allow sheet after which wold runs
/// the change itself.
pub fn request(call: &PackageCall, staged: Option<(&str, &PkgFacts)>) -> ApprovalRequest {
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
        PackageCall::InstallFile { .. } => {
            let none = PkgFacts::default();
            let (path, f) = staged.unwrap_or(("", &none));
            ApprovalRequest::PackageFile {
                staged: path.to_owned(),
                name: f.name.clone(),
                version: f.version.clone(),
                sha256: f.sha256.clone(),
            }
        }
        PackageCall::SystemChannel { channel } => ApprovalRequest::SystemChannel {
            channel: channel.clone(),
        },
        PackageCall::SystemRollback {} => ApprovalRequest::SystemRollback,
        PackageCall::SystemUpdate { .. } => ApprovalRequest::Change {
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
fn approved(
    sys: &dyn System,
    uid: u32,
    app: &str,
    call: &PackageCall,
    staged: Option<(&str, &PkgFacts)>,
) -> Result<Approved, Answer> {
    let request = request(call, staged);
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
        Some(Sheet::Failed { message }) if admin => {
            Err(Answer::err(Code::Internal, failed_message(call, &message)))
        }
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

/// What the app hears when the person allowed an administrator change and it failed (S4-13).
/// One name is one pacman transaction: nothing changed. Several names are one wol-admind request
/// each in the shell, so some may be done already: no such promise, and the shell's own sentence
/// (one line, bounded) says what happened.
fn failed_message(call: &PackageCall, shell: &str) -> String {
    let names = match call {
        PackageCall::Change { names, .. } => names.len(),
        PackageCall::UpdateHolding { hold } => hold.len().min(1),
        _ => 1,
    };
    let said: String = shell
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_SHELL_MESSAGE)
        .collect();
    let said = said.trim();
    // Update Now (everything, holds or not): wol-admind's own sentence, passed on by the shell,
    // is truer than any promise made here (the restore point it kept, or why none could be).
    let update_now = matches!(call, PackageCall::Change { verb: Verb::Update, names } if names.is_empty())
        || matches!(call, PackageCall::UpdateHolding { .. });
    if update_now && !said.is_empty() {
        return said.to_owned();
    }
    match (names > 1, said.is_empty()) {
        (false, _) => "The change did not finish. Nothing was changed; try again.".into(),
        (true, true) => {
            "The change did not finish; some of it may already be done. Check the apps and try again.".into()
        }
        (true, false) => format!(
            "The change did not finish ({said}); some of it may already be done. Check the apps and try again."
        ),
    }
}

/// What an administrator sheet shows about a staged package (ADR 0034): the copy's `.PKGINFO`
/// `pkgname` and `pkgver`, and the SHA-256 of the copy (64 lowercase hex).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PkgFacts {
    pub name: String,
    pub version: String,
    pub sha256: String,
}

/// Decompressed bytes of a package wold reads looking for `.PKGINFO` (makepkg writes it first).
pub const PKGINFO_HEAD: usize = 1 << 20;
/// Largest `.PKGINFO` wold reads.
pub const MAX_PKGINFO: usize = 64 * 1024;
/// Longest `pkgname` / `pkgver` a sheet shows; a package that claims more is refused.
pub const MAX_PKG_NAME: usize = 128;
pub const MAX_PKG_VERSION: usize = 64;
/// Longest wold waits for the system `zstd` to unpack a whole staged package and for the walk
/// over all its tar headers ([`scan_package`]).
const ZSTD_TIMEOUT: Duration = Duration::from_secs(120);
/// Most decompressed bytes wold walks through in one staged package (4 x [`MAX_STAGED`]); a
/// package that unpacks to more is refused.
pub const MAX_UNPACKED: u64 = 4 * MAX_STAGED;
/// Largest pax extended header or long-name header wold reads (all other bodies are skipped).
const MAX_TAR_META: u64 = 1 << 20;

/// The facts of wold's staged copy at `staged`, read in one pass of the system `zstd` (root-side
/// launch with a cleared environment, [`ZSTD_TIMEOUT`]) through [`scan_package`]: the first
/// [`PKGINFO_HEAD`] decompressed bytes must hold a well-formed `.PKGINFO` ([`pkginfo_in`],
/// [`parse_pkginfo`]), and every later tar header is checked so that no second `.PKGINFO`
/// stands behind the one the sheet shows. The SHA-256 is of the whole copy, which only root can
/// write, so it is what wol-admind installs. Err: why not.
pub fn inspect_staged(staged: &str) -> Result<PkgFacts, String> {
    let info = zstd_read(staged, ZSTD_TIMEOUT, |out| scan_package(out, MAX_UNPACKED))??;
    let text = std::str::from_utf8(&info).map_err(|_| "its .PKGINFO is not text".to_owned())?;
    let (name, version) = parse_pkginfo(text)?;
    let sha256 = wol_core::package::sha256_file(std::path::Path::new(staged))
        .map_err(|e| format!("it could not be read again: {e}"))?;
    Ok(PkgFacts {
        name,
        version,
        sha256,
    })
}

/// `read` on the output of `zstd -d -c` on `path`, within `timeout`. The decompressor is stopped
/// as soon as `read` returns (or the time is up), so only what `read` needed is decompressed.
fn zstd_read<T: Send + 'static>(
    path: &str,
    timeout: Duration,
    read: impl FnOnce(&mut dyn std::io::Read) -> T + Send + 'static,
) -> Result<T, String> {
    let mut child = crate::privileged::privileged_command("zstd")
        .args(["-q", "-d", "-c", "--", path])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "zstd is missing".to_owned())?;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("zstd did not start".into());
    };
    let (tx, rx) = std::sync::mpsc::channel();
    // Ends when `read` is done, at zstd's end, or when zstd is killed below (its pipe closes).
    let reader = std::thread::Builder::new()
        .name("wold-zstd-read".into())
        .spawn(move || {
            let _ = tx.send(read(&mut out));
        });
    let got = match reader {
        Ok(_) => rx
            .recv_timeout(timeout)
            .map_err(|_| "decompressing it took too long".to_owned()),
        Err(_) => Err("wold could not start reading it".to_owned()),
    };
    // Done reading (or out of time): stop it unless it has already exited (wold's reaper may
    // have collected it, then try_wait errs).
    if let Ok(None) = child.try_wait() {
        let _ = child.kill();
        let _ = child.wait();
    }
    got
}

/// The `.PKGINFO` of a decompressed package stream `r`: the first one must lie in its first
/// [`PKGINFO_HEAD`] bytes ([`pkginfo_in`]), then every tar header up to the end-of-archive block
/// is walked ([`tar_walk`], bodies skipped unread, at most `cap` bytes in all). Err when there is
/// no such `.PKGINFO`, when any other member could also be one, or when the archive is damaged,
/// cut off, larger than `cap` or uses a member kind the walk does not know.
pub fn scan_package(r: &mut dyn std::io::Read, cap: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut head = Vec::new();
    r.take(PKGINFO_HEAD as u64)
        .read_to_end(&mut head)
        .map_err(|_| "it could not be decompressed".to_owned())?;
    let info = pkginfo_in(&head)?.to_vec();
    let found = tar_walk(&mut (&head[..]).chain(r), cap)?;
    if found != 1 {
        return Err("it has more than one package information file (.PKGINFO)".into());
    }
    Ok(info)
}

/// Does a member name resolve to `.PKGINFO` (leading `./` or `/`, trailing `/` ignored)?
fn names_pkginfo(name: &[u8]) -> bool {
    let mut n = name;
    loop {
        if let Some(rest) = n.strip_prefix(b"./") {
            n = rest;
        } else if let Some(rest) = n.strip_prefix(b"/") {
            n = rest;
        } else {
            break;
        }
    }
    while let Some(rest) = n.strip_suffix(b"/") {
        n = rest;
    }
    n == b".PKGINFO"
}

/// A checksum-checked tar header block: its kind, its own size field and its name (ustar
/// prefix joined).
fn tar_header(h: &[u8]) -> Result<(u8, u64, Vec<u8>), String> {
    let bad = || "it is not a package (a damaged tar header)".to_owned();
    let sum: u64 = h
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(*b)
            }
        })
        .sum();
    if tar_octal(&h[148..156]) != Some(sum) {
        return Err(bad());
    }
    let own = tar_octal(&h[124..136]).ok_or_else(bad)?;
    let (n, prefix) = (tar_text(&h[..100]), tar_text(&h[345..500]));
    let name = if &h[257..262] == b"ustar" && !prefix.is_empty() {
        [prefix, &b"/"[..], n].concat()
    } else {
        n.to_vec()
    };
    Ok((h[156], own, name))
}

/// Walks every tar header of the stream `r` up to its end-of-archive block, skipping member
/// bodies without keeping them and reading at most `cap` bytes; returns how many members could
/// be `.PKGINFO` by any of their names (header, pax `path`, long-name header; [`names_pkginfo`]).
/// Err when a header is damaged, the stream ends before the end-of-archive block, it is larger
/// than `cap`, or a member is of a kind the walk does not know (or a link, device or folder
/// carries data, which the package manager would read differently).
fn tar_walk(r: &mut dyn std::io::Read, cap: u64) -> Result<usize, String> {
    use std::io::Read;
    // One tar block: `BLOCK` sizes the header buffer, `B` (the same, widened) counts bytes.
    const BLOCK: usize = 512;
    const B: u64 = BLOCK as u64;
    let bad = || "it is not a package (a damaged tar header)".to_owned();
    let cut = || "it is cut off".to_owned();
    let big = || format!("it unpacks to more than {} GiB", cap >> 30);
    let (mut total, mut found) = (0u64, 0usize);
    let (mut next_names, mut next_size) = (Vec::<Vec<u8>>::new(), None::<u64>);
    loop {
        let mut h = [0u8; BLOCK];
        r.read_exact(&mut h).map_err(|_| cut())?;
        total += B;
        if total > cap {
            return Err(big());
        }
        if h.iter().all(|b| *b == 0) {
            return Ok(found);
        }
        let (kind, own, name) = tar_header(&h)?;
        let size = if matches!(kind, b'x' | b'g' | b'L' | b'K') {
            own
        } else {
            next_size.take().unwrap_or(own)
        };
        let padded = size.div_ceil(B).checked_mul(B).ok_or_else(big)?;
        total = total
            .checked_add(padded)
            .filter(|t| *t <= cap)
            .ok_or_else(big)?;
        let mut body = Vec::new();
        if matches!(kind, b'x' | b'L') {
            if size > MAX_TAR_META {
                return Err(bad());
            }
            r.take(size).read_to_end(&mut body).map_err(|_| cut())?;
            if body.len() as u64 != size {
                return Err(cut());
            }
        }
        match kind {
            b'x' => {
                let (p, s) = pax_records(&body)?;
                next_names.extend(p);
                next_size = s.or(next_size);
            }
            b'L' => next_names.push(tar_text(&body).to_vec()),
            b'g' | b'K' => {}
            0 | b'0'..=b'7' => {
                if !matches!(kind, 0 | b'0' | b'7') && size != 0 {
                    return Err(bad());
                }
                next_names.push(name);
                if next_names.drain(..).any(|n| names_pkginfo(&n)) {
                    found += 1;
                }
            }
            _ => return Err("it holds a kind of entry WolOS does not read".into()),
        }
        let skip = padded - body.len() as u64;
        let skipped = std::io::copy(&mut r.take(skip), &mut std::io::sink()).map_err(|_| cut())?;
        if skipped != skip {
            return Err(cut());
        }
    }
}

/// Value of an octal tar number field (NUL/space padded).
fn tar_octal(b: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(b).ok()?;
    let s = s.trim_matches(|c| c == '\0' || c == ' ');
    if s.is_empty() || !s.bytes().all(|c| (b'0'..=b'7').contains(&c)) {
        return None;
    }
    u64::from_str_radix(s, 8).ok()
}

/// A NUL-terminated tar text field.
fn tar_text(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())]
}

/// The `path` and `size` records of a pax extended header (`<len> <key>=<value>\n` each).
fn pax_records(mut d: &[u8]) -> Result<(Option<Vec<u8>>, Option<u64>), String> {
    let bad = || "it is not a package (a damaged tar header)".to_owned();
    let (mut path, mut size) = (None, None);
    while !d.is_empty() && d[0] != 0 {
        let sp = d.iter().position(|c| *c == b' ').ok_or_else(bad)?;
        let len: usize = std::str::from_utf8(&d[..sp])
            .ok()
            .and_then(|n| n.parse().ok())
            .ok_or_else(bad)?;
        let rec = d
            .get(sp + 1..len)
            .filter(|r| r.ends_with(b"\n"))
            .ok_or_else(bad)?;
        let rec = &rec[..rec.len() - 1];
        let eq = rec.iter().position(|c| *c == b'=').ok_or_else(bad)?;
        match &rec[..eq] {
            b"path" => path = Some(rec[eq + 1..].to_vec()),
            b"size" => {
                let v = std::str::from_utf8(&rec[eq + 1..]).ok();
                size = Some(v.and_then(|v| v.parse().ok()).ok_or_else(bad)?);
            }
            _ => {}
        }
        d = &d[len..];
    }
    Ok((path, size))
}

/// The `.PKGINFO` member in the first bytes of a package's tar stream (`head`), as the package
/// manager names members: a pax `path` or long-name header overrides the header's own name, and a
/// ustar prefix is joined. Every header's checksum is checked. Err when it is missing from
/// `head`, larger than [`MAX_PKGINFO`] or not a plain file.
pub fn pkginfo_in(head: &[u8]) -> Result<&[u8], String> {
    const B: usize = 512;
    let missing = || "it has no package information (.PKGINFO) at its start".to_owned();
    let (mut pos, mut next_name, mut next_size) = (0usize, None::<Vec<u8>>, None::<u64>);
    loop {
        let Some(h) = head.get(pos..pos + B) else {
            return Err(missing());
        };
        if h.iter().all(|b| *b == 0) {
            return Err(missing());
        }
        let (kind, own, own_name) = tar_header(h)?;
        let size = if matches!(kind, b'x' | b'g' | b'L' | b'K') {
            own
        } else {
            next_size.take().unwrap_or(own)
        };
        let start = pos + B;
        let data_end = usize::try_from(size)
            .ok()
            .and_then(|n| start.checked_add(n));
        let data = data_end.and_then(|e| head.get(start..e));
        match kind {
            b'x' => {
                let (p, s) = pax_records(data.ok_or_else(missing)?)?;
                next_name = p.or(next_name);
                next_size = s.or(next_size);
            }
            b'L' => next_name = Some(tar_text(data.ok_or_else(missing)?).to_vec()),
            b'g' | b'K' => {}
            _ => {
                let name = next_name.take().unwrap_or(own_name);
                if name == b".PKGINFO" {
                    if !matches!(kind, b'0' | 0) {
                        return Err("its .PKGINFO is not a plain file".into());
                    }
                    if size > MAX_PKGINFO as u64 {
                        return Err("its .PKGINFO is too large".into());
                    }
                    return data.ok_or_else(missing);
                }
            }
        }
        let blocks = usize::try_from(size.div_ceil(B as u64)).map_err(|_| missing())?;
        pos = blocks
            .checked_mul(B)
            .and_then(|n| start.checked_add(n))
            .ok_or_else(missing)?;
    }
}

/// `pkgname` and `pkgver` of a `.PKGINFO` (`key = value` lines, `#` comments). Each must be
/// there exactly once: a name of letters, digits and `@._+-` (not starting with `-` or `.`) of
/// at most [`MAX_PKG_NAME`] bytes, a version (`[epoch:]version-release`) of letters, digits and
/// `._+~:-` of at most [`MAX_PKG_VERSION`] bytes, so the sheet line stays short and plain.
pub fn parse_pkginfo(text: &str) -> Result<(String, String), String> {
    let (mut name, mut version) = (None, None);
    for l in text.lines().map(str::trim) {
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let slot = match k.trim() {
            "pkgname" => &mut name,
            "pkgver" => &mut version,
            _ => continue,
        };
        if slot.replace(v.trim().to_owned()).is_some() {
            return Err(format!("its .PKGINFO names {} twice", k.trim()));
        }
    }
    let name = name.ok_or("its .PKGINFO has no package name")?;
    let version = version.ok_or("its .PKGINFO has no version")?;
    let name_ok = !name.is_empty()
        && name.len() <= MAX_PKG_NAME
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"@._+-".contains(&c));
    if !name_ok {
        return Err("its .PKGINFO has an invalid package name".into());
    }
    let version_ok = !version.is_empty()
        && version.len() <= MAX_PKG_VERSION
        && version
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+~:-".contains(&c));
    if !version_ok {
        return Err("its .PKGINFO has an invalid version".into());
    }
    Ok((name, version))
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

/// The refusal names the pane and the row the person turns on: Privacy & Security lists the app
/// under the section of its permission (the shell's `privacy::KINDS` titles), with its switch
/// (wold noted the refusal, `portal::Remembering`). There is no "Software" section.
fn denied(grant: &str, app: &str) -> Answer {
    let (what, section) = match grant {
        "packages-install" => ("install system apps", "Install Apps"),
        "packages-remove" => ("remove system apps", "Remove Apps"),
        "packages-update" => ("update the system", "System Updates"),
        _ => ("install apps built on this computer", "Unreviewed Apps"),
    };
    let name = app.rsplit('/').next().unwrap_or(app);
    Answer::err(
        Code::PermissionDenied,
        format!(
            "{name} is not allowed to {what}. Turn on {name} under {section} in System Settings > Privacy & Security."
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
    if let PackageCall::InstallFile { path } = &call
        && let Err(why) = sys.room(path)
    {
        return Answer::err(Code::Unavailable, why);
    }
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
    // ADR 0034: the sheet names what the copy contains (its .PKGINFO name and version) and its
    // SHA-256, read from wold's own copy; a copy that cannot be read is refused before any sheet.
    let facts = match &staged {
        Some(copy) => match sys.inspect(copy) {
            Ok(f) => Some(f),
            Err(why) => {
                sys.discard(copy);
                return Answer::err(
                    Code::InvalidArgument,
                    format!("That package file can't be read ({why}), so it was not installed."),
                );
            }
        },
        None => None,
    };
    let shown = staged.as_deref().zip(facts.as_ref());
    let yes = approved(sys, uid, app, &call, shown);
    // ADR 0034: wol-admind installed the copy before the shell answered (or it was refused); wold
    // never runs it itself.
    if let Some(copy) = &staged {
        sys.discard(copy);
    }
    match yes {
        Ok(Approved::RunHere) => {}
        Ok(Approved::Done) => return Answer::ok(),
        Err(no) => return no,
    }
    let ran = |argv: Vec<String>, what: &str| match sys.run(&argv) {
        0 => Answer::ok(),
        // S4-11: stopped at the deadline; it may have done part of its work.
        crate::reaper::TIMED_OUT => Answer::err(
            Code::Internal,
            format!("{what} took too long and was stopped. Try again."),
        ),
        _ => Answer::err(
            Code::Internal,
            format!("{what} did not finish. Nothing was changed; try again."),
        ),
    };
    match call {
        // Administrator sheets: the shell ran them through wol-admind (Done above). Never here,
        // whatever the answer (audit C7-1: a local file, the channel and undo too).
        PackageCall::Change { .. }
        | PackageCall::UpdateHolding { .. }
        | PackageCall::InstallFile { .. }
        | PackageCall::SystemChannel { .. }
        | PackageCall::SystemRollback {} => Answer::err(
            Code::PermissionDenied,
            "This change needs the administrator password, so nothing was changed.",
        ),
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
            // The running slot's release file has no image hash (wolos-base writes it): Running.
            let running = sys
                .running_release()
                .and_then(|t| wol_core::update::check::Running::parse(&t));
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
    }
}

#[cfg(test)]
mod tests;
