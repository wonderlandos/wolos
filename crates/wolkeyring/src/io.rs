// SPDX-License-Identifier: BSD-2-Clause
//! Every file, process and socket access of the keyring (ADR 0054). The core modules stay pure;
//! this layer reads and writes the store and the audit log (0600, never through a symlink,
//! appends `fsync`ed, rewrites atomic), seals and opens factor secrets with the TPM through
//! `systemd-creds`, asks `fprintd-verify` and `fido2-assert`, and says who a peer process is
//! from the kernel (a pinned `/proc/<pid>` directory, STANDARDS 7.1 "Identity").
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

use crate::item::AppId;
use crate::{Error, Result};

fn io(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Io(format!("{what}: {e}"))
}

/// `$XDG_DATA_HOME/wolos/keyring`, else `$HOME/.local/share/wolos/keyring`.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("WOLOS_KEYRING_DIR") {
        return Ok(PathBuf::from(d));
    }
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or(Error::Io("HOME is not set".into()))?)
            .join(".local/share"),
    };
    Ok(base.join("wolos/keyring"))
}

/// The store file of keyring `name`.
pub fn store_path(name: &str) -> Result<PathBuf> {
    check_name(name)?;
    Ok(data_dir()?.join(format!("{name}.wkr")))
}

/// The folder of the keyring's rollback record, deliberately not the data folder: a backup or a
/// copy of `~/.local/share/wolos/keyring` does not carry it. `WOLOS_KEYRING_STATE_DIR`, else
/// `$XDG_STATE_HOME/wolos/keyring`, else `$HOME/.local/state/wolos/keyring`; under a test's
/// `WOLOS_KEYRING_DIR`, its `state` subfolder.
pub fn state_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("WOLOS_KEYRING_STATE_DIR") {
        return Ok(PathBuf::from(d));
    }
    if let Some(d) = std::env::var_os("WOLOS_KEYRING_DIR") {
        return Ok(PathBuf::from(d).join("state"));
    }
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or(Error::Io("HOME is not set".into()))?)
            .join(".local/state"),
    };
    Ok(base.join("wolos/keyring"))
}

/// The rollback record of keyring `name`.
pub fn state_path(name: &str) -> Result<PathBuf> {
    check_name(name)?;
    Ok(state_dir()?.join(format!("{name}.state")))
}

/// The audit log of keyring `name`.
pub fn audit_path(name: &str) -> Result<PathBuf> {
    check_name(name)?;
    Ok(data_dir()?.join(format!("{name}.audit")))
}

/// The fast-path socket: `$XDG_RUNTIME_DIR/wolos/keyring.sock`.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(s) = std::env::var_os("WOLOS_KEYRING_SOCKET") {
        return Ok(PathBuf::from(s));
    }
    let r = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or(Error::Io("XDG_RUNTIME_DIR is not set".into()))?;
    Ok(PathBuf::from(r).join("wolos/keyring.sock"))
}

fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(()).ok_or(Error::Malformed("keyring name"))
}

fn open_opts() -> std::fs::OpenOptions {
    let mut o = std::fs::OpenOptions::new();
    o.mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    o
}

/// Reads a file of at most `max` bytes; Ok(None) when it does not exist.
pub fn read(path: &Path, max: usize) -> Result<Option<Vec<u8>>> {
    let mut f = match open_opts().read(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("the keyring file could not be opened", e)),
    };
    let mut v = Vec::new();
    Read::by_ref(&mut f)
        .take(max as u64 + 1)
        .read_to_end(&mut v)
        .map_err(|e| io("the keyring file could not be read", e))?;
    if v.len() > max {
        return Err(Error::Limit("file size"));
    }
    Ok(Some(v))
}

/// Appends and syncs.
pub fn append(path: &Path, bytes: &[u8]) -> Result<()> {
    append_with(path, bytes, true)
}

