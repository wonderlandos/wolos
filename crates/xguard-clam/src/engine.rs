// SPDX-License-Identifier: GPL-2.0-or-later
//! libclamav, and the socket in front of it.
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_ulong, c_void};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::Instant;

#[repr(C)]
#[derive(Default)]
struct ScanOptions {
    general: u32,
    parse: u32,
    heuristic: u32,
    mail: u32,
    dev: u32,
}

const CL_SUCCESS: c_int = 0;
const CL_VIRUS: c_int = 1;
const CL_INIT_DEFAULT: c_uint = 0;
/// CL_DB_PHISHING | CL_DB_PHISHING_URLS | CL_DB_BYTECODE, as clamscan loads by default. No PUA.
const CL_DB_STDOPT: c_uint = 0x2 | 0x8 | 0x2000;

#[link(name = "clamav")]
unsafe extern "C" {
    fn cl_init(options: c_uint) -> c_int;
    fn cl_engine_new() -> *mut c_void;
    fn cl_engine_free(e: *mut c_void) -> c_int;
    fn cl_engine_compile(e: *mut c_void) -> c_int;
    fn cl_load(path: *const c_char, e: *mut c_void, signo: *mut c_uint, opts: c_uint) -> c_int;
    fn cl_scandesc(
        fd: c_int,
        name: *const c_char,
        virname: *mut *const c_char,
        scanned: *mut c_ulong,
        e: *const c_void,
        opts: *mut ScanOptions,
    ) -> c_int;
    fn cl_strerror(code: c_int) -> *const c_char;
}

fn why(code: c_int) -> String {
    // SAFETY: cl_strerror returns a static string for any code.
    unsafe { CStr::from_ptr(cl_strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

struct Engine {
    e: *mut c_void,
    sigs: u32,
}

// SAFETY: a compiled engine is read-only; libclamav supports scans from many threads on one engine.
unsafe impl Send for Engine {}
// SAFETY: as above.
unsafe impl Sync for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: created by cl_engine_new, freed once.
        unsafe { cl_engine_free(self.e) };
    }
}

impl Engine {
    fn load(db: &str) -> Result<Self, String> {
        let path = CString::new(db).map_err(|_| "bad path".to_string())?;
        // SAFETY: plain library calls; `e` is checked, and freed by Drop on any early return.
        unsafe {
            let r = cl_init(CL_INIT_DEFAULT);
            if r != CL_SUCCESS {
                return Err(why(r));
            }
            let e = cl_engine_new();
            if e.is_null() {
                return Err("no engine".into());
            }
            let mut me = Self { e, sigs: 0 };
            let r = cl_load(path.as_ptr(), e, &mut me.sigs, CL_DB_STDOPT);
            if r != CL_SUCCESS {
                return Err(why(r));
            }
            let r = cl_engine_compile(e);
            if r != CL_SUCCESS {
                return Err(why(r));
            }
            Ok(me)
        }
    }

    /// The same parsers clamscan uses by default: archives, documents and executables.
    fn scan(&self, fd: RawFd) -> String {
        let mut opts = ScanOptions {
            parse: !0,
            ..ScanOptions::default()
        };
        let (mut virname, mut scanned): (*const c_char, c_ulong) = (std::ptr::null(), 0);
        // SAFETY: fd is open, engine compiled; virname points into the engine, read before return.
        let r = unsafe {
            cl_scandesc(
                fd,
                c"file".as_ptr(),
                &mut virname,
                &mut scanned,
                self.e,
                &mut opts,
            )
        };
        match r {
            CL_SUCCESS => "clean".into(),
            CL_VIRUS => {
                // SAFETY: set by libclamav on CL_VIRUS.
                let n = unsafe { CStr::from_ptr(virname) }.to_string_lossy();
                format!("found {n}")
            }
            e => format!("error {}", why(e)),
        }
    }
}

/// One message with one file descriptor (see `xguard::ctl::send_fd`).
fn recv_fd(s: &UnixStream) -> Option<OwnedFd> {
    let mut byte = [0u8; 64];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: byte.len(),
    };
    // SAFETY: CMSG_SPACE is a size computation.
    let mut ctl = vec![0u8; unsafe { libc::CMSG_SPACE(4) } as usize];
    // SAFETY: an all-zero msghdr is valid.
    let mut m: libc::msghdr = unsafe { std::mem::zeroed() };
    m.msg_iov = &mut iov;
    m.msg_iovlen = 1;
    m.msg_control = ctl.as_mut_ptr().cast();
    m.msg_controllen = ctl.len() as _;
    // SAFETY: buffers outlive the call; the header is read only as the kernel filled it.
    unsafe {
        if libc::recvmsg(s.as_raw_fd(), &mut m, libc::MSG_CMSG_CLOEXEC) <= 0 {
            return None;
        }
        let c = libc::CMSG_FIRSTHDR(&m);
        if c.is_null() || (*c).cmsg_level != libc::SOL_SOCKET || (*c).cmsg_type != libc::SCM_RIGHTS
        {
            return None;
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(c).cast::<RawFd>());
        Some(OwnedFd::from_raw_fd(fd))
    }
}

