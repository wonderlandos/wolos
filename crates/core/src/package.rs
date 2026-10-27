//! `.wol` package files (ADR 0011, archicture/modules/packages.md): read and verify the ustar
//! archive (hashes, Ed25519 signature, Info.json), extract the payload, plan the pkg(8) install.
use shared::bundle::{INFO_FILE, Info};
use shared::package::{
    Entry, Index, MANIFEST, MAX_SIZE, MEMBERS, PAYLOAD, SIGNATURE, is_app_id, is_version,
};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const BLOCK: u64 = 512;
/// Largest `Info.json` / `SHA256` / icon read into memory.
const MAX_SMALL: u64 = 1 << 20;

/// One archive member: where its bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Verified by the trusted key at this index.
    Signed(usize),
    /// No signature member.
    Unsigned,
}

/// A package file that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub info: Info,
    pub trust: Trust,
    pub payload: Member,
    pub icon: Option<Member>,
}

fn octal(b: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(b).ok()?;
    let s = s.trim_matches(|c| c == '\0' || c == ' ');
    if s.is_empty() || !s.bytes().all(|c| (b'0'..=b'7').contains(&c)) {
        return None;
    }
    u64::from_str_radix(s, 8).ok()
}

fn field(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())]
}

/// Regular-file members of a ustar archive, in order. Anything else (links, dirs, pax/GNU
/// headers, prefixes, bad checksums, truncation) is an error.
pub fn members(path: &Path) -> Result<Vec<Member>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len > MAX_SIZE {
        return Err("package too large".into());
    }
    let mut out = Vec::new();
    let mut pos = 0u64;
    let mut h = [0u8; BLOCK as usize];
    loop {
        if pos + BLOCK > len {
            return Err("truncated archive".into());
        }
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        f.read_exact(&mut h).map_err(|e| e.to_string())?;
        if h.iter().all(|b| *b == 0) {
            return Ok(out);
        }
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
        if octal(&h[148..156]) != Some(sum) {
            return Err("bad header checksum".into());
        }
        if &h[257..262] != b"ustar" {
            return Err("not a ustar archive".into());
        }
        if !matches!(h[156], b'0' | 0) || !field(&h[345..500]).is_empty() {
            return Err("only plain files are allowed".into());
        }
        let name = std::str::from_utf8(field(&h[..100]))
            .map_err(|_| "member name not UTF-8".to_string())?
            .to_owned();
        let size = octal(&h[124..136]).ok_or("bad member size")?;
        let offset = pos + BLOCK;
        let next = offset + size.div_ceil(BLOCK) * BLOCK;
        if offset + size > len {
            return Err("truncated archive".into());
        }
        out.push(Member { name, offset, size });
        pos = next;
    }
}

/// Bytes of a small member (at most `max`).
pub fn read_member(path: &Path, m: &Member, max: u64) -> Result<Vec<u8>, String> {
    if m.size > max {
        return Err(format!("{} too large", m.name));
    }
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(m.offset))
        .map_err(|e| e.to_string())?;
    let mut b = vec![0; m.size as usize];
    f.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}

fn hash_member(path: &Path, m: &Member) -> Result<String, String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(m.offset))
        .map_err(|e| e.to_string())?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f.take(m.size), &mut h).map_err(|e| e.to_string())?;
    Ok(hex(h.finalize().into()))
}

/// Copy one member out (the daemon extracts `payload.pkg` from its own staged copy).
pub fn extract(path: &Path, m: &Member, dest: &Path) -> std::io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(m.offset))?;
    let mut out = std::fs::File::create(dest)?;
    let n = std::io::copy(&mut f.take(m.size), &mut out)?;
    if n != m.size {
        return Err(std::io::Error::other("short member"));
    }
    out.sync_all()
}

pub fn hex<const N: usize>(b: [u8; N]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    hex(sha2::Sha256::digest(data).into())
}