/// Appends without waiting for the disk: the bytes are in the file for every reader at once and
/// survive a crash of this process; a power cut can lose them until the next [`append`] to the
/// same file syncs it (the audit log's reads, ADR 0054 section 6).
pub fn append_lazy(path: &Path, bytes: &[u8]) -> Result<()> {
    append_with(path, bytes, false)
}

fn append_with(path: &Path, bytes: &[u8], sync: bool) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| io("the keyring folder could not be made", e))?;
    }
    let mut f = open_opts()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|e| io("the keyring file could not be opened", e))?;
    f.write_all(bytes)
        .map_err(|e| io("the keyring file could not be written", e))?;
    if sync {
        f.sync_data()
            .map_err(|e| io("the keyring file could not be synced", e))?;
    }
    Ok(())
}

/// Replaces a file atomically: a temp file in the same folder, synced, renamed, the folder synced.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or(Error::Io("no folder".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| io("the keyring folder could not be made", e))?;
    let tmp = dir.join(format!(
        ".{}.tmp{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("keyring"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp);
    let mut f = open_opts()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| io("the keyring file could not be made", e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| io("the keyring file could not be written", e))?;
    std::fs::rename(&tmp, path).map_err(|e| io("the keyring file could not be replaced", e))?;
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| io("the keyring folder could not be synced", e))
}

/// The keyring's write lock: every append to the store or the audit log, by the daemon or the
/// command line, happens under it after reading what the other wrote. Held while the returned
/// file is open; waits for the other holder.
pub fn lock(name: &str) -> Result<std::fs::File> {
    let p = data_dir()?.join(format!("{name}.lock"));
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| io("the keyring folder could not be made", e))?;
    }
    let f = open_opts()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&p)
        .map_err(|e| io("the keyring lock could not be opened", e))?;
    f.lock()
        .map_err(|e| io("the keyring lock could not be taken", e))?;
    Ok(f)
}

/// (inode, length) of a file, or None when it does not exist.
pub fn stat(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.ino(), m.len()))
}

/// The bytes of a file from `from` on (at most `max`).
pub fn read_from(path: &Path, from: u64, max: usize) -> Result<Vec<u8>> {
    use std::io::Seek;
    let mut f = open_opts()
        .read(true)
        .open(path)
        .map_err(|e| io("the keyring file could not be opened", e))?;
    f.seek(std::io::SeekFrom::Start(from))
        .map_err(|e| io("the keyring file could not be read", e))?;
    let mut v = Vec::new();
    Read::by_ref(&mut f)
        .take(max as u64 + 1)
        .read_to_end(&mut v)
        .map_err(|e| io("the keyring file could not be read", e))?;
    if v.len() > max {
        return Err(Error::Limit("file size"));
    }
    Ok(v)
}

/// Unix seconds now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether the machine has a TPM 2.0.
pub fn has_tpm2() -> bool {
    std::fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major").is_ok_and(|s| s.trim() == "2")
}

fn run_stdin(argv: &[&str], input: &[u8], max_out: usize) -> Result<Zeroizing<Vec<u8>>> {
    let (prog, args) = argv.split_first().ok_or(Error::Io("no program".into()))?;
    let mut c = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| io(&format!("{prog} could not start"), e))?;
    if let Some(mut i) = c.stdin.take() {
        i.write_all(input)
            .map_err(|e| io(&format!("{prog} input"), e))?;
    }
    let mut out = Zeroizing::new(Vec::new());
    if let Some(o) = c.stdout.take() {
        o.take(max_out as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|e| io(&format!("{prog} output"), e))?;
    }
    let st = c
        .wait()
        .map_err(|e| io(&format!("{prog} did not finish"), e))?;
    if !st.success() || out.len() > max_out {
        return Err(Error::Io(format!("{prog} refused ({st})")));
    }
    Ok(out)
}

