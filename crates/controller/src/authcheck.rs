//! Pure parts of `wol-authcheck`: stdin password framing, PAM conversation policy, exit codes.

/// Longest password accepted from stdin (bytes, excluding the trailing newline).
pub const MAX_PASSWORD: usize = 512;

/// Exit codes: the only output of the helper.
pub const EXIT_OK: i32 = 0;
pub const EXIT_DENIED: i32 = 1;
pub const EXIT_ERROR: i32 = 2;
/// `--admin` used by a non-root caller or with a bad user name.
pub const EXIT_NOPERM: i32 = 77;

/// What to check: the real uid's password (lock screen, PAM `wol-lock`) or, for wol-admind only,
/// a named user's password (PAM `wol-admin`).
#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Lock,
    Admin(String),
}

/// argv after argv0. `[]` = Lock; `["--admin", user]` = Admin, real uid 0 only. Err = exit code.
pub fn mode(args: &[&str], real_uid: u32) -> Result<Mode, i32> {
    match args {
        [] => Ok(Mode::Lock),
        ["--admin", user] if real_uid == 0 && crate::login::valid_name(user) => {
            Ok(Mode::Admin((*user).into()))
        }
        ["--admin", ..] => Err(EXIT_NOPERM),
        _ => Err(EXIT_ERROR),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Auth {
    Ok,
    Denied,
    Error,
}

pub fn exit_code(a: &Auth) -> i32 {
    match a {
        Auth::Ok => EXIT_OK,
        Auth::Denied => EXIT_DENIED,
        Auth::Error => EXIT_ERROR,
    }
}

/// Stdin was not a valid password (too long, NUL byte, read error).
#[derive(Debug, PartialEq, Eq)]
pub struct BadInput;

/// Read the whole of `input` into `buf`; returns the password length. One trailing `\n` is dropped.
/// Rejects NUL bytes and input longer than `MAX_PASSWORD`.
pub fn read_password(
    input: &mut impl std::io::Read,
    buf: &mut [u8; MAX_PASSWORD + 2],
) -> Result<usize, BadInput> {
    let mut n = 0;
    while n < buf.len() {
        match input.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(BadInput),
        }
    }
    if n == buf.len() {
        return Err(BadInput);
    } // more input than any accepted password
    if n > 0 && buf[n - 1] == b'\n' {
        n -= 1;
    }
    if n > MAX_PASSWORD || buf[..n].contains(&0) {
        return Err(BadInput);
    }
    Ok(n)
}

pub const PROMPT_ECHO_OFF: i32 = 1;
pub const PROMPT_ECHO_ON: i32 = 2;
pub const ERROR_MSG: i32 = 3;
pub const TEXT_INFO: i32 = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Password,
    User,
    None,
    Fail,
}

/// Password only to hidden prompts; never echo it.
pub fn reply(style: i32) -> Reply {
    match style {
        PROMPT_ECHO_OFF => Reply::Password,
        PROMPT_ECHO_ON => Reply::User,
        ERROR_MSG | TEXT_INFO => Reply::None,
        _ => Reply::Fail,
    }
}

/// Overwrite `buf` so the compiler cannot elide it.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &[u8]) -> Result<Vec<u8>, BadInput> {
        let mut buf = [0u8; MAX_PASSWORD + 2];
        read_password(&mut &input[..], &mut buf).map(|n| buf[..n].to_vec())
    }

    #[test]
    fn password_framing() {
        let long = vec![b'a'; MAX_PASSWORD];
        let too_long = vec![b'a'; MAX_PASSWORD + 1];
        type Case<'a> = (&'a [u8], Result<&'a [u8], BadInput>);
        let cases: &[Case] = &[
            (b"hunter2", Ok(b"hunter2")),
            (b"hunter2\n", Ok(b"hunter2")),
            (b"two\n\n", Ok(b"two\n")),
            (b"", Ok(b"")),
            (b"\n", Ok(b"")),
            (b"a\0b", Err(BadInput)),
            (&long, Ok(&long)),
            (&too_long, Err(BadInput)),
        ];
        for (input, want) in cases {
            assert_eq!(
                read(input),
                want.as_ref().map(|w| w.to_vec()).map_err(|_| BadInput),
                "input {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn conversation_policy() {
        for (style, want) in [
            (PROMPT_ECHO_OFF, Reply::Password),
            (PROMPT_ECHO_ON, Reply::User),
            (ERROR_MSG, Reply::None),
            (TEXT_INFO, Reply::None),
            (99, Reply::Fail),
        ] {
            assert_eq!(reply(style), want);
        }
    }

    #[test]
    fn mode_table() {
        let cases: &[(&[&str], u32, Result<Mode, i32>)] = &[
            (&[], 1000, Ok(Mode::Lock)),
            (&[], 0, Ok(Mode::Lock)),
            (&["--admin", "ana"], 0, Ok(Mode::Admin("ana".into()))),
            (&["--admin", "ana"], 1000, Err(EXIT_NOPERM)),
            (&["--admin", "-rf"], 0, Err(EXIT_NOPERM)),
            (&["--admin", "a b"], 0, Err(EXIT_NOPERM)),
            (&["--admin"], 0, Err(EXIT_NOPERM)),
            (&["--admin", "ana", "x"], 0, Err(EXIT_NOPERM)),
            (&["ana"], 0, Err(EXIT_ERROR)),
        ];
        for (args, uid, want) in cases {
            assert_eq!(&mode(args, *uid), want, "{args:?} uid {uid}");
        }
    }

    #[test]
    fn exit_codes() {
        assert_eq!(
            [
                exit_code(&Auth::Ok),
                exit_code(&Auth::Denied),
                exit_code(&Auth::Error)
            ],
            [0, 1, 2]
        );
    }

    #[test]
    fn wipe_zeroes() {
        let mut b = *b"secret";
        wipe(&mut b);
        assert_eq!(b, [0; 6]);
    }
}
