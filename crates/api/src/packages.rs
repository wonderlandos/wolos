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
        if !md.is_file() || md.uid() != uid {
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
            ask_wait(&approval.request),
        )
    }
}

/// How long wold waits for the answer to `request`. An Allow sheet answers when it is clicked; an
/// administrator sheet answers after wol-admind ran it, so the wait also covers its steps at
/// wol-admind's own deadlines (one request per package name). Otherwise wold would give up, and
/// drop a staged copy, while pacman still works on it.
pub fn ask_wait(request: &ApprovalRequest) -> Duration {
    use wol_core::admin::{LONG_STEP_SECS, REPLY_MARGIN_SECS, STEP_SECS};
    let run: u64 = match request {
        ApprovalRequest::Change { .. } => 0,
        ApprovalRequest::Packages { names, .. } => {
            u64::from(LONG_STEP_SECS) * names.len().max(1) as u64
        }
        ApprovalRequest::PackageFile { .. } | ApprovalRequest::SystemRollback => {
            LONG_STEP_SECS.into()
        }
        _ => STEP_SECS.into(),
    };
    let margin = if run > 0 { REPLY_MARGIN_SECS.into() } else { 0 };
    Duration::from_secs(u64::from(shared::shell::APPROVAL_WAIT_SECS) + run + margin)
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
            request(&call, None),
            ApprovalRequest::Packages {
                verb: "update-holding".into(),
                names: vec!["firefox".into(), "gimp".into()]
            }
        );
        let none = PackageCall::UpdateHolding { hold: vec![] };
        assert_eq!(
            request(&none, None),
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
        seed: [u8; 32],
        running: Option<String>,
        /// The sheet's answer; None = no session answered.
        sheet: Option<Sheet>,
        asked: RefCell<Vec<Approval>>,
        discarded: RefCell<Vec<String>>,
        staging: Staging,
        /// Exit code of every run (0 = it worked).
        code: i32,
        /// Why the staged copy cannot be read (None: it reads as [`facts`]).
        unreadable: Option<String>,
        /// Set: the disk has no room for the copy (the refusal shown).
        no_room: Option<String>,
        /// How many times the fake was asked to copy a file.
        staged_calls: std::cell::Cell<u32>,
    }

    /// What the fake reads from every staged copy.
    fn facts() -> PkgFacts {
        PkgFacts {
            name: "x".into(),
            version: "1-1".into(),
            sha256: "ab".repeat(32),
        }
    }

    /// The sheet for the fake's staged copy of `/home/a/x-1-1-x86_64.pkg.tar.zst`.
    fn file_sheet() -> ApprovalRequest {
        let f = facts();
        ApprovalRequest::PackageFile {
            staged: format!("{STAGING}/n/x-1-1-x86_64.pkg.tar.zst"),
            name: f.name,
            version: f.version,
            sha256: f.sha256,
        }
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
            self.code
        }
        fn room(&self, _: &str) -> Result<(), String> {
            self.no_room.clone().map_or(Ok(()), Err)
        }
        fn stage(&self, p: &str, uid: u32) -> Option<String> {
            self.staged_calls.set(self.staged_calls.get() + 1);
            (uid == 1000).then(|| format!("{STAGING}/n/{}", p.rsplit('/').next().unwrap()))
        }
        fn discard(&self, staged: &str) {
            self.discarded.borrow_mut().push(staged.into());
        }
        fn inspect(&self, _: &str) -> Result<PkgFacts, String> {
            self.unreadable.clone().map_or_else(|| Ok(facts()), Err)
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

    /// Audit S4-11: the one change wold runs itself (a signed newer release) is stopped at the
    /// deadline when it hangs (TIMED_OUT from reaper::run), and the app hears so.
    #[test]
    fn a_system_update_past_its_deadline_says_so() {
        let seed = [3u8; 32];
        let f = Fake {
            code: crate::reaper::TIMED_OUT,
            running: Some(manifest("2027/01/10")),
            ..yes(seed)
        };
        let all = |_: &str, _: &str| true;
        let newer = manifest("2027/03/01");
        let sig: String = wol_core::package::sign(&seed, newer.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let call = PackageCall::SystemUpdate {
            manifest: newer,
            signature: sig,
        }
        .encode();
        let a = handle(&call, Some((1000, APP)), &all, &f);
        assert!(!a.ok);
        assert!(
            a.message.as_deref().unwrap().contains("took too long"),
            "{a:?}"
        );
        assert_eq!(f.ran.borrow().len(), 1);
    }

    /// An administrator sheet answers only after wol-admind ran it, so wold waits for its
    /// deadlines too; an Allow sheet only for the click.
    #[test]
    fn administrator_sheets_are_waited_for_as_long_as_they_may_run() {
        let click = Duration::from_secs(shared::shell::APPROVAL_WAIT_SECS.into());
        let change = ApprovalRequest::Change {
            capability: "packages-update".into(),
            what: "x".into(),
        };
        assert_eq!(ask_wait(&change), click);
        let file = file_sheet();
        let long = Duration::from_secs(wol_core::admin::LONG_STEP_SECS.into());
        assert!(ask_wait(&file) >= click + long);
        assert!(ask_wait(&ApprovalRequest::SystemRollback) >= click + long);
        let three = ApprovalRequest::Packages {
            verb: "install".into(),
            names: vec!["a".into(), "b".into(), "c".into()],
        };
        assert!(ask_wait(&three) >= click + long * 3);
        let chan = ApprovalRequest::SystemChannel {
            channel: "beta".into(),
        };
        assert!(ask_wait(&chan) > click);
    }

    /// Audit S4-13: a failed administrator sheet of several names makes no "nothing changed"
    /// promise (one wol-admind request per name in the shell) and passes on the shell's reason,
    /// bounded and on one line; one name keeps the promise (one pacman transaction).
    #[test]
    fn a_failed_multi_name_change_does_not_claim_nothing_changed() {
        let all = |_: &str, _: &str| true;
        let failed = |m: &str| Fake {
            sheet: Some(Sheet::Failed { message: m.into() }),
            ..Default::default()
        };
        let many = r#"{"call":"package.change","verb":"install","names":["a","b","c"]}"#;
        let f = failed("b could not be installed\nrm -rf");
        let a = handle(many, Some((1000, APP)), &all, &f);
        let m = a.message.unwrap();
        assert!(!m.contains("Nothing was changed"), "{m}");
        assert!(m.contains("some of it may already be done"), "{m}");
        assert!(m.contains("b could not be installed"), "{m}");
        assert!(!m.contains('\n'), "{m}");
        let long = "x".repeat(5000);
        let a = handle(many, Some((1000, APP)), &all, &failed(&long));
        assert!(a.message.unwrap().len() < 400);
        let one = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
        let a = handle(one, Some((1000, APP)), &all, &failed("x"));
        assert!(a.message.unwrap().contains("Nothing was changed"));
    }

    /// Audit S4-2: what an app names for install-file is opened without becoming wold's
    /// terminal and checked on the descriptor: a device or a FIFO is refused, and at once.
    #[test]
    fn staging_refuses_devices_and_fifos_without_waiting() {
        let bus = Bus::default();
        let asks = Asks::default();
        let host = Host {
            bus: &bus,
            asks: &asks,
        };
        let me = shared::sys::euid();
        let t = std::time::Instant::now();
        assert!(host.stage("/dev/null", me).is_none());
        assert!(host.stage("/dev/tty", me).is_none());
        let fifo = std::env::temp_dir().join(format!("wol-stage-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&fifo);
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            assert!(host.stage(fifo.to_str().unwrap(), me).is_none());
            let _ = std::fs::remove_file(&fifo);
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
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

    /// Audit P6-5: a copy that would leave the disk under its reserve is refused with a clear
    /// message before any file is copied or any sheet shown.
    #[test]
    fn a_staged_install_is_refused_below_the_free_space_reserve() {
        let call = r#"{"call":"package.install-file","path":"/home/a/x.pkg.tar.zst"}"#;
        let all = |_: &str, _: &str| true;
        let full = Fake {
            no_room: Some(no_room_message()),
            ..yes([0; 32])
        };
        let a = handle(call, Some((1000, APP)), &all, &full);
        assert!(!a.ok);
        assert_eq!(a.code, Some(Code::Unavailable), "{a:?}");
        assert!(
            a.message.as_deref().unwrap().contains("free space"),
            "{a:?}"
        );
        assert_eq!(full.staged_calls.get(), 0, "nothing was copied");
        assert!(full.asked.borrow().is_empty() && full.ran.borrow().is_empty());
        assert!(
            full.staging.enter(1000).is_some() && full.staging.enter(1001).is_some(),
            "the place was given back"
        );
        let roomy = yes([0; 32]);
        assert!(handle(call, Some((1000, APP)), &all, &roomy).ok);
        assert_eq!(roomy.staged_calls.get(), 1);
    }

    #[test]
    fn the_reserve_is_a_tenth_of_the_disk_and_at_least_a_gigabyte() {
        const G: u64 = 1 << 30;
        assert_eq!(staging_reserve(5 * G), G, "small disk: the floor");
        assert_eq!(staging_reserve(500 * G), 50 * G, "big disk: a tenth");
        assert!(
            enough_room(10 * G, 100 * G, 0),
            "exactly the reserve is kept"
        );
        assert!(enough_room(15 * G, 100 * G, 5 * G));
        assert!(!enough_room(15 * G, 100 * G, 6 * G), "would dip into it");
        assert!(
            !enough_room(2 * G, 100 * G, 4 * G),
            "bigger than what is free"
        );
        assert!(!enough_room(0, 0, 0), "an unreadable disk keeps nothing");
    }

    /// The real host reads the real disk: a temp-dir filesystem answers, with its free space.
    #[test]
    fn the_space_of_a_real_directory_is_read() {
        let (free, total) = crate::os::space_at(&std::env::temp_dir().join("no/such/dir")).unwrap();
        assert!(total > 0 && free <= total);
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
        // A local file is an administrator sheet on wold's staged copy (C7-1, ADR 0034): the
        // shell ran it through wol-admind; wold runs nothing and drops its copy.
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        assert!(handle(file, Some((1000, APP)), &all, &f).ok);
        assert!(f.ran.borrow().is_empty());
        assert_eq!(f.asked.borrow()[1].request, file_sheet());
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
                fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
                    self.0.inspect(s)
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

    /// Audit C7-1: a local file, the channel and Undo Update are administrator sheets the shell
    /// runs through wol-admind (which checks the administrator itself). An Allow Once from the
    /// caller's own shell, forged or confused, runs nothing as root; wold never runs them.
    #[test]
    fn root_changes_need_the_administrator_sheet() {
        let all = |_: &str, _: &str| true;
        for (line, want) in [
            (
                r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#,
                file_sheet(),
            ),
            (
                r#"{"call":"system.rollback"}"#,
                ApprovalRequest::SystemRollback,
            ),
            (
                r#"{"call":"system.channel","channel":"beta"}"#,
                ApprovalRequest::SystemChannel {
                    channel: "beta".into(),
                },
            ),
        ] {
            let call = PackageCall::decode(line.as_bytes()).unwrap();
            assert!(
                request(
                    &call,
                    Some((&format!("{STAGING}/n/x-1-1-x86_64.pkg.tar.zst"), &facts()))
                )
                .is_admin()
            );
            for answer in [Sheet::AllowOnce, Sheet::AllowAlways] {
                struct Raw(Fake, Sheet);
                impl System for Raw {
                    fn run(&self, a: &[String]) -> i32 {
                        self.0.run(a)
                    }
                    fn stage(&self, p: &str, u: u32) -> Option<String> {
                        self.0.stage(p, u)
                    }
                    fn discard(&self, s: &str) {
                        self.0.discard(s)
                    }
                    fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
                        self.0.inspect(s)
                    }
                    fn keys(&self) -> Vec<[u8; 32]> {
                        vec![]
                    }
                    fn running_release(&self) -> Option<String> {
                        None
                    }
                    fn ask(&self, _: u32, a: &Approval) -> Option<Sheet> {
                        self.0.asked.borrow_mut().push(a.clone());
                        Some(self.1.clone())
                    }
                    fn staging(&self) -> &Staging {
                        &self.0.staging
                    }
                }
                let f = Raw(Fake::default(), answer.clone());
                let a = handle(line, Some((1000, APP)), &all, &f);
                assert!(!a.ok, "{line} {answer:?}");
                assert_eq!(a.code, Some(Code::PermissionDenied), "{line}");
                assert!(f.0.ran.borrow().is_empty(), "{line}: nothing ran as root");
                assert_eq!(f.0.asked.borrow()[0].request, want, "{line}");
                let staged = matches!(want, ApprovalRequest::PackageFile { .. });
                assert_eq!(f.0.discarded.borrow().len(), usize::from(staged), "{line}");
            }
            // The password sheet: wol-admind ran it in the shell (Done); wold still runs nothing.
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
            fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
                self.fake.inspect(s)
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
        assert!(matches!(
            approval.request,
            ApprovalRequest::PackageFile { .. }
        ));
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
        assert!(f.ran.borrow().is_empty(), "wol-admind installed the copy");
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
            .ok,
            "wol-admind wrote the channel after the password"
        );
        assert!(f.ran.borrow().is_empty());
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

    /// One tar header of `kind` for `name`, then the data, padded to whole blocks.
    fn tar_member(name: &str, kind: u8, data: &[u8]) -> Vec<u8> {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut out = h.to_vec();
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
        out
    }

    /// A pax extended header holding `records`.
    fn pax(records: &[(&str, &str)]) -> Vec<u8> {
        let mut d = Vec::new();
        for (k, v) in records {
            let body = format!(" {k}={v}\n");
            // The length counts its own digits.
            let mut n = body.len() + 1;
            while format!("{n}{body}").len() != n {
                n += 1;
            }
            d.extend_from_slice(format!("{n}{body}").as_bytes());
        }
        tar_member("PaxHeader", b'x', &d)
    }

    const INFO: &[u8] =
        b"# Generated by makepkg\npkgname = hello\npkgbase = hello\npkgver = 1:2.0-3\nsize = 10\n";

    /// ADR 0034: the sheet's name and version come from the copy's .PKGINFO, found the way the
    /// package manager names members (pax path, long-name header), with every header checked.
    #[test]
    fn the_pkginfo_is_found_like_the_package_manager_finds_it() {
        let plain = [
            tar_member(".BUILDINFO", b'0', b"format = 2\n"),
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("usr/bin/hello", b'0', b"\x7fELF"),
        ]
        .concat();
        assert_eq!(pkginfo_in(&plain).unwrap(), INFO);
        assert_eq!(
            parse_pkginfo(std::str::from_utf8(INFO).unwrap()).unwrap(),
            ("hello".to_owned(), "1:2.0-3".to_owned())
        );
        // A pax path renames the member that follows it: that one is the .PKGINFO, and a header
        // called .PKGINFO that pax renames is not.
        let renamed = [
            pax(&[("path", "usr/share/x")]),
            tar_member(".PKGINFO", b'0', b"pkgname = fake\npkgver = 9-9\n"),
            pax(&[("path", ".PKGINFO"), ("mtime", "1.5")]),
            tar_member("whatever", b'0', INFO),
        ]
        .concat();
        assert_eq!(pkginfo_in(&renamed).unwrap(), INFO);
        let long = [
            tar_member("././@LongLink", b'L', b".PKGINFO\0"),
            tar_member("trunc", b'0', INFO),
        ]
        .concat();
        assert_eq!(pkginfo_in(&long).unwrap(), INFO);
        // Refusals: missing, damaged, not a file, too large, cut off.
        let none = [tar_member("usr/bin/hello", b'0', b"x"), vec![0; 1024]].concat();
        assert!(pkginfo_in(&none).unwrap_err().contains(".PKGINFO"));
        let mut damaged = plain.clone();
        damaged[1024 + 3] ^= 1;
        assert!(pkginfo_in(&damaged).unwrap_err().contains("damaged"));
        let link = tar_member(".PKGINFO", b'2', b"");
        assert!(pkginfo_in(&link).unwrap_err().contains("not a plain file"));
        let big = tar_member(".PKGINFO", b'0', &vec![b'#'; MAX_PKGINFO + 1]);
        assert!(pkginfo_in(&big).unwrap_err().contains("too large"));
        let cut = &plain[..512 * 3];
        assert!(pkginfo_in(cut).is_err(), "the .PKGINFO data is cut off");
        assert!(pkginfo_in(b"not a tar at all").is_err());
        assert!(pkginfo_in(&[]).is_err());
    }

    /// The values a sheet shows are short, plain and unambiguous; anything else is refused.
    #[test]
    fn pkginfo_values_are_checked_before_they_reach_a_sheet() {
        let p = parse_pkginfo;
        assert!(
            p("pkgname = a\npkgver = 1-1\npkgname = b\n")
                .unwrap_err()
                .contains("twice")
        );
        assert!(p("pkgname = a\n").unwrap_err().contains("no version"));
        assert!(p("pkgver = 1-1\n").unwrap_err().contains("no package name"));
        for bad in ["a b", "-a", ".a", "a\u{202e}b", "a/b", ""] {
            assert!(
                p(&format!("pkgname = {bad}\npkgver = 1-1\n")).is_err(),
                "{bad:?}"
            );
        }
        let long = "a".repeat(MAX_PKG_NAME + 1);
        assert!(p(&format!("pkgname = {long}\npkgver = 1-1\n")).is_err());
        let long = "1".repeat(MAX_PKG_VERSION + 1);
        for bad in ["1 2", "1;2", "", long.as_str()] {
            assert!(
                p(&format!("pkgname = a\npkgver = {bad}\n")).is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(
            p("pkgname = lib32-gcc_libs+x@y\npkgver = 2:14.2.1+r134~g1-2.1\n").unwrap(),
            ("lib32-gcc_libs+x@y".into(), "2:14.2.1+r134~g1-2.1".into())
        );
    }

    /// A package stream: `members`, then the end-of-archive blocks.
    fn package(members: &[Vec<u8>]) -> Vec<u8> {
        [members.concat(), vec![0; 1024]].concat()
    }

    /// ADR 0034: every tar header of the package is walked before the sheet, so a second
    /// .PKGINFO (anywhere, under any name that resolves to it) or a damaged or cut-off archive
    /// is refused, and a normal package passes.
    #[test]
    fn a_package_with_a_second_pkginfo_or_a_damaged_archive_is_refused() {
        let scan = |b: &[u8]| scan_package(&mut &b[..], MAX_UNPACKED);
        let body = tar_member("usr/share/big", b'0', &vec![7u8; 2 * PKGINFO_HEAD]);
        let normal = package(&[
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("usr/bin/", b'5', b""),
            body.clone(),
            tar_member("usr/bin/hi", b'2', b""),
        ]);
        assert_eq!(scan(&normal).unwrap(), INFO);
        let fake: &[u8] = b"pkgname = other\npkgver = 9-9\n";
        for (what, late) in [
            ("plain", vec![tar_member(".PKGINFO", b'0', fake)]),
            ("dot-slash", vec![tar_member("./.PKGINFO", b'0', fake)]),
            (
                "pax path",
                vec![pax(&[("path", ".PKGINFO")]), tar_member("x", b'0', fake)],
            ),
            (
                "long-name header",
                vec![
                    tar_member("././@LongLink", b'L', b"./.PKGINFO\0"),
                    tar_member("x", b'0', fake),
                ],
            ),
        ] {
            let mut m = vec![tar_member(".PKGINFO", b'0', INFO), body.clone()];
            m.extend(late);
            let p = package(&m);
            assert!(p.len() > PKGINFO_HEAD, "{what}: past the first MiB");
            let e = scan(&p).unwrap_err();
            assert!(e.contains("more than one"), "{what}: {e}");
        }
        // Cut off in a body, or with no end-of-archive block.
        let e = scan(&normal[..PKGINFO_HEAD + 700]).unwrap_err();
        assert!(e.contains("cut off"), "{e}");
        let e = scan(&normal[..normal.len() - 1024]).unwrap_err();
        assert!(e.contains("cut off"), "{e}");
        let mut damaged = normal.clone();
        damaged[1024 + 512 + 3] ^= 1;
        assert!(scan(&damaged).unwrap_err().contains("damaged"));
        // A link carrying data, a kind the walk does not know, more than the cap.
        let odd = package(&[
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("usr/bin/hi", b'2', b"hidden"),
        ]);
        assert!(scan(&odd).unwrap_err().contains("damaged"));
        let sparse = package(&[
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("s", b'S', b""),
        ]);
        assert!(scan(&sparse).unwrap_err().contains("kind of entry"));
        let e = scan_package(&mut &normal[..], 2 << 20).unwrap_err();
        assert!(e.contains("unpacks to more than"), "{e}");
        // The first .PKGINFO still has to be in the first MiB.
        let late = package(&[body.clone(), tar_member(".PKGINFO", b'0', INFO)]);
        assert!(scan(&late).unwrap_err().contains("at its start"));
    }

    /// End to end on a real .pkg.tar.zst (when the system zstd is there): name, version and the
    /// SHA-256 of the whole file, with a payload larger than the first MiB; a package with a
    /// second .PKGINFO past it, a damaged or a missing file is refused.
    #[test]
    fn a_staged_package_is_read_with_the_system_zstd() {
        let dir = std::env::temp_dir().join(format!("wol-pkginfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tar = [
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("usr/bin/hello", b'0', &vec![7u8; 3 * PKGINFO_HEAD]),
            vec![0; 1024],
        ]
        .concat();
        std::fs::write(dir.join("x.tar"), &tar).unwrap();
        let zst = dir.join("hello-2.0-3-x86_64.pkg.tar.zst");
        let made = std::process::Command::new("zstd")
            .args(["-q", "-f", "-o"])
            .arg(&zst)
            .arg(dir.join("x.tar"))
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("no zstd on this machine: end-to-end read skipped");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let f = inspect_staged(zst.to_str().unwrap()).unwrap();
        assert_eq!((f.name.as_str(), f.version.as_str()), ("hello", "1:2.0-3"));
        assert_eq!(f.sha256, wol_core::package::sha256_file(&zst).unwrap());
        assert_eq!(f.sha256.len(), 64);
        let twice = [
            tar_member(".PKGINFO", b'0', INFO),
            tar_member("usr/bin/hello", b'0', &vec![7u8; 3 * PKGINFO_HEAD]),
            tar_member(".PKGINFO", b'0', b"pkgname = other\npkgver = 9-9\n"),
            vec![0; 1024],
        ]
        .concat();
        std::fs::write(dir.join("x.tar"), &twice).unwrap();
        let made = std::process::Command::new("zstd")
            .args(["-q", "-f", "-o"])
            .arg(&zst)
            .arg(dir.join("x.tar"))
            .status()
            .unwrap();
        assert!(made.success());
        let e = inspect_staged(zst.to_str().unwrap()).unwrap_err();
        assert!(e.contains("more than one"), "{e}");
        std::fs::write(&zst, b"(\xb5/\xfd garbage").unwrap();
        assert!(inspect_staged(zst.to_str().unwrap()).is_err());
        assert!(inspect_staged(dir.join("gone.pkg.tar.zst").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR 0034: a copy whose .PKGINFO cannot be read is refused with the reason before any
    /// sheet shows, and the copy is dropped.
    #[test]
    fn an_unreadable_package_is_refused_before_the_sheet() {
        let all = |_: &str, _: &str| true;
        let f = Fake {
            unreadable: Some("its .PKGINFO has no version".into()),
            ..yes([0; 32])
        };
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        let a = handle(file, Some((1000, APP)), &all, &f);
        assert!(!a.ok);
        assert_eq!(a.code, Some(Code::InvalidArgument));
        assert_eq!(
            a.message.as_deref(),
            Some(
                "That package file can't be read (its .PKGINFO has no version), so it was not installed."
            )
        );
        assert!(f.asked.borrow().is_empty(), "no sheet");
        assert!(f.ran.borrow().is_empty());
        assert_eq!(f.discarded.borrow().len(), 1, "the copy is dropped");
    }
}