/// `SHA256` text for `(member, bytes)`: BSD `sha256` lines sorted by member name.
pub fn manifest(members: &[(&str, &[u8])]) -> String {
    let mut lines: Vec<String> = members
        .iter()
        .map(|(n, b)| format!("SHA256 ({n}) = {}\n", sha256_hex(b)))
        .collect();
    lines.sort();
    lines.concat()
}

fn parse_manifest(text: &str) -> Option<Vec<(&str, &str)>> {
    text.lines()
        .map(|l| {
            let rest = l.strip_prefix("SHA256 (")?;
            let (name, hash) = rest.split_once(") = ")?;
            Some((name, hash))
        })
        .collect()
}

/// Check order, names, hashes, signature and Info.json. `keys`: trusted Ed25519 public keys.
pub fn verify(path: &Path, keys: &[[u8; 32]]) -> Result<Verified, String> {
    let ms = members(path)?;
    // Exactly the contract members, each once, in contract order.
    let mut want = MEMBERS.iter().peekable();
    for m in &ms {
        loop {
            match want.next() {
                Some((n, _)) if *n == m.name => break,
                Some((_, false)) => continue,
                _ => return Err(format!("unexpected member {}", m.name)),
            }
        }
    }
    let get = |n: &str| ms.iter().find(|m| m.name == n);
    for (n, required) in MEMBERS {
        if required && get(n).is_none() {
            return Err(format!("missing {n}"));
        }
    }
    let man_m = get(MANIFEST).ok_or("missing manifest")?;
    let man = read_member(path, man_m, MAX_SMALL)?;
    let text = std::str::from_utf8(&man).map_err(|_| "manifest not UTF-8")?;
    let lines = parse_manifest(text).ok_or("bad manifest line")?;
    let hashed: Vec<&Member> = ms
        .iter()
        .filter(|m| m.name != MANIFEST && m.name != SIGNATURE)
        .collect();
    if lines.len() != hashed.len() {
        return Err("manifest does not match the members".into());
    }
    for m in &hashed {
        let Some((_, h)) = lines.iter().find(|(n, _)| *n == m.name) else {
            return Err(format!("{} not in manifest", m.name));
        };
        if *h != hash_member(path, m)? {
            return Err(format!("{}: hash mismatch", m.name));
        }
    }
    let trust = match get(SIGNATURE) {
        None => Trust::Unsigned,
        Some(s) => {
            let sig = read_member(path, s, 64)?;
            let sig: [u8; 64] = sig.try_into().map_err(|_| "bad signature size")?;
            let i = keys
                .iter()
                .position(|k| verify_sig(k, &man, &sig))
                .ok_or("signature not from a trusted key")?;
            Trust::Signed(i)
        }
    };
    let info_m = get(INFO_FILE).ok_or("missing Info.json")?;
    let info = read_member(path, info_m, MAX_SMALL)?;
    let info = Info::parse(std::str::from_utf8(&info).map_err(|_| "Info.json not UTF-8")?)?;
    if !is_app_id(&info.id) || info.package.is_empty() || !is_version(&info.version) {
        return Err("Info.json needs an app id, a package and a pkg version".into());
    }
    Ok(Verified {
        info,
        trust,
        payload: get(PAYLOAD).cloned().ok_or("missing payload")?,
        icon: get(shared::package::ICON).cloned(),
    })
}

fn verify_sig(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    ed25519_dalek::VerifyingKey::from_bytes(key).is_ok_and(|k| {
        k.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(sig))
            .is_ok()
    })
}

/// Ed25519 signature of `msg` with the private key `seed` (producers and tests).
pub fn sign(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(seed)
        .sign(msg)
        .to_bytes()
}

