// SPDX-License-Identifier: GPL-2.0-or-later
//! The SOCKS5 wire format (RFC 1928, username/password RFC 1929), the server side the browser
//! talks to and the client side the relay speaks to tor (ADR 0053 D2).
//!
//! Every read is a fixed-size `read_exact` (a length byte bounds the next one, at most 255), so a
//! peer cannot make the relay buffer more than 262 bytes per message. Only what Tor carries is
//! accepted: version 5, `CONNECT`, the three address types. SOCKS4/4a, `BIND` and
//! `UDP ASSOCIATE` are refused with the reply the RFC names.
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

/// SOCKS version byte.
pub const VERSION: u8 = 5;
/// Method: no authentication.
pub const NO_AUTH: u8 = 0x00;
/// Method: username/password (RFC 1929).
pub const USER_PASS: u8 = 0x02;
/// Method reply: no acceptable method.
pub const NO_METHOD: u8 = 0xff;

/// Reply codes (RFC 1928 section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// 0x00.
    Succeeded,
    /// 0x01: also what the relay answers when tor is not there (the kill switch).
    GeneralFailure,
    /// 0x02: refused by the relay's rules (an internal address).
    NotAllowed,
    /// 0x03.
    NetworkUnreachable,
    /// 0x04.
    HostUnreachable,
    /// 0x05.
    ConnectionRefused,
    /// 0x06.
    TtlExpired,
    /// 0x07: `BIND`, `UDP ASSOCIATE`.
    CommandNotSupported,
    /// 0x08.
    AddressTypeNotSupported,
    /// Any other byte tor sends (its extended onion errors are 0xF0..0xF7).
    Other(u8),
}

impl Reply {
    /// The wire byte.
    pub fn code(self) -> u8 {
        match self {
            Self::Succeeded => 0,
            Self::GeneralFailure => 1,
            Self::NotAllowed => 2,
            Self::NetworkUnreachable => 3,
            Self::HostUnreachable => 4,
            Self::ConnectionRefused => 5,
            Self::TtlExpired => 6,
            Self::CommandNotSupported => 7,
            Self::AddressTypeNotSupported => 8,
            Self::Other(b) => b,
        }
    }

    /// From the wire byte.
    pub fn from_code(b: u8) -> Self {
        match b {
            0 => Self::Succeeded,
            1 => Self::GeneralFailure,
            2 => Self::NotAllowed,
            3 => Self::NetworkUnreachable,
            4 => Self::HostUnreachable,
            5 => Self::ConnectionRefused,
            6 => Self::TtlExpired,
            7 => Self::CommandNotSupported,
            8 => Self::AddressTypeNotSupported,
            o => Self::Other(o),
        }
    }
}

/// Where a `CONNECT` goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    /// A name, resolved by the exit (never by this machine).
    Domain(String),
    /// An IPv4 literal.
    V4(Ipv4Addr),
    /// An IPv6 literal.
    V6(Ipv6Addr),
}

/// A `CONNECT` target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Host.
    pub host: Host,
    /// Port.
    pub port: u16,
}

/// A wire error: the connection is closed after the reply (if any) in `reply()`.
#[derive(Debug)]
pub enum SocksError {
    /// Read or write failed, or the peer closed.
    Io(io::Error),
    /// Not SOCKS5 (SOCKS4/4a starts with 4).
    Version(u8),
    /// No method the relay accepts.
    NoMethod,
    /// A command other than `CONNECT`.
    Command(u8),
    /// An address type other than 1, 3, 4, or a name that is not text.
    AddressType(u8),
    /// The username/password sub-negotiation was malformed or refused by tor.
    Auth,
}

impl SocksError {
    /// The reply a server sends before closing, when there is one.
    pub fn reply(&self) -> Option<Reply> {
        match self {
            Self::Command(_) => Some(Reply::CommandNotSupported),
            Self::AddressType(_) => Some(Reply::AddressTypeNotSupported),
            Self::Io(_) | Self::Version(_) | Self::NoMethod | Self::Auth => None,
        }
    }
}

impl fmt::Display for SocksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "the SOCKS connection ended early ({e})"),
            Self::Version(v) => write!(
                f,
                "SOCKS version {v} is not accepted; only SOCKS5 goes to Tor"
            ),
            Self::NoMethod => f.write_str("the SOCKS client offered no method the relay accepts"),
            Self::Command(c) => write!(
                f,
                "SOCKS command {c} is not carried over Tor (only CONNECT)"
            ),
            Self::AddressType(a) => write!(f, "SOCKS address type {a} is not accepted"),
            Self::Auth => f.write_str("the SOCKS username/password exchange did not complete"),
        }
    }
}

impl std::error::Error for SocksError {}