/// Seals 32 bytes to this machine's TPM2 (`systemd-creds --user encrypt --with-key=tpm2`), as
/// the store keeps it; the credential name binds it to the keyring and factor.
pub fn tpm_seal(name: &str, secret: &[u8; 32]) -> Result<Vec<u8>> {
    let n = format!("--name={name}");
    Ok(run_stdin(
        &[
            "systemd-creds",
            "--user",
            "encrypt",
            "--with-key=tpm2",
            // Explicit, so a change of systemd's default cannot change what the factor is bound
            // to (ADR 0054 section 3): the Secure Boot state, PCR 7, and nothing else.
            "--tpm2-pcrs=7",
            &n,
            "-",
            "-",
        ],
        secret,
        64 * 1024,
    )?
    .to_vec())
}

/// Opens what [`tpm_seal`] made.
pub fn tpm_open(name: &str, blob: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let n = format!("--name={name}");
    let out = run_stdin(
        &["systemd-creds", "--user", "decrypt", &n, "-", "-"],
        blob,
        64,
    )?;
    let mut s = Zeroizing::new([0u8; 32]);
    if out.len() != 32 {
        return Err(Error::Damaged("security chip secret length"));
    }
    s.copy_from_slice(&out);
    Ok(s)
}

/// Asks fprintd for a fingerprint match of the current user (`fprintd-verify`, exit 0 = match).
/// `Ok(false)` is a reader that said no; an error is a program that could not run (no fprintd), so
/// the caller can tell "not you" from "cannot ask".
pub fn fingerprint_matches() -> Result<bool> {
    let status = Command::new("fprintd-verify")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| io("fprintd-verify", e))?;
    Ok(status.success())
}

/// The hmac-secret output of a FIDO2 key for a credential and salt (`fido2-assert -G -h`, input
/// lines: client data hash, relying party, credential id, hmac salt, all base64 except the
/// relying party). `device` is a hidraw path from `fido2-token -L`.
pub fn fido2_hmac(device: &str, credential: &[u8], salt: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>> {
    let std64 = |b: &[u8]| {
        crate::wire::b64(b).replace('-', "+").replace('_', "/")
            + match b.len() % 3 {
                1 => "==",
                2 => "=",
                _ => "",
            }
    };
    let input = format!(
        "{}\nwolos.keyring\n{}\n{}\n",
        std64(&[0u8; 32]),
        std64(credential),
        std64(salt)
    );
    let out = run_stdin(
        &["fido2-assert", "-G", "-h", device],
        input.as_bytes(),
        4096,
    )?;
    let text = String::from_utf8_lossy(&out);
    // The hmac-secret is the last line, standard base64 of 32 bytes.
    let last = text
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .replace('+', "-")
        .replace('/', "_");
    let v = crate::wire::unb64(last.trim_end_matches('='), 32)?;
    let mut s = Zeroizing::new([0u8; 32]);
    if v.len() != 32 {
        return Err(Error::Damaged("security key answer"));
    }
    s.copy_from_slice(&v);
    Ok(s)
}

/// A peer process, pinned at accept: `/proc/<pid>` opened as a directory once, everything read
/// through that descriptor, so a reused pid fails instead of naming the newcomer.
pub struct Pinned {
    dir: std::fs::File,
    pub uid: u32,
    pub pid: i32,
}

impl Pinned {
    /// SO_PEERCRED of a Unix socket and its pinned process.
    pub fn of(stream: &impl AsRawFd) -> Option<Pinned> {
        let p = shared::sys::peer(stream)?;
        Self::pid(p.pid, p.uid)
    }

    /// A pid learnt from the bus, pinned now (the bus said it a moment ago; ADR 0054 section 4
    /// names the window).
    pub fn pid(pid: i32, uid: u32) -> Option<Pinned> {
        (pid > 0).then_some(())?;
        let dir = std::fs::File::open(format!("/proc/{pid}")).ok()?;
        Some(Pinned { dir, uid, pid })
    }

    pub(crate) fn at(&self, what: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}/{what}", self.dir.as_raw_fd()))
    }

    /// The canonical executable path.
    pub fn exe(&self) -> Option<String> {
        let t = std::fs::read_link(self.at("exe")).ok()?;
        let s = t.into_os_string().into_string().ok()?;
        (s.len() < 4096).then_some(s)
    }

    /// (device, inode, change time, length) of the process's executable, read through the pinned
    /// directory: it changes when the process executes another program or its file is replaced.
    pub fn exe_key(&self) -> Option<(u64, u64, i64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(self.at("exe")).ok()?;
        Some((m.dev(), m.ino(), m.ctime(), m.len()))
    }

    /// The systemd app id of the process's scope or unit, from its cgroup
    /// (`app-<id>@<n>.scope`, `app-flatpak-<id>-<n>.scope`, `app-<id>.service`).
    pub fn app(&self) -> Option<String> {
        let cg = std::fs::read_to_string(self.at("cgroup")).ok()?;
        app_of_cgroup(&cg)
    }

    /// The identity the access lists use (executable digest from `digest`, a cache the daemon
    /// keeps by device, inode and change time).
    pub fn app_id(&self, digest: &mut dyn FnMut(&Path) -> Option<String>) -> Option<AppId> {
        let exe = self.exe()?;
        let sha256 = digest(&self.at("exe"));
        Some(AppId {
            exe,
            sha256,
            app: self.app(),
            signer: None,
        })
    }
}