pub fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let s: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let body = s.strip_suffix(b"==").or(s.strip_suffix(b"=")).unwrap_or(&s);
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for c in body {
        acc = acc << 6 | u32::from(val(*c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// SPKI DER prefix of an Ed25519 public key (OID 1.3.101.112).
const SPKI_ED25519: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Raw key from `openssl pkey -pubout` PEM.
pub fn public_key(pem: &str) -> Option<[u8; 32]> {
    let body = pem
        .split("-----BEGIN PUBLIC KEY-----")
        .nth(1)?
        .split("-----END PUBLIC KEY-----")
        .next()?;
    let der = base64(body)?;
    let key = der.strip_prefix(&SPKI_ED25519[..])?;
    key.try_into().ok()
}

/// PEM SPKI text of a raw key (what `openssl pkey -pubout` prints).
pub fn public_pem(key: &[u8; 32]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let der = [&SPKI_ED25519[..], key].concat();
    let mut b64 = String::new();
    for c in der.chunks(3) {
        let n = c.iter().fold(0u32, |a, x| a << 8 | u32::from(*x)) << (8 * (3 - c.len()));
        for i in 0..4 {
            b64.push(if i <= c.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    format!("-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----\n")
}

/// SHA-256 of a whole file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
    Ok(hex(h.finalize().into()))
}

/// Every `*.pub` in `dir` that is an Ed25519 key, sorted by file name.
pub fn read_keys(dir: &Path) -> Vec<[u8; 32]> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pub"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| public_key(&std::fs::read_to_string(p).ok()?))
        .collect()
}

/// ustar bytes of plain files (mode 0644, root, mtime 0): `.wol` producers and tests.
pub fn ustar(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in members {
        let mut h = [0u8; BLOCK as usize];
        h[..name.len()].copy_from_slice(name.as_bytes());
        for (at, v) in [(100, "0000644"), (108, "0000000"), (116, "0000000")] {
            h[at..at + 7].copy_from_slice(v.as_bytes());
        }
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(BLOCK as usize) * BLOCK as usize, 0);
    }
    out.resize(out.len() + 2 * BLOCK as usize, 0);
    out
}

/// Steps after verification: dependencies from FreeBSD pkg (marked automatic, so
/// `pkg autoremove` drops them with the app), then the payload. Another installed version is
/// deleted right before the add, after the dependencies are in place.
pub fn install_steps(
    payload: &str,
    package: &str,
    version: &str,
    deps: &[String],
    installed: Option<&str>,
) -> Result<Vec<Vec<String>>, String> {
    if let Some(d) = deps.iter().find(|d| !shared::package::is_pkg_name(d)) {
        return Err(format!("bad dependency {d}"));
    }
    if installed == Some(version) {
        return Ok(vec![]);
    }
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
    let mut steps = Vec::new();
    if !deps.is_empty() {
        let mut i = v(&["pkg", "install", "-y", "-A"]);
        i.extend(deps.iter().cloned());
        steps.push(i);
    }
    if installed.is_some() {
        steps.push(v(&["pkg", "delete", "-y", package]));
    }
    steps.push(v(&["pkg", "add", payload]));
    Ok(steps)
}

/// Remove an installed app from its install record text: the package, then dependencies
/// nothing else needs.
pub fn remove_steps(id: &str, record: &str) -> Result<Vec<Vec<String>>, String> {
    let i = Info::parse(record)?;
    if i.id != id || i.package.is_empty() {
        return Err(format!("{id}: install record does not match"));
    }
    Ok(vec![
        ["pkg", "delete", "-y", &i.package]
            .map(String::from)
            .to_vec(),
        ["pkg", "autoremove", "-y"].map(String::from).to_vec(),
    ])
}

/// `pkg query -F <file> '%n\t%v'` of the payload.
pub fn query_name_version_argv(payload: &str) -> Vec<String> {
    ["pkg", "query", "-F", payload, "%n\t%v"]
        .map(String::from)
        .to_vec()
}

/// `pkg query -F <file> '%dn'`: the payload's dependency names.
pub fn query_deps_argv(payload: &str) -> Vec<String> {
    ["pkg", "query", "-F", payload, "%dn"]
        .map(String::from)
        .to_vec()
}

/// `pkg query %v <name>`: installed version (fails when not installed).
pub fn query_installed_argv(package: &str) -> Vec<String> {
    ["pkg", "query", "%v", package].map(String::from).to_vec()
}

/// One `name<TAB>version` line.
pub fn parse_name_version(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let (n, v) = lines.next()?.split_once('\t')?;
    (lines.next().is_none() && shared::package::is_pkg_name(n) && is_version(v))
        .then(|| (n.to_owned(), v.to_owned()))
}

/// One pkg name per line; None if any line is not one.
pub fn parse_deps(text: &str) -> Option<Vec<String>> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| shared::package::is_pkg_name(l).then(|| l.to_owned()))
        .collect()
}

/// Install records `<dir>/<id>/Info.json` (the directory name must be the id), sorted by id.
pub fn installed(dir: &Path) -> Vec<Info> {
    let mut v: Vec<Info> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path().join(INFO_FILE)).ok()?;
            let i = Info::parse(&text).ok()?;
            (e.file_name().to_str() == Some(i.id.as_str()) && !i.package.is_empty()).then_some(i)
        })
        .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// Case-insensitive match on name, summary, package and id.