impl From<io::Error> for SocksError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn byte(r: &mut impl Read) -> Result<u8, SocksError> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn bytes(r: &mut impl Read, n: u8) -> Result<Vec<u8>, SocksError> {
    let mut v = vec![0u8; usize::from(n)];
    r.read_exact(&mut v)?;
    Ok(v)
}

/// Server: reads the method list and answers it. Returns the chosen method: username/password
/// when offered (the browser's isolation credential), else no-auth.
pub fn server_greeting(s: &mut (impl Read + Write)) -> Result<u8, SocksError> {
    let v = byte(s)?;
    if v != VERSION {
        return Err(SocksError::Version(v));
    }
    let n = byte(s)?;
    let methods = bytes(s, n)?;
    let chosen = if methods.contains(&USER_PASS) {
        USER_PASS
    } else if methods.contains(&NO_AUTH) {
        NO_AUTH
    } else {
        NO_METHOD
    };
    s.write_all(&[VERSION, chosen])?;
    if chosen == NO_METHOD {
        return Err(SocksError::NoMethod);
    }
    Ok(chosen)
}

/// Server: reads the RFC 1929 sub-negotiation and accepts it (the relay checks nothing: the
/// credential only chooses a circuit). Names that are not UTF-8 are refused.
pub fn server_userpass(s: &mut (impl Read + Write)) -> Result<(String, String), SocksError> {
    if byte(s)? != 1 {
        let _ = s.write_all(&[1, 1]);
        return Err(SocksError::Auth);
    }
    let ul = byte(s)?;
    let user = bytes(s, ul)?;
    let pl = byte(s)?;
    let pass = bytes(s, pl)?;
    let (Ok(user), Ok(pass)) = (String::from_utf8(user), String::from_utf8(pass)) else {
        let _ = s.write_all(&[1, 1]);
        return Err(SocksError::Auth);
    };
    s.write_all(&[1, 0])?;
    Ok((user, pass))
}

/// Server: reads a request; only `CONNECT` passes.
pub fn server_request(s: &mut impl Read) -> Result<Target, SocksError> {
    let v = byte(s)?;
    if v != VERSION {
        return Err(SocksError::Version(v));
    }
    let cmd = byte(s)?;
    let _reserved = byte(s)?;
    let atyp = byte(s)?;
    let host = read_host(s, atyp)?;
    let mut port = [0u8; 2];
    s.read_exact(&mut port)?;
    if cmd != 1 {
        return Err(SocksError::Command(cmd));
    }
    Ok(Target {
        host,
        port: u16::from_be_bytes(port),
    })
}

fn read_host(s: &mut impl Read, atyp: u8) -> Result<Host, SocksError> {
    match atyp {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a)?;
            Ok(Host::V4(Ipv4Addr::from(a)))
        }
        3 => {
            let n = byte(s)?;
            let name = bytes(s, n)?;
            match String::from_utf8(name) {
                Ok(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_graphic()) => {
                    Ok(Host::Domain(n.to_ascii_lowercase()))
                }
                _ => Err(SocksError::AddressType(3)),
            }
        }
        4 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a)?;
            Ok(Host::V6(Ipv6Addr::from(a)))
        }
        other => Err(SocksError::AddressType(other)),
    }
}

/// A reply with an all-zero bound address (the relay never tells the browser an address).
pub fn reply_bytes(r: Reply) -> [u8; 10] {
    [VERSION, r.code(), 0, 1, 0, 0, 0, 0, 0, 0]
}

fn request_bytes(t: &Target) -> Result<Vec<u8>, SocksError> {
    let mut out = vec![VERSION, 1, 0];
    match &t.host {
        Host::V4(a) => {
            out.push(1);
            out.extend_from_slice(&a.octets());
        }
        Host::Domain(d) => {
            let n = u8::try_from(d.len()).map_err(|_| SocksError::AddressType(3))?;
            out.push(3);
            out.push(n);
            out.extend_from_slice(d.as_bytes());
        }
        Host::V6(a) => {
            out.push(4);
            out.extend_from_slice(&a.octets());
        }
    }
    out.extend_from_slice(&t.port.to_be_bytes());
    Ok(out)
}