/// The app id named by a cgroup file's last unit.
pub fn app_of_cgroup(cg: &str) -> Option<String> {
    let path = cg.lines().find_map(|l| l.strip_prefix("0::"))?;
    let unit = path.rsplit('/').find(|u| u.starts_with("app-"))?;
    let u = unit.strip_prefix("app-")?;
    let u = u.strip_prefix("flatpak-").unwrap_or(u);
    let u = u
        .strip_suffix(".scope")
        .or_else(|| u.strip_suffix(".service"))
        .unwrap_or(u);
    let u = u.split('@').next().unwrap_or(u);
    // Flatpak scopes end with -<number>.
    let u = match u.rsplit_once('-') {
        Some((a, n)) if n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() => a,
        _ => u,
    };
    (!u.is_empty() && u.len() <= 255).then(|| u.to_string())
}

/// The most of an executable that is hashed to identify a caller (64 MiB: every program of the
/// system and every packaged app fits; a bigger file is a bundle, not what a digest ACL names).
pub const MAX_HASHED: u64 = 64 << 20;

/// What hashing a caller's executable gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hashed {
    /// SHA-256, hex.
    Digest(String),
    /// Longer than [`MAX_HASHED`]: no digest, so a grant bound to a digest cannot match this
    /// caller (QA-A9-SEC-4); the file is not read.
    TooLarge,
}

/// SHA-256 of a file, hex, read in 1 MiB pieces; a file longer than [`MAX_HASHED`] is not read
/// (it is [`Hashed::TooLarge`], also when it grows past the cap while read). None when it cannot
/// be read.
pub fn sha256_file(path: &Path) -> Option<Hashed> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > MAX_HASHED {
        return Some(Hashed::TooLarge);
    }
    let mut r = std::io::BufReader::with_capacity(1 << 20, f.take(MAX_HASHED + 1));
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    let mut total = 0u64;
    loop {
        let buf = r.fill_buf().ok()?;
        if buf.is_empty() {
            break;
        }
        total += buf.len() as u64;
        h.update(buf);
        let n = buf.len();
        r.consume(n);
    }
    if total > MAX_HASHED {
        return Some(Hashed::TooLarge);
    }
    Some(Hashed::Digest(crate::wire::hex(&h.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_from_cgroups() {
        assert_eq!(app_of_cgroup("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.wolos.Files@123.scope\n").as_deref(), Some("org.wolos.Files"));
        assert_eq!(app_of_cgroup("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.discordapp.Discord-4242.scope\n").as_deref(), Some("com.discordapp.Discord"));
        assert_eq!(
            app_of_cgroup("0::/user.slice/user-1000.slice/session-2.scope\n"),
            None
        );
    }

    #[test]
    fn names_are_checked() {
        assert!(store_path("../x").is_err());
        assert!(check_name("login").is_ok());
    }
}