pub fn search<'a>(index: &'a Index, term: &str) -> Vec<&'a Entry> {
    let t = term.trim().to_lowercase();
    if t.is_empty() {
        return Vec::new();
    }
    index
        .packages
        .iter()
        .filter(|e| {
            [&e.name, &e.summary, &e.package, &e.id]
                .iter()
                .any(|f| f.to_lowercase().contains(&t))
        })
        .collect()
}

/// By package name or app id (case-insensitive id).
pub fn find<'a>(index: &'a Index, key: &str) -> Option<&'a Entry> {
    index
        .packages
        .iter()
        .find(|e| e.package == key || e.id.eq_ignore_ascii_case(key))
}

/// Installed apps whose index version differs (the index is the latest).
pub fn updates<'a>(installed: &'a [Info], index: &'a Index) -> Vec<(&'a Info, &'a Entry)> {
    installed
        .iter()
        .filter_map(|i| {
            let e = index.packages.iter().find(|e| e.id == i.id)?;
            (e.version != i.version).then_some((i, e))
        })
        .collect()
}

/// `index.json` with its optional `index.json.sig`: a signature must come from a trusted key.
pub fn verify_index(text: &[u8], sig: Option<&[u8]>, keys: &[[u8; 32]]) -> Result<Trust, String> {
    let Some(sig) = sig else {
        return Ok(Trust::Unsigned);
    };
    let sig: [u8; 64] = sig.try_into().map_err(|_| "bad index signature size")?;
    keys.iter()
        .position(|k| verify_sig(k, text, &sig))
        .map(Trust::Signed)
        .ok_or_else(|| "index signature not from a trusted key".into())
}