fn listener(socket: Option<&str>) -> Result<UnixListener, String> {
    if let Some(p) = socket {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::remove_file(p);
        let l = UnixListener::bind(p).map_err(|e| format!("{p}: {e}"))?;
        // A caller gains nothing: it passes a file it could already read, and hears one line.
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o666));
        return Ok(l);
    }
    let mine = std::env::var("LISTEN_PID").ok() == Some(std::process::id().to_string());
    if !mine || std::env::var("LISTEN_FDS").ok().as_deref() != Some("1") {
        return Err("no socket".into());
    }
    // SAFETY: systemd handed this descriptor to this process (LISTEN_PID checked).
    Ok(unsafe { UnixListener::from_raw_fd(3) })
}

fn serve(db: &str, socket: Option<&str>, idle_ms: i32) -> i32 {
    // The socket first: callers queue during the load instead of being refused.
    let l = match listener(socket) {
        Ok(l) => l,
        Err(m) => {
            eprintln!("wol-xguard-clam: {m}");
            return 3;
        }
    };
    let e = match Engine::load(db) {
        Ok(e) => Arc::new(e),
        Err(m) => {
            eprintln!("wol-xguard-clam: {db}: {m}");
            return 3;
        }
    };
    let busy = Arc::new(AtomicUsize::new(0));
    loop {
        let mut p = libc::pollfd {
            fd: l.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut p, 1, idle_ms) };
        if ready == 0 && busy.load(SeqCst) == 0 {
            return 0;
        }
        if ready <= 0 {
            continue;
        }
        let Ok((s, _)) = l.accept() else { continue };
        let (e, busy) = (e.clone(), busy.clone());
        busy.fetch_add(1, SeqCst);
        std::thread::spawn(move || {
            while let Some(fd) = recv_fd(&s) {
                let mut line = e.scan(fd.as_raw_fd());
                line.push('\n');
                if (&s).write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
            busy.fetch_sub(1, SeqCst);
        });
    }
}

fn peak_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            let l = t.lines().find(|l| l.starts_with("VmHWM:"))?.to_owned();
            l.split_whitespace().nth(1)?.parse().ok()
        })
        .unwrap_or(0)
}

fn scan_once(db: &str, path: &str) -> i32 {
    let t = Instant::now();
    let e = match Engine::load(db) {
        Ok(e) => e,
        Err(m) => {
            eprintln!("wol-xguard-clam: {db}: {m}");
            return 3;
        }
    };
    let load_ms = t.elapsed().as_millis();
    let Ok(f) = std::fs::File::open(path) else {
        eprintln!("wol-xguard-clam: {path}: cannot open");
        return 3;
    };
    let t = Instant::now();
    let line = e.scan(f.as_raw_fd());
    println!(
        "{line}\nsignatures={} load_ms={load_ms} scan_ms={} peak_rss_kb={}",
        e.sigs,
        t.elapsed().as_millis(),
        peak_kb()
    );
    match line.split(' ').next() {
        Some("clean") => 0,
        Some("found") => 1,
        _ => 3,
    }
}

pub fn main(args: &[String]) -> i32 {
    let (mut db, mut socket, mut idle, mut pos) = (None, None, 60, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().cloned(),
            "--socket" => socket = it.next().cloned(),
            "--idle" => idle = it.next().and_then(|v| v.parse().ok()).unwrap_or(60),
            p => pos.push(p.to_owned()),
        }
    }
    let db = db.unwrap_or_else(|| "/var/lib/wolos/xguard/clamav".into());
    match pos.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve"] => serve(&db, socket.as_deref(), idle * 1000),
        ["scan", path] => scan_once(&db, path),
        _ => {
            eprintln!(
                "usage: wol-xguard-clam serve|scan [--db DIR] [--socket PATH] [--idle SECS] [PATH]"
            );
            3
        }
    }
}