/// Client (relay to tor): username/password greeting, credential, `CONNECT`. Returns tor's reply;
/// the stream is ready to carry data when it is `Succeeded`.
pub fn client_connect(
    s: &mut (impl Read + Write),
    user: &str,
    pass: &str,
    t: &Target,
) -> Result<Reply, SocksError> {
    let (Ok(ul), Ok(pl)) = (u8::try_from(user.len()), u8::try_from(pass.len())) else {
        return Err(SocksError::Auth);
    };
    if ul == 0 {
        return Err(SocksError::Auth);
    }
    s.write_all(&[VERSION, 1, USER_PASS])?;
    let mut m = [0u8; 2];
    s.read_exact(&mut m)?;
    if m[0] != VERSION {
        return Err(SocksError::Version(m[0]));
    }
    if m[1] != USER_PASS {
        return Err(SocksError::NoMethod);
    }
    let mut auth = vec![1, ul];
    auth.extend_from_slice(user.as_bytes());
    auth.push(pl);
    auth.extend_from_slice(pass.as_bytes());
    s.write_all(&auth)?;
    let mut a = [0u8; 2];
    s.read_exact(&mut a)?;
    if a[1] != 0 {
        return Err(SocksError::Auth);
    }
    s.write_all(&request_bytes(t)?)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[0] != VERSION {
        return Err(SocksError::Version(head[0]));
    }
    let _bound = read_host(s, head[3])?;
    let mut port = [0u8; 2];
    s.read_exact(&mut port)?;
    Ok(Reply::from_code(head[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A duplex buffer: reads from `input`, collects writes in `output`.
    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.input.read(b)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn duplex(input: &[u8]) -> Duplex {
        Duplex {
            input: Cursor::new(input.to_vec()),
            output: vec![],
        }
    }

    #[test]
    fn greeting_prefers_user_pass() {
        let mut d = duplex(&[5, 2, 0, 2]);
        assert_eq!(server_greeting(&mut d).unwrap(), USER_PASS);
        assert_eq!(d.output, [5, 2]);
        let mut d = duplex(&[5, 1, 0]);
        assert_eq!(server_greeting(&mut d).unwrap(), NO_AUTH);
    }

    #[test]
    fn socks4_and_gssapi_only_are_refused() {
        assert!(matches!(
            server_greeting(&mut duplex(&[4, 1, 0, 80])),
            Err(SocksError::Version(4))
        ));
        let mut d = duplex(&[5, 1, 1]);
        assert!(matches!(server_greeting(&mut d), Err(SocksError::NoMethod)));
        assert_eq!(d.output, [5, 0xff]);
    }

    #[test]
    fn userpass_is_read_and_acked() {
        let mut d = duplex(&[1, 3, b'a', b'.', b'b', 1, b'7']);
        assert_eq!(server_userpass(&mut d).unwrap(), ("a.b".into(), "7".into()));
        assert_eq!(d.output, [1, 0]);
        let mut d = duplex(&[1, 1, 0xff, 0]);
        assert!(matches!(server_userpass(&mut d), Err(SocksError::Auth)));
    }

    #[test]
    fn connect_requests_parse_all_three_address_types() {
        let mut req = vec![5, 1, 0, 3, 11];
        req.extend_from_slice(b"Example.ORG");
        req.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(
            server_request(&mut Cursor::new(req)).unwrap(),
            Target {
                host: Host::Domain("example.org".into()),
                port: 443
            }
        );
        let r = server_request(&mut Cursor::new(vec![5, 1, 0, 1, 192, 0, 2, 1, 0, 80])).unwrap();
        assert_eq!(r.host, Host::V4(Ipv4Addr::new(192, 0, 2, 1)));
        let mut v6 = vec![5, 1, 0, 4];
        v6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        v6.extend_from_slice(&[0, 22]);
        assert_eq!(
            server_request(&mut Cursor::new(v6)).unwrap().host,
            Host::V6(Ipv6Addr::LOCALHOST)
        );
    }

    #[test]
    fn bind_and_udp_associate_are_not_supported() {
        for cmd in [2u8, 3] {
            let e = server_request(&mut Cursor::new(vec![5, cmd, 0, 1, 192, 0, 2, 1, 0, 53]))
                .unwrap_err();
            assert!(matches!(e, SocksError::Command(c) if c == cmd));
            assert_eq!(e.reply(), Some(Reply::CommandNotSupported));
        }
        let e = server_request(&mut Cursor::new(vec![5, 1, 0, 9, 0, 0])).unwrap_err();
        assert_eq!(e.reply(), Some(Reply::AddressTypeNotSupported));
    }

    #[test]
    fn client_side_sends_credential_and_reads_reply() {
        let tor_says = [5, 2, 1, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let mut d = duplex(&tor_says);
        let t = Target {
            host: Host::Domain("example.org".into()),
            port: 443,
        };
        assert_eq!(
            client_connect(&mut d, "g/example.org/0", "x", &t).unwrap(),
            Reply::Succeeded
        );
        let mut want = vec![5, 1, 2, 1, 15];
        want.extend_from_slice(b"g/example.org/0");
        want.extend_from_slice(&[1, b'x', 5, 1, 0, 3, 11]);
        want.extend_from_slice(b"example.org");
        want.extend_from_slice(&[1, 187]);
        assert_eq!(d.output, want);
    }

    #[test]
    fn reply_codes_round_trip_through_bytes() {
        for b in 0..=255u8 {
            assert_eq!(Reply::from_code(b).code(), b);
        }
        assert_eq!(reply_bytes(Reply::GeneralFailure)[1], 1);
    }
}