/// FreeBSD fetch(1) into `dest`.
pub fn fetch_argv(url: &str, dest: &str) -> Vec<String> {
    ["fetch", "-q", "-o", dest, url].map(String::from).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::package::{ICON, MANIFEST, PAYLOAD, SIGNATURE};

    const SEED: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [9; 32];
    const INFO: &str = r#"{"id":"org.wolos.Fixture","version":"1.0","name":"Fixture","package":"wol-fixture","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/fixture"}]}"#;

    fn dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "wol-package-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn file(members: &[(&str, &[u8])]) -> std::path::PathBuf {
        let p = dir().join("t.wol");
        std::fs::write(&p, ustar(members)).unwrap();
        p
    }

    /// A well-formed package, optionally signed with `seed`.
    fn good(seed: Option<&[u8; 32]>) -> Vec<(String, Vec<u8>)> {
        let mut m: Vec<(String, Vec<u8>)> = vec![
            ("Info.json".into(), INFO.into()),
            (ICON.into(), b"\x89PNG".to_vec()),
            (PAYLOAD.into(), b"pkg payload bytes".to_vec()),
        ];
        let refs: Vec<(&str, &[u8])> = m.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let man = manifest(&refs);
        if let Some(s) = seed {
            let sig = sign(s, man.as_bytes()).to_vec();
            m.push((MANIFEST.into(), man.into_bytes()));
            m.push((SIGNATURE.into(), sig));
        } else {
            m.push((MANIFEST.into(), man.into_bytes()));
        }
        m
    }

    fn write(m: &[(String, Vec<u8>)]) -> std::path::PathBuf {
        let refs: Vec<(&str, &[u8])> = m.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        file(&refs)
    }

    #[test]
    fn signed_unsigned_and_untrusted() {
        let keys = [public_of(&OTHER), public_of(&SEED)];
        let v = verify(&write(&good(Some(&SEED))), &keys).unwrap();
        assert_eq!(v.trust, Trust::Signed(1));
        assert_eq!(
            (v.info.package.as_str(), v.info.version.as_str()),
            ("wol-fixture", "1.0")
        );
        assert!(v.icon.is_some());
        let v = verify(&write(&good(None)), &keys).unwrap();
        assert_eq!(v.trust, Trust::Unsigned);
        // Signed, but by a key this machine does not trust: refused, never "unsigned".
        let e = verify(&write(&good(Some(&SEED))), &[public_of(&OTHER)]).unwrap_err();
        assert!(e.contains("signature"), "{e}");
        assert!(verify(&write(&good(Some(&SEED))), &[]).is_err());
    }

    #[test]
    fn tampering_is_refused() {
        type Edit = fn(&mut Vec<(String, Vec<u8>)>);
        let cases: [(&str, Edit); 10] = [
            ("payload byte", |m| m[2].1[0] ^= 1),
            ("info byte", |m| m[0].1[2] ^= 1),
            ("signature byte", |m| m[4].1[0] ^= 1),
            ("short signature", |m| m[4].1.truncate(63)),
            ("manifest line", |m| {
                m[3].1.extend_from_slice(b"SHA256 (x) = 00\n")
            }),
            ("extra member", |m| {
                m.insert(3, ("evil.sh".into(), b"x".to_vec()))
            }),
            ("duplicate", |m| m.insert(1, m[0].clone())),
            ("wrong order", |m| m.swap(0, 2)),
            ("no payload", |m| {
                m.remove(2);
            }),
            ("path member", |m| m[1].0 = "../icon.png".into()),
        ];
        for (what, edit) in cases {
            let mut m = good(Some(&SEED));
            edit(&mut m);
            assert!(verify(&write(&m), &[public_of(&SEED)]).is_err(), "{what}");
        }
    }

    #[test]
    fn info_must_describe_a_package() {
        for info in [
            r#"{"id":"org.wolos.Fixture","version":"1.0","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/x"}]}"#,
            r#"{"id":"Fixture","version":"1.0","package":"x","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/x"}]}"#,
            r#"{"id":"org.wolos.Fixture","version":"1 0","package":"x","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/x"}]}"#,
            r#"{"id":"org.wolos.Fixture","version":"1.0","package":"x","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"/bin/x"}]}"#,
        ] {
            let mut m = good(None);
            m[0].1 = info.as_bytes().to_vec();
            let refs: Vec<(&str, &[u8])> = m[..3]
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect();
            m[3].1 = manifest(&refs).into_bytes();
            assert!(verify(&write(&m), &[]).is_err(), "{info}");
        }
    }

    #[test]
    fn archive_shape_is_checked() {
        let p = write(&good(None));
        let bytes = std::fs::read(&p).unwrap();
        // Truncated mid-member.
        std::fs::write(&p, &bytes[..bytes.len() / 2]).unwrap();
        assert!(verify(&p, &[]).is_err());
        // Symlink member (typeflag '2').
        let mut b = bytes.clone();
        b[156] = b'2';
        let sum: u32 = b[..512]
            .iter()
            .enumerate()
            .map(|(i, x)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u32::from(*x)
                }
            })
            .sum();
        b[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        std::fs::write(&p, &b).unwrap();
        assert!(verify(&p, &[]).is_err());
        // Bad header checksum.
        let mut b = bytes;
        b[0] ^= 1;
        std::fs::write(&p, &b).unwrap();
        assert!(verify(&p, &[]).is_err());
        assert!(verify(&p.with_extension("missing"), &[]).is_err());
    }

    #[test]
    fn extract_payload() {
        let p = write(&good(Some(&SEED)));
        let v = verify(&p, &[public_of(&SEED)]).unwrap();
        let out = p.with_file_name("payload.pkg");
        extract(&p, &v.payload, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"pkg payload bytes");
        assert_eq!(
            read_member(&p, v.icon.as_ref().unwrap(), 16).unwrap(),
            b"\x89PNG"
        );
    }

    #[test]
    fn public_keys_from_pem() {
        // `openssl pkey -pubout` of an Ed25519 key (RFC 8032 test 1 public key).
        let pem = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n";
        assert_eq!(
            public_key(pem).map(hex),
            Some("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into())
        );
        for bad in [
            "",
            "-----BEGIN PUBLIC KEY-----\n!!!!\n-----END PUBLIC KEY-----\n",
            // RSA-sized blob.
            "-----BEGIN PUBLIC KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf\n-----END PUBLIC KEY-----\n",
            // X25519 key (OID 1.3.101.110), same length.
            "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VuAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n",
        ] {
            assert_eq!(public_key(bad), None, "{bad}");
        }
        let d = dir();
        std::fs::write(d.join("wolos.pub"), pem).unwrap();
        std::fs::write(d.join("notes.txt"), pem).unwrap();
        std::fs::write(d.join("bad.pub"), "x").unwrap();
        assert_eq!(read_keys(&d).len(), 1);
    }

    #[test]
    fn public_pem_roundtrip_and_file_hash() {
        let k = public_of(&SEED);
        let pem = public_pem(&k);
        assert!(
            pem.starts_with("-----BEGIN PUBLIC KEY-----\n")
                && pem.ends_with("-----END PUBLIC KEY-----\n")
        );
        assert_eq!(public_key(&pem), Some(k));
        let p = dir().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), sha256_hex(b"abc"));
        assert!(sha256_file(&p.with_extension("x")).is_err());
    }

    #[test]
    fn sha256_known_answer() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn install_steps_table() {
        let deps = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let show = |r: Result<Vec<Vec<String>>, String>| match r {
            Ok(steps) => steps
                .iter()
                .map(|s| s.join(" "))
                .collect::<Vec<_>>()
                .join("; "),
            Err(e) => format!("error: {e}"),
        };
        let p = "/var/cache/wolos/install/1/payload.pkg";
        let cases: [(&[&str], Option<&str>, &str); 5] = [
            (
                &["gtk4", "libadwaita"],
                None,
                "pkg install -y -A gtk4 libadwaita; pkg add /var/cache/wolos/install/1/payload.pkg",
            ),
            (&[], None, "pkg add /var/cache/wolos/install/1/payload.pkg"),
            // Same version installed: nothing to do.
            (&["gtk4"], Some("3.1.0"), ""),
            // Another version: replace it (deps first, so a failure leaves the old app in place).
            (
                &["gtk4"],
                Some("3.0.0"),
                "pkg install -y -A gtk4; pkg delete -y raider; pkg add /var/cache/wolos/install/1/payload.pkg",
            ),
            (&["-f"], None, "error: bad dependency -f"),
        ];
        for (d, installed, want) in cases {
            assert_eq!(
                show(install_steps(p, "raider", "3.1.0", &deps(d), installed)),
                want,
                "{d:?} {installed:?}"
            );
        }
    }

    #[test]
    fn remove_steps_from_the_record() {
        let rec = |pkg: &str| INFO.replace("wol-fixture", pkg);
        assert_eq!(
            remove_steps("org.wolos.Fixture", &rec("wol-fixture")).unwrap(),
            [
                vec!["pkg", "delete", "-y", "wol-fixture"],
                vec!["pkg", "autoremove", "-y"]
            ]
        );
        // The record must be the app asked for and name a package.
        assert!(remove_steps("org.wolos.Other", &rec("wol-fixture")).is_err());
        assert!(
            remove_steps(
                "org.wolos.Fixture",
                &INFO.replace(r#""package":"wol-fixture","#, "")
            )
            .is_err()
        );
        assert!(remove_steps("org.wolos.Fixture", "{").is_err());
    }

    #[test]
    fn pkg_query_output() {
        assert_eq!(
            parse_name_version("raider\t3.1.0\n"),
            Some(("raider".into(), "3.1.0".into()))
        );
        for bad in ["", "raider", "raider\t", "-x\t1", "a\t1 2", "a\t1\nb\t2\n"] {
            assert_eq!(parse_name_version(bad), None, "{bad:?}");
        }
        assert_eq!(
            parse_deps("gtk4\nlibadwaita\n\n"),
            Some(vec!["gtk4".into(), "libadwaita".into()])
        );
        assert_eq!(parse_deps(""), Some(vec![]));
        assert_eq!(parse_deps("gtk4\n-f\n"), None);
    }

    #[test]
    fn installed_records() {
        let d = dir();
        let rec = |id: &str, text: &str| {
            std::fs::create_dir_all(d.join(id)).unwrap();
            std::fs::write(d.join(id).join("Info.json"), text).unwrap();
        };
        rec("org.wolos.Fixture", INFO);
        // Directory name must be the id; broken records are skipped.
        rec("org.wolos.Other", INFO);
        rec("org.wolos.Broken", "{");
        let got = installed(&d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "org.wolos.Fixture");
        assert!(installed(&d.join("missing")).is_empty());
    }

    fn index() -> shared::package::Index {
        let e = |id: &str, name: &str, package: &str, version: &str, summary: &str| {
            shared::package::Entry {
                id: id.into(),
                name: name.into(),
                summary: summary.into(),
                version: version.into(),
                package: package.into(),
                abi: "FreeBSD:15:amd64".into(),
                url: format!("https://example.org/{package}-{version}.wol"),
                sha256: "0".repeat(64),
                size: 1,
                ..Default::default()
            }
        };
        shared::package::Index {
            format: 1,
            packages: vec![
                e(
                    "io.github.adbeveridge.Raider",
                    "File Shredder",
                    "raider",
                    "3.1.0",
                    "Securely delete your files",
                ),
                e(
                    "com.rafaelmardojai.Blanket",
                    "Blanket",
                    "blanket",
                    "0.8.0",
                    "Listen to ambient sounds",
                ),
                e(
                    "org.wolos.Fixture",
                    "Fixture",
                    "wol-fixture",
                    "2.0",
                    "Test package",
                ),
            ],
        }
    }

    #[test]
    fn index_search_find_and_updates() {
        let i = index();
        let ids = |v: Vec<&shared::package::Entry>| {
            v.iter().map(|e| e.package.clone()).collect::<Vec<_>>()
        };
        assert_eq!(ids(search(&i, "SHRED")), ["raider"]);
        assert_eq!(ids(search(&i, "sounds")), ["blanket"]);
        assert!(search(&i, " ").is_empty());
        for key in [
            "raider",
            "io.github.adbeveridge.Raider",
            "io.github.adbeveridge.raider",
        ] {
            assert_eq!(
                find(&i, key).map(|e| e.package.as_str()),
                Some("raider"),
                "{key}"
            );
        }
        assert_eq!(find(&i, "nope"), None);
        let mut old = Info::parse(INFO).unwrap();
        let current = old.clone();
        old.version = "1.0".into();
        let olds = [old];
        let up = updates(&olds, &i);
        assert_eq!(up.len(), 1);
        assert_eq!(
            (up[0].0.version.as_str(), up[0].1.version.as_str()),
            ("1.0", "2.0")
        );
        let mut same = current;
        same.version = "2.0".into();
        assert!(updates(&[same], &i).is_empty());
    }

    #[test]
    fn index_signature() {
        let text = b"{\"format\":1,\"packages\":[]}";
        let sig = sign(&SEED, text);
        let keys = [public_of(&SEED)];
        assert_eq!(verify_index(text, Some(&sig), &keys), Ok(Trust::Signed(0)));
        assert_eq!(verify_index(text, None, &keys), Ok(Trust::Unsigned));
        assert!(verify_index(b"{}", Some(&sig), &keys).is_err());
        assert!(verify_index(text, Some(&sig[..10]), &keys).is_err());
        assert!(verify_index(text, Some(&sig), &[public_of(&OTHER)]).is_err());
    }
}
