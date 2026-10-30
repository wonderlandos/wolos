//! Minimal PAM client (libpam on FreeBSD and Linux): password-only conversation, no echo, no logging.
#![allow(unsafe_code)]

use crate::authcheck::{Auth, Reply, reply, wipe};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_SILENT: c_int = 0x8000_0000u32 as c_int;

#[repr(C)]
struct PamMessage {
    style: c_int,
    msg: *const c_char,
}
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    retcode: c_int,
}
#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    data: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        h: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(h: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(h: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(h: *mut c_void, status: c_int) -> c_int;
    fn pam_set_item(h: *mut c_void, item: c_int, value: *const c_void) -> c_int;
    fn pam_putenv(h: *mut c_void, name_value: *const c_char) -> c_int;
    fn pam_getenvlist(h: *mut c_void) -> *mut *mut c_char;
    fn pam_setcred(h: *mut c_void, flags: c_int) -> c_int;
    fn pam_open_session(h: *mut c_void, flags: c_int) -> c_int;
    fn pam_close_session(h: *mut c_void, flags: c_int) -> c_int;
}
const PAM_TTY: c_int = 3;
const PAM_ESTABLISH_CRED: c_int = 0x2;
const PAM_DELETE_CRED: c_int = 0x4;
unsafe extern "C" {
    fn getuid() -> u32;
    fn getpwuid_r(
        uid: u32,
        pw: *mut c_void,
        buf: *mut c_char,
        len: usize,
        out: *mut *mut c_void,
    ) -> c_int;
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

struct Call<'a> {
    password: &'a [u8],
    user: &'a CStr,
}

/// Login name of the real uid. `pw_name` is the first field of `struct passwd` on FreeBSD and Linux.
pub fn real_user() -> Option<CString> {
    #[repr(C, align(8))]
    struct Pw([u8; 256]);
    let mut pw = Pw([0; 256]);
    let mut buf = vec![0 as c_char; 4096];
    let mut out = std::ptr::null_mut();
    let rc = unsafe {
        getpwuid_r(
            getuid(),
            pw.0.as_mut_ptr().cast(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut out,
        )
    };
    if rc != 0 || out.is_null() {
        return None;
    }
    let name = unsafe { *(out as *const *const c_char) };
    if name.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(name) }.to_owned())
}

/// Malloc'd NUL-terminated copy; PAM owns and frees it.
fn cdup(bytes: &[u8]) -> *mut c_char {
    let p = unsafe { calloc(bytes.len() + 1, 1) } as *mut u8;
    if !p.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) }
    }
    p.cast()
}

extern "C" fn converse(
    n: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if !(1..=32).contains(&n) || msg.is_null() || resp.is_null() {
        return PAM_CONV_ERR;
    }
    let call = unsafe { &*(data as *const Call) };
    let n = n as usize;
    let r = unsafe { calloc(n, size_of::<PamResponse>()) } as *mut PamResponse;
    if r.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..n {
        let style = unsafe { (**msg.add(i)).style };
        let p = match reply(style) {
            Reply::Password => cdup(call.password),
            Reply::User => cdup(call.user.to_bytes()),
            Reply::None => std::ptr::null_mut(),
            Reply::Fail => {
                for j in 0..i {
                    let q = unsafe { (*r.add(j)).resp };
                    if !q.is_null() {
                        unsafe {
                            wipe(std::slice::from_raw_parts_mut(
                                q.cast(),
                                CStr::from_ptr(q).count_bytes(),
                            ));
                            free(q.cast())
                        }
                    }
                }
                unsafe { free(r.cast()) };
                return PAM_CONV_ERR;
            }
        };
        unsafe { (*r.add(i)).resp = p };
    }
    unsafe { *resp = r };
    PAM_SUCCESS
}

/// PAM `service` check of `user`'s password plus account management.
pub fn authenticate(service: &CStr, user: &CStr, password: &[u8]) -> Auth {
    let call = Call { password, user };
    let conv = PamConv {
        conv: converse,
        data: (&call as *const Call).cast_mut().cast(),
    };
    let mut h = std::ptr::null_mut();
    if unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut h) } != PAM_SUCCESS {
        return Auth::Error;
    }
    let mut rc = unsafe { pam_authenticate(h, PAM_SILENT) };
    if rc == PAM_SUCCESS {
        rc = unsafe { pam_acct_mgmt(h, PAM_SILENT) };
    }
    unsafe { pam_end(h, rc) };
    if rc == PAM_SUCCESS {
        Auth::Ok
    } else {
        Auth::Denied
    }
}

/// An open PAM session (already authenticated user): closed, credentials deleted, on drop.
pub struct Session {
    h: *mut c_void,
    /// The environment the modules set (pam_systemd: XDG_RUNTIME_DIR, XDG_SESSION_ID).
    pub env: Vec<String>,
}

/// pam_acct_mgmt, pam_setcred and pam_open_session for `user` on `tty`; `env` (`NAME=value`) goes
/// to the modules first (pam_systemd reads XDG_SESSION_TYPE, XDG_SEAT, XDG_VTNR). Root only.
pub fn open_session(service: &CStr, user: &CStr, tty: &CStr, env: &[&str]) -> Option<Session> {
    let call = Call {
        password: b"",
        user,
    };
    let conv = PamConv {
        conv: converse,
        data: (&call as *const Call).cast_mut().cast(),
    };
    let mut h = std::ptr::null_mut();
    // SAFETY: valid C strings; `conv` and `call` outlive every PAM call that may converse (all
    // conversation happens before this function returns: session modules do not prompt).
    if unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut h) } != PAM_SUCCESS {
        return None;
    }
    let fail = |h, rc| {
        // SAFETY: `h` came from pam_start.
        unsafe { pam_end(h, rc) };
        None
    };
    // SAFETY: `h` from pam_start; strings are copied by libpam.
    let mut rc = unsafe { pam_set_item(h, PAM_TTY, tty.as_ptr().cast()) };
    for e in env {
        let Ok(e) = CString::new(*e) else {
            return fail(h, PAM_BUF_ERR);
        };
        if rc == PAM_SUCCESS {
            rc = unsafe { pam_putenv(h, e.as_ptr()) };
        }
    }
    if rc == PAM_SUCCESS {
        rc = unsafe { pam_acct_mgmt(h, PAM_SILENT) };
    }
    if rc == PAM_SUCCESS {
        rc = unsafe { pam_setcred(h, PAM_ESTABLISH_CRED | PAM_SILENT) };
    }
    if rc == PAM_SUCCESS {
        rc = unsafe { pam_open_session(h, PAM_SILENT) };
    }
    if rc != PAM_SUCCESS {
        return fail(h, rc);
    }
    let mut out = vec![];
    // SAFETY: NULL-terminated array of malloc'd strings, all owned by us.
    let list = unsafe { pam_getenvlist(h) };
    if !list.is_null() {
        let mut i = 0;
        loop {
            let p = unsafe { *list.add(i) };
            if p.is_null() {
                break;
            }
            out.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
            unsafe { free(p.cast()) };
            i += 1;
        }
        unsafe { free(list.cast()) };
    }
    Some(Session { h, env: out })
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: `h` is the open handle from open_session, ended exactly once here.
        unsafe {
            let rc = pam_close_session(self.h, PAM_SILENT);
            pam_setcred(self.h, PAM_DELETE_CRED | PAM_SILENT);
            pam_end(self.h, rc);
        }
    }
}
