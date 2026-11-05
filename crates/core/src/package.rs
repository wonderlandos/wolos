// SPDX-License-Identifier: BSD-2-Clause
//! `.wol` package files (ADR 0011, docs/modules/packages.md): read and verify the ustar
//! archive (hashes, Ed25519 signature, Info.json), extract the payload (a pacman package), plan the `wol pkg -- -U` install (the sealed-system pacman).
use shared::bundle::{INFO_FILE, Info};
use shared::package::{
    Entry, Index, MANIFEST, MAX_SCREENSHOT_SIZE, MAX_SCREENSHOTS, MAX_SIZE, MEMBERS, PAYLOAD,
    SIGNATURE, is_app_id, is_screenshot_member, is_version,
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
    /// The listing screenshots (`screenshots/<name>.png|webp`), in archive order.
    pub screenshots: Vec<Member>,
}

/// Where the screenshots sit in [`MEMBERS`] order: in front of `payload.pkg` (after `icon.png`).
const SHOT_SLOT: usize = 2;

/// A member's place in contract order; every screenshot shares [`SHOT_SLOT`], the members from the
/// payload on move up by one.
fn slot(name: &str) -> Option<usize> {
    if is_screenshot_member(name) {
        return Some(SHOT_SLOT);
    }
    MEMBERS
        .iter()
        .position(|(n, _)| *n == name)
        .map(|i| if i >= SHOT_SLOT { i + 1 } else { i })
}

/// A PNG or WebP by its first bytes (the screenshot's name says which).
fn picture_matches_name(head: &[u8], name: &str) -> bool {
    if name.ends_with(".png") {
        head.starts_with(b"\x89PNG\r\n\x1a\n")
    } else {
        head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP"
    }
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
        let Some(m) = header(&h, pos, len)? else {
            return Ok(out);
        };
        pos = m.offset + m.size.div_ceil(BLOCK) * BLOCK;
        out.push(m);
    }
}

/// Why a member other than a plain file is refused.
const ONLY_PLAIN: &str = "only plain files are allowed";

/// One ustar header block at `pos` of an archive `len` bytes long: the member, or None for the
/// end-of-archive block. Only plain files without a name prefix pass.
fn header(h: &[u8; BLOCK as usize], pos: u64, len: u64) -> Result<Option<Member>, String> {
    if h.iter().all(|b| *b == 0) {
        return Ok(None);
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
        return Err(ONLY_PLAIN.into());
    }
    let name = std::str::from_utf8(field(&h[..100]))
        .map_err(|_| "member name not UTF-8".to_string())?
        .to_owned();
    let size = octal(&h[124..136]).ok_or("bad member size")?;
    let offset = pos + BLOCK;
    if offset.checked_add(size).is_none_or(|end| end > len) {
        return Err("truncated archive".into());
    }
    Ok(Some(Member { name, offset, size }))
}

/// Regular-file members of a ustar archive held in memory (a decompressed `.wolplug`), with the
/// same rules as [`members`].
pub fn members_in(tar: &[u8]) -> Result<Vec<Member>, String> {
    let len = tar.len() as u64;
    let mut out = Vec::new();
    let mut pos = 0u64;
    loop {
        let Some(block) = tar.get(pos as usize..(pos + BLOCK) as usize) else {
            return Err("truncated archive".into());
        };
        let h: &[u8; BLOCK as usize] = block.try_into().map_err(|_| "truncated archive")?;
        let Some(m) = header(h, pos, len)? else {
            return Ok(out);
        };
        pos = m.offset + m.size.div_ceil(BLOCK) * BLOCK;
        out.push(m);
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
    // Exactly the contract members, each once, in contract order. Between the icon and the payload
    // may sit the hub's listing screenshots: up to MAX_SCREENSHOTS, each name once, each a PNG or
    // WebP (by name and first bytes) of at most MAX_SCREENSHOT_SIZE.
    let mut last: Option<usize> = None;
    let mut shots: Vec<Member> = Vec::new();
    for m in &ms {
        let at = slot(&m.name).ok_or_else(|| format!("unexpected member {}", m.name))?;
        if last.is_some_and(|l| at < l || (at == l && at != SHOT_SLOT)) {
            return Err(format!("unexpected member {}", m.name));
        }
        if at == SHOT_SLOT {
            if shots.len() >= MAX_SCREENSHOTS || shots.iter().any(|s| s.name == m.name) {
                return Err(format!("unexpected member {}", m.name));
            }
            if m.size > MAX_SCREENSHOT_SIZE {
                return Err(format!("{} too large", m.name));
            }
            shots.push(m.clone());
        }
        last = Some(at);
    }
    for s in &shots {
        let head = Member {
            size: s.size.min(12),
            ..s.clone()
        };
        if !picture_matches_name(&read_member(path, &head, 12)?, &s.name) {
            return Err(format!("{} is not a PNG or WebP picture", s.name));
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
        screenshots: shots,
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

// ---- `.wolplug` (ADR 0031 W4, docs/modules/plugins.md "Distribution") ----

/// Largest `.wolplug` file read (zstd-compressed): a full plugin folder compresses well below it.
pub const WOLPLUG_MAX_FILE: u64 = 2 << 20;
/// Largest decompressed `.wolplug` tar: the folder cap ([`shared::plugins::MAX_TOTAL_SIZE`]) plus
/// a header block per file and the hash list. The decompressor stops at this many bytes.
pub const WOLPLUG_MAX_TAR: u64 = 4 << 20;
/// Signature member: the raw Ed25519 signature of the `SHA256` member, by the index key.
pub const WOLPLUG_SIG: &str = "SHA256.sig";
/// Files in one plugin.
const WOLPLUG_MAX_FILES: usize = 256;

/// A `.wolplug` that passed every check, ready to be written as `<plugins dir>/<id>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPlugin {
    pub manifest: shared::plugins::Manifest,
    pub trust: Trust,
    /// (path relative to the plugin folder, bytes), in archive order.
    pub files: Vec<(String, Vec<u8>)>,
}

/// Checks a decompressed `.wolplug` in this order, stopping at the first failure: size, archive
/// shape (plain files `<id>/<path>`, then `SHA256`, then optionally `SHA256.sig`), the signature
/// over `SHA256` (before anything inside is read as text), every hash, then the folder rules and
/// `plugin.toml` through [`shared::plugins::parse`]. A signature from an unknown key is refused;
/// no signature gives [`Trust::Unsigned`] (the caller decides, see [`require_signed`]).
pub fn verify_wolplug(tar: &[u8], keys: &[[u8; 32]]) -> Result<VerifiedPlugin, String> {
    use crate::plugins::{MAX_FILE, PACK_MANIFEST};
    use shared::plugins::{MANIFEST_FILE, MAX_TOTAL_SIZE, is_plugin_id, is_plugin_path};
    if tar.len() as u64 > WOLPLUG_MAX_TAR {
        return Err("The plugin file is too large.".into());
    }
    let ms = members_in(tar).map_err(|e| {
        // QA-PLUG-20: a hand-made tar (folder entries) is not damaged, it is not a package.
        if e == ONLY_PLAIN {
            "The plugin file has folders or links in it, so it was not made by wol plugin pack. Make it with wol plugin pack.".to_owned()
        } else {
            format!("The plugin file is damaged ({e}).")
        }
    })?;
    let bytes = |m: &Member| &tar[m.offset as usize..(m.offset + m.size) as usize];
    let names: Vec<&str> = ms.iter().map(|m| m.name.as_str()).collect();
    let (man_at, sig) = match names[..] {
        [.., PACK_MANIFEST, WOLPLUG_SIG] => (ms.len() - 2, Some(&ms[ms.len() - 1])),
        [.., PACK_MANIFEST] => (ms.len() - 1, None),
        _ => return Err("The plugin file has no SHA256 list at its end.".into()),
    };
    let files = &ms[..man_at];
    let man = bytes(&ms[man_at]);
    if man.len() as u64 > MAX_SMALL {
        return Err("The plugin's SHA256 list is too large.".into());
    }
    // The signature first: nothing inside is trusted, or parsed, before it holds.
    let trust = match sig {
        None => Trust::Unsigned,
        Some(s) => {
            let sig: [u8; 64] = bytes(s)
                .try_into()
                .map_err(|_| "The plugin's signature is damaged.")?;
            let i = keys
                .iter()
                .position(|k| verify_sig(k, man, &sig))
                .ok_or("The plugin is not signed by a trusted key.")?;
            Trust::Signed(i)
        }
    };
    let text = std::str::from_utf8(man).map_err(|_| "The plugin's SHA256 list is damaged.")?;
    let lines = parse_manifest(text).ok_or("The plugin's SHA256 list is damaged.")?;
    if files.is_empty() || files.len() > WOLPLUG_MAX_FILES || lines.len() != files.len() {
        return Err("The plugin's SHA256 list does not match its files.".into());
    }
    for m in files {
        let mut found = lines.iter().filter(|(n, _)| *n == m.name);
        let (Some((_, h)), None) = (found.next(), found.next()) else {
            return Err(format!("{} does not match the SHA256 list.", m.name));
        };
        if *h != sha256_hex(bytes(m)) {
            return Err(format!("{} does not match the SHA256 list.", m.name));
        }
    }
    // Hashes hold: now the folder rules.
    let id = files[0].name.split('/').next().unwrap_or_default();
    if !is_plugin_id(id) {
        return Err("The plugin file does not hold one plugin folder.".into());
    }
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for m in files {
        let rel = m
            .name
            .strip_prefix(id)
            .and_then(|r| r.strip_prefix('/'))
            .filter(|r| is_plugin_path(r))
            .ok_or_else(|| format!("{} is not a file inside the plugin folder.", m.name))?;
        if out.iter().any(|(r, _)| r == rel) {
            return Err(format!("{} is in the plugin file twice.", m.name));
        }
        if m.size > MAX_FILE {
            return Err(format!("{rel} is larger than {} KB.", MAX_FILE / 1024));
        }
        total += m.size;
        out.push((rel.to_owned(), bytes(m).to_vec()));
    }
    if total > MAX_TOTAL_SIZE {
        return Err("The plugin is larger than 1 MB.".into());
    }
    let toml = out
        .iter()
        .find(|(r, _)| r == MANIFEST_FILE)
        .ok_or_else(|| format!("The plugin has no {MANIFEST_FILE}."))?;
    let toml = std::str::from_utf8(&toml.1).map_err(|_| format!("{MANIFEST_FILE} is not text."))?;
    let manifest = shared::plugins::parse(toml)?;
    if manifest.id != id {
        return Err(format!(
            "The folder is {id} but {MANIFEST_FILE} says {}.",
            manifest.id
        ));
    }
    if manifest.is_reserved() {
        return Err("Plugin ids starting with org.wolos. are kept for WolOS.".into());
    }
    if let Some(f) = manifest
        .files()
        .into_iter()
        .find(|f| !out.iter().any(|(r, _)| r == f))
    {
        return Err(format!(
            "{MANIFEST_FILE} names {f}, which is not in the plugin."
        ));
    }
    Ok(VerifiedPlugin {
        manifest,
        trust,
        files: out,
    })
}

/// Plugins from the index must carry the index key's signature (the index CI signs each one).
pub fn require_signed(v: VerifiedPlugin) -> Result<VerifiedPlugin, String> {
    match v.trust {
        Trust::Signed(_) => Ok(v),
        Trust::Unsigned => Err("The plugin is not signed by the WolOS plugin index.".into()),
    }
}

/// Removes a staging folder; one that stays is found and removed by the next install.
fn discard(stage: &Path) {
    let _ = std::fs::remove_dir_all(stage);
}

/// Why a plugin could not be written: a full disk is named (QA-STORE-24), anything else carries
/// the system's reason.
fn plugin_write_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(28) {
        "There is no room left on the disk. Free some space, then try again.".into()
    } else {
        format!("The plugin could not be written: {e}.")
    }
}

/// Staging folders (`.stage-<id>-<pid>`) that an install left behind when its process died
/// (QA-STORE-23): removed, and when it died between the two renames (the older version moved
/// to the folder's `old`, the new one not yet in place) the older version goes back. A folder of
/// a process that is still running is left alone.
pub fn recover_stages(plugins_dir: &Path) {
    let Ok(rd) = std::fs::read_dir(plugins_dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((id, pid)) = name
            .strip_prefix(".stage-")
            .and_then(|r| r.rsplit_once('-'))
            .and_then(|(id, pid)| Some((id, pid.parse::<u32>().ok()?)))
        else {
            continue;
        };
        if pid == std::process::id() || Path::new(&format!("/proc/{pid}")).exists() {
            continue;
        }
        let (stage, dest) = (e.path(), plugins_dir.join(id));
        let old = stage.join("old");
        if old.is_dir() && !dest.exists() && !id.is_empty() && !id.contains(['/', '\0']) {
            let _ = std::fs::rename(&old, &dest);
        }
        discard(&stage);
    }
}

/// Writes a verified plugin as `<plugins_dir>/<id>/` for this account: staged next to it, checked
/// with [`crate::plugins::check`], then moved in place of any older version. Returns the folder.
pub fn install_plugin(
    v: &VerifiedPlugin,
    plugins_dir: &Path,
) -> Result<std::path::PathBuf, String> {
    let id = &v.manifest.id;
    let fail = plugin_write_error;
    std::fs::create_dir_all(plugins_dir).map_err(fail)?;
    recover_stages(plugins_dir);
    let stage = plugins_dir.join(format!(".stage-{id}-{}", std::process::id()));
    discard(&stage);
    let staged = stage.join(id);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&staged)?;
        for (rel, b) in &v.files {
            let p = shared::safe_path::safe_join_str(&staged, rel)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&p, b)?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        discard(&stage);
        return Err(fail(e));
    }
    if let Some(p) = crate::plugins::check(&staged, false).problems.first() {
        discard(&stage);
        return Err(p.clone());
    }
    let dest = plugins_dir.join(id);
    let old = stage.join("old");
    let moved_old = dest.exists() && std::fs::rename(&dest, &old).is_ok();
    if let Err(e) = std::fs::rename(&staged, &dest) {
        if moved_old {
            let _ = std::fs::rename(&old, &dest);
        }
        discard(&stage);
        return Err(fail(e));
    }
    discard(&stage);
    Ok(dest)
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

/// The revocation list next to the keys (audit A13): one key per line as 64 lowercase hex
/// characters, a space and the reason; `#` comments.
pub const REVOKED_FILE: &str = "revoked.list";

/// Keys listed in `text` (a [`REVOKED_FILE`]); malformed lines are skipped (an unreadable entry
/// cannot revoke, the contract test refuses one in the shipped file).
pub fn parse_revoked(text: &str) -> Vec<[u8; 32]> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let key = l.split_whitespace().next()?;
            if key.len() != 64 {
                return None;
            }
            let mut out = [0u8; 32];
            for (i, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(key.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            Some(out)
        })
        .collect()
}

/// Every `*.pub` in `dir` that is an Ed25519 key and not in `dir`/[`REVOKED_FILE`], sorted by
/// file name. Empty when the directory is unreadable. Only `dir` itself is read, never a
/// subfolder (keys/[`HUB_KEYS_SUBDIR`] and keys/shards hold keys with a narrower reach).
pub fn read_keys(dir: &Path) -> Vec<[u8; 32]> {
    let revoked = read_revoked(dir);
    pub_keys(dir)
        .into_iter()
        .filter(|k| !revoked.contains(k))
        .collect()
}

/// The subfolder of the keys folder with the Hub's plugin index key(s)
/// (`/usr/share/wolos/keys/hub`): trusted for plugins only ([`Purpose::Plugins`]).
pub const HUB_KEYS_SUBDIR: &str = "hub";

/// What a set of keys is trusted to verify ([`read_keys_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Everything that changes the system: `.wol` packages, the release feed, the update
    /// manifest (`base.sig`), xguard rules. Only the keys folder itself.
    System,
    /// The plugin index and `.wolplug` signatures: the keys folder plus its
    /// [`HUB_KEYS_SUBDIR`] (a release-signed index still verifies).
    Plugins,
}

/// The trusted keys below the keys folder `dir` for `purpose`. [`Purpose::System`] is exactly
/// [`read_keys`]`(dir)`. [`Purpose::Plugins`] adds `dir`/[`HUB_KEYS_SUBDIR`]; a key listed in
/// either folder's [`REVOKED_FILE`] is dropped, and a key in both folders counts once.
pub fn read_keys_for(dir: &Path, purpose: Purpose) -> Vec<[u8; 32]> {
    match purpose {
        Purpose::System => read_keys(dir),
        Purpose::Plugins => {
            let hub = dir.join(HUB_KEYS_SUBDIR);
            let mut revoked = read_revoked(dir);
            revoked.extend(read_revoked(&hub));
            let mut out: Vec<[u8; 32]> = Vec::new();
            for k in pub_keys(dir).into_iter().chain(pub_keys(&hub)) {
                if !revoked.contains(&k) && !out.contains(&k) {
                    out.push(k);
                }
            }
            out
        }
    }
}

fn read_revoked(dir: &Path) -> Vec<[u8; 32]> {
    std::fs::read_to_string(dir.join(REVOKED_FILE))
        .map(|t| parse_revoked(&t))
        .unwrap_or_default()
}

/// The Ed25519 keys of the `*.pub` files directly in `dir`, sorted by file name.
fn pub_keys(dir: &Path) -> Vec<[u8; 32]> {
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

/// Steps after verification: `-U` of the payload (a pacman package) through the sealed-system
/// pacman (`wol pkg --`, like [`remove_steps`]; the bare `pacman` on wol-admind's `PATH` keeps its
/// database on the read-only /usr). pacman resolves its dependencies from the pinned repos and
/// replaces another installed version itself; the same version installed: nothing to do.
pub fn install_steps(
    payload: &str,
    version: &str,
    installed: Option<&str>,
) -> Result<Vec<Vec<String>>, String> {
    if !payload.starts_with('/') {
        return Err(format!("payload {payload} is not an absolute path"));
    }
    if installed == Some(version) {
        return Ok(vec![]);
    }
    Ok(vec![
        ["wol", "pkg", "--", "-U", "--noconfirm", payload]
            .map(String::from)
            .to_vec(),
    ])
}

/// Remove an installed app from its install record text: the package and the dependencies
/// nothing else needs (`-Rns`), through the sealed-system pacman (`wol pkg --`, as the install
/// plans in `store` run it). The bare `pacman` wol-admind finds on its `PATH` keeps its database
/// on the read-only /usr and cannot remove anything.
pub fn remove_steps(id: &str, record: &str) -> Result<Vec<Vec<String>>, String> {
    let i = Info::parse(record)?;
    if i.id != id || i.package.is_empty() {
        return Err(format!("{id}: install record does not match"));
    }
    Ok(vec![
        ["wol", "pkg", "--", "-Rns", "--noconfirm", &i.package]
            .map(String::from)
            .to_vec(),
    ])
}

/// `pacman -Qp <file>`: the payload's `name version`.
pub fn query_name_version_argv(payload: &str) -> Vec<String> {
    ["pacman", "-Qp", payload].map(String::from).to_vec()
}

/// `pacman -Q <name>`: installed `name version` (fails when not installed).
pub fn query_installed_argv(package: &str) -> Vec<String> {
    ["pacman", "-Q", package].map(String::from).to_vec()
}

/// One `name version` line (`pacman -Q` / `-Qp`).
pub fn parse_name_version(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let mut f = lines.next()?.split(' ');
    let (n, v) = (f.next()?, f.next()?);
    (f.next().is_none()
        && lines.next().is_none()
        && shared::package::is_pkg_name(n)
        && is_version(v))
    .then(|| (n.to_owned(), v.to_owned()))
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

/// Written next to the cached index once it verified as signed: from then on a missing
/// `index.json.sig` is an error, never "unsigned" (CORE-8, a stripped signature is a downgrade).
pub const INDEX_SIGNED_MARK: &str = "index.signed";

/// A missing index signature is refused when trusted keys are installed and the index is the
/// official one, or when this cache once held a signed index ([`INDEX_SIGNED_MARK`]). Only a
/// developer index (`WOL_INDEX_URL`) on a cache that never saw a signature may be unsigned.
pub fn index_sig_required(have_keys: bool, developer_url: bool, was_signed: bool) -> bool {
    was_signed || (have_keys && !developer_url)
}

/// [`verify_index`] for a fetched index: when `required` ([`index_sig_required`]) a missing
/// signature is an error instead of [`Trust::Unsigned`].
pub fn verify_fetched_index(
    text: &[u8],
    sig: Option<&[u8]>,
    keys: &[[u8; 32]],
    required: bool,
) -> Result<Trust, String> {
    if sig.is_none() && required {
        return Err("index signature missing".into());
    }
    verify_index(text, sig, keys)
}

/// Most bytes the store index (or its signature) may be (audit 8 #5): curl stops a larger
/// download, and the controller reads no more of what is on disk.
pub const INDEX_MAX_BYTES: u64 = 32 << 20;

/// curl into `dest` (fails on HTTP errors): https only, also after a redirect, nothing larger
/// than [`INDEX_MAX_BYTES`], no `.curlrc`.
pub fn fetch_argv(url: &str, dest: &str) -> Vec<String> {
    let max = INDEX_MAX_BYTES.to_string();
    [
        "curl",
        "--disable",
        // Silent: curl's own text never reaches the person ("curl: (22) The requested URL returned
        // error: 404"; a missing .sig is normal); the caller words its exit status in one sentence
        // (QA-STORE-61, QA-PWR-17).
        "-fsL",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-filesize",
        &max,
        "-o",
        dest,
        "--",
        url,
    ]
    .map(String::from)
    .to_vec()
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

    /// A PNG-looking picture of `n` bytes (the reader checks the first bytes only).
    fn png(n: usize) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.resize(n, 7);
        b
    }

    fn webp() -> Vec<u8> {
        let mut b = b"RIFF\x20\0\0\0WEBPVP8L".to_vec();
        b.resize(40, 3);
        b
    }

    /// `good()` with the given screenshots between the icon and the payload, manifest and signature redone.
    fn with_shots(shots: &[(&str, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
        let mut m: Vec<(String, Vec<u8>)> = vec![
            ("Info.json".into(), INFO.into()),
            (ICON.into(), b"\x89PNG".to_vec()),
        ];
        m.extend(shots.iter().map(|(n, b)| ((*n).into(), b.clone())));
        m.push((PAYLOAD.into(), b"pkg payload bytes".to_vec()));
        let refs: Vec<(&str, &[u8])> = m.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let man = manifest(&refs);
        let sig = sign(&SEED, man.as_bytes()).to_vec();
        m.push((MANIFEST.into(), man.into_bytes()));
        m.push((SIGNATURE.into(), sig));
        m
    }

    /// The hub's listing pictures (hubcheck `facts`) are part of the contract: `screenshots/<name>.png|webp`
    /// between the icon and the payload, up to eight, each a real picture of at most 2 MiB.
    #[test]
    fn screenshots_are_accepted_and_bounded() {
        let keys = [public_of(&SEED)];
        let ok = |m: &[(String, Vec<u8>)]| verify(&write(m), &keys);
        let v = ok(&with_shots(&[
            ("screenshots/main.png", png(100)),
            ("screenshots/two_2.webp", webp()),
        ]))
        .unwrap();
        assert_eq!(v.trust, Trust::Signed(0));
        assert_eq!(
            v.screenshots
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            ["screenshots/main.png", "screenshots/two_2.webp"]
        );
        assert!(v.icon.is_some());
        // none at all is still a package
        assert!(ok(&with_shots(&[])).unwrap().screenshots.is_empty());
        // eight are the most
        let eight: Vec<(String, Vec<u8>)> = (0..8)
            .map(|i| (format!("screenshots/s{i}.png"), png(64)))
            .collect();
        let refs: Vec<(&str, Vec<u8>)> =
            eight.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
        assert_eq!(ok(&with_shots(&refs)).unwrap().screenshots.len(), 8);
        let nine: Vec<(String, Vec<u8>)> = (0..9)
            .map(|i| (format!("screenshots/s{i}.png"), png(64)))
            .collect();
        let refs: Vec<(&str, Vec<u8>)> =
            nine.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
        assert!(ok(&with_shots(&refs)).is_err(), "nine screenshots");
        // size: 2 MiB is the limit
        assert!(ok(&with_shots(&[("screenshots/big.png", png(2 << 20))])).is_ok());
        let e = ok(&with_shots(&[("screenshots/big.png", png((2 << 20) + 1))])).unwrap_err();
        assert!(e.contains("too large"), "{e}");
        // names: charset, length, extension, directory
        for bad in [
            "screenshots/Main.png",
            "screenshots/a b.png",
            "screenshots/a.gif",
            "screenshots/a.svg",
            "screenshots/.png",
            "screenshots/sub/a.png",
            "screenshots/../a.png",
            "screenshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
            "shots/a.png",
            "screenshots.png",
        ] {
            assert!(ok(&with_shots(&[(bad, png(64))])).is_err(), "{bad}");
        }
        // the same name twice; a picture that is not what its name says
        assert!(
            ok(&with_shots(&[
                ("screenshots/a.png", png(64)),
                ("screenshots/a.png", png(65))
            ]))
            .is_err()
        );
        assert!(
            ok(&with_shots(&[("screenshots/a.png", webp())])).is_err(),
            "webp bytes named .png"
        );
        assert!(
            ok(&with_shots(&[("screenshots/a.webp", png(64))])).is_err(),
            "png bytes named .webp"
        );
        assert!(ok(&with_shots(&[("screenshots/a.png", b"<svg/>".to_vec())])).is_err());
        assert!(
            ok(&with_shots(&[("screenshots/a.png", Vec::new())])).is_err(),
            "empty"
        );
    }

    /// Screenshots keep their slot: not before the icon or Info.json, not after the payload or the manifest,
    /// and a changed byte is still a hash mismatch.
    #[test]
    fn screenshots_have_their_place_in_the_archive() {
        let keys = [public_of(&SEED)];
        let base = with_shots(&[("screenshots/a.png", png(64))]);
        // [Info, icon, shot, payload, SHA256, SHA256.sig]
        assert!(verify(&write(&base), &keys).is_ok());
        for (what, edit) in [
            (
                "shot first",
                (|m: &mut Vec<(String, Vec<u8>)>| m.swap(0, 2)) as fn(&mut Vec<_>),
            ),
            ("shot before Info.json", |m| m.swap(0, 2)),
            ("shot before the icon", |m| m.swap(1, 2)),
            ("shot after the payload", |m| m.swap(2, 3)),
            ("shot after the manifest", |m| m.swap(2, 4)),
            ("shot byte", |m| m[2].1[20] ^= 1),
            ("shot not in the manifest", |m| {
                let t = String::from_utf8(m[4].1.clone()).unwrap();
                let t: String = t
                    .lines()
                    .filter(|l| !l.contains("screenshots/"))
                    .map(|l| format!("{l}\n"))
                    .collect();
                m[4].1 = t.into_bytes();
            }),
        ] {
            let mut m = base.clone();
            edit(&mut m);
            assert!(verify(&write(&m), &keys).is_err(), "{what}");
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
        // A revoked key is never trusted, even with its .pub still present.
        let k = read_keys(&d)[0];
        std::fs::write(
            d.join(REVOKED_FILE),
            format!(
                "# list
{} leaked 2026-11
not-a-key x
",
                hex(k)
            ),
        )
        .unwrap();
        assert!(read_keys(&d).is_empty());
        assert_eq!(
            parse_revoked(
                "zz short
"
            ),
            Vec::<[u8; 32]>::new()
        );
    }

    /// HUB-KEYDIR: `read_keys` reads only `dir` itself, never a subfolder (keys/hub, keys/shards).
    #[test]
    fn read_keys_ignores_subfolders() {
        let d = dir();
        std::fs::create_dir_all(d.join(HUB_KEYS_SUBDIR)).unwrap();
        std::fs::write(
            d.join(HUB_KEYS_SUBDIR).join("hub.pub"),
            public_pem(&public_of(&OTHER)),
        )
        .unwrap();
        // A folder whose name ends in .pub is not a key either.
        std::fs::create_dir_all(d.join("nested.pub")).unwrap();
        std::fs::write(
            d.join("nested.pub").join("x.pub"),
            public_pem(&public_of(&OTHER)),
        )
        .unwrap();
        assert!(read_keys(&d).is_empty());
        std::fs::write(d.join("wolos.pub"), public_pem(&public_of(&SEED))).unwrap();
        assert_eq!(read_keys(&d), [public_of(&SEED)]);
    }

    /// HUB-KEYDIR: a key in keys/hub is trusted for plugins (index + .wolplug) only; a key in
    /// keys/ is trusted for both. Revoked in either list means not trusted for plugins.
    #[test]
    fn hub_keys_are_scoped_to_plugins() {
        let d = dir();
        let hub = d.join(HUB_KEYS_SUBDIR);
        std::fs::create_dir_all(&hub).unwrap();
        std::fs::write(hub.join("hub-index.pub"), public_pem(&public_of(&OTHER))).unwrap();
        let text = br#"{"format":1}"#;
        let by_hub = sign(&OTHER, text);
        let system = read_keys_for(&d, Purpose::System);
        let plugins = read_keys_for(&d, Purpose::Plugins);
        assert!(system.is_empty(), "a hub key is never a system key");
        assert_eq!(plugins, [public_of(&OTHER)]);
        assert!(matches!(
            verify_index(text, Some(&by_hub), &plugins),
            Ok(Trust::Signed(_))
        ));
        assert!(verify_index(text, Some(&by_hub), &system).is_err());
        // A release key verifies both.
        std::fs::write(d.join("wolos.pub"), public_pem(&public_of(&SEED))).unwrap();
        let by_release = sign(&SEED, text);
        let system = read_keys_for(&d, Purpose::System);
        let plugins = read_keys_for(&d, Purpose::Plugins);
        assert_eq!(system, [public_of(&SEED)]);
        assert_eq!(plugins, [public_of(&SEED), public_of(&OTHER)]);
        assert!(matches!(
            verify_index(text, Some(&by_release), &system),
            Ok(Trust::Signed(_))
        ));
        assert!(matches!(
            verify_index(text, Some(&by_release), &plugins),
            Ok(Trust::Signed(_))
        ));
        assert!(verify_index(text, Some(&by_hub), &system).is_err());
        // The same key in both folders counts once.
        std::fs::write(hub.join("dup.pub"), public_pem(&public_of(&SEED))).unwrap();
        assert_eq!(
            read_keys_for(&d, Purpose::Plugins),
            [public_of(&SEED), public_of(&OTHER)]
        );
        // The system revocation list also revokes hub keys.
        std::fs::write(
            d.join(REVOKED_FILE),
            format!("{} leaked\n", hex(public_of(&OTHER))),
        )
        .unwrap();
        assert_eq!(read_keys_for(&d, Purpose::Plugins), [public_of(&SEED)]);
        std::fs::remove_file(d.join(REVOKED_FILE)).unwrap();
        // And keys/hub has its own.
        std::fs::write(
            hub.join(REVOKED_FILE),
            format!("{} leaked\n", hex(public_of(&OTHER))),
        )
        .unwrap();
        assert_eq!(read_keys_for(&d, Purpose::Plugins), [public_of(&SEED)]);
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
        let p = "/var/cache/wolos/install/1/payload.pkg";
        let show = |r: Result<Vec<Vec<String>>, String>| match r {
            Ok(steps) => steps
                .iter()
                .map(|s| s.join(" "))
                .collect::<Vec<_>>()
                .join("; "),
            Err(e) => format!("error: {e}"),
        };
        let cases: [(&str, Option<&str>, &str); 4] = [
            (
                p,
                None,
                "wol pkg -- -U --noconfirm /var/cache/wolos/install/1/payload.pkg",
            ),
            // Same version installed: nothing to do.
            (p, Some("3.1.0-1"), ""),
            // Another version: pacman replaces it.
            (
                p,
                Some("3.0.0-1"),
                "wol pkg -- -U --noconfirm /var/cache/wolos/install/1/payload.pkg",
            ),
            ("-x", None, "error: payload -x is not an absolute path"),
        ];
        for (payload, installed, want) in cases {
            assert_eq!(
                show(install_steps(payload, "3.1.0-1", installed)),
                want,
                "{installed:?}"
            );
        }
    }

    #[test]
    fn remove_steps_from_the_record() {
        let rec = |pkg: &str| INFO.replace("wol-fixture", pkg);
        assert_eq!(
            remove_steps("org.wolos.Fixture", &rec("wol-fixture")).unwrap(),
            [vec![
                "wol",
                "pkg",
                "--",
                "-Rns",
                "--noconfirm",
                "wol-fixture"
            ]]
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
        let a = fetch_argv("https://x/a.wol", "/t/a");
        assert_eq!(a[0], "curl");
        let has = |w: &[&str]| a.windows(w.len()).any(|x| x == w);
        assert!(has(&["--proto", "=https"]) && has(&["--proto-redir", "=https"]));
        assert!(has(&["--max-filesize", "33554432"]));
        // QA-PWR-17: curl stays silent; a 404 on the index (or its optional .sig) is wol's to word.
        assert!(
            has(&["-fsL"]) && !a.iter().any(|x| x.contains('S')),
            "{a:?}"
        );
        assert_eq!(
            a[a.len() - 2..],
            ["--", "https://x/a.wol"],
            "the url is never an option"
        );
    }

    #[test]
    fn pkg_query_output() {
        assert_eq!(
            parse_name_version("raider 3.1.0-1\n"),
            Some(("raider".into(), "3.1.0-1".into()))
        );
        for bad in ["", "raider", "raider ", "-x 1", "a 1 2", "a 1\nb 2\n"] {
            assert_eq!(parse_name_version(bad), None, "{bad:?}");
        }
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
        // CORE-8: a stripped signature is refused once a signature is expected.
        assert!(verify_fetched_index(text, None, &keys, true).is_err());
        assert_eq!(
            verify_fetched_index(text, None, &keys, false),
            Ok(Trust::Unsigned)
        );
        assert_eq!(
            verify_fetched_index(text, Some(&sig), &keys, true),
            Ok(Trust::Signed(0))
        );
        assert!(index_sig_required(true, false, false));
        assert!(index_sig_required(false, true, true));
        assert!(!index_sig_required(true, true, false));
        assert!(!index_sig_required(false, false, false));
    }

    // ---- `.wolplug` ----

    /// Files of a plugin that passes `wol plugin check`, as `<id>/<rel>` members.
    fn plugin_files(id: &str) -> Vec<(String, Vec<u8>)> {
        crate::plugins::scaffold(id, "Demo", &[shared::plugins::Kind::Theme])
            .unwrap()
            .into_iter()
            .map(|(rel, text)| (format!("{id}/{rel}"), text.into_bytes()))
            .collect()
    }

    /// `.wolplug` tar of `files`: the hash list, then the signature by `seed` if any.
    fn wolplug(files: &[(String, Vec<u8>)], seed: Option<&[u8; 32]>) -> Vec<u8> {
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let sha = manifest(&refs);
        let sig = seed.map(|s| sign(s, sha.as_bytes()));
        let mut all = refs;
        all.push(("SHA256", sha.as_bytes()));
        if let Some(s) = &sig {
            all.push((WOLPLUG_SIG, s.as_slice()));
        }
        ustar(&all)
    }

    /// QA-PLUG-20: a hand-made archive (`tar -cf`, with its folder entry) is refused with what to
    /// do (make it with `wol plugin pack`), not as "damaged".
    #[test]
    fn a_hand_made_tar_names_wol_plugin_pack() {
        let d = std::env::temp_dir().join(format!("wol-handtar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("alice.hand")).unwrap();
        std::fs::write(d.join("alice.hand/plugin.toml"), "schema = 1\n").unwrap();
        let out = std::process::Command::new("tar")
            .args(["--format=ustar", "-cf", "-", "-C"])
            .arg(&d)
            .arg("alice.hand")
            .output()
            .unwrap();
        assert!(out.status.success());
        let e = verify_wolplug(&out.stdout, &[]).unwrap_err();
        assert!(
            e.contains("wol plugin pack") && !e.contains("damaged"),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// QA-PLUG-21: the manual names the per-file cap `check` enforces, next to the 1 MiB total.
    #[test]
    fn the_manual_names_the_per_file_cap() {
        let doc = include_str!("../../../docs/modules/plugins.md");
        let cap = format!("{} KiB per file", crate::plugins::MAX_FILE / 1024);
        assert!(doc.contains(&cap), "{cap}");
    }

    #[test]
    fn a_signed_wolplug_verifies_and_installs_for_the_account() {
        let keys = [public_of(&OTHER), public_of(&SEED)];
        let tar = wolplug(&plugin_files("alice.demo"), Some(&SEED));
        let v = verify_wolplug(&tar, &keys).unwrap();
        assert_eq!(v.trust, Trust::Signed(1));
        assert_eq!(v.manifest.id, "alice.demo");
        assert!(v.files.iter().any(|(r, _)| r == "plugin.toml"));
        let v = require_signed(v).unwrap();
        let d = dir();
        let at = install_plugin(&v, &d).unwrap();
        assert_eq!(at.file_name().unwrap(), "alice.demo");
        assert!(crate::plugins::check(&at, false).problems.is_empty());
        // A second install replaces the folder and leaves no staging behind.
        std::fs::write(at.join("stale.txt"), "old").unwrap();
        install_plugin(&v, &d).unwrap();
        assert!(!at.join("stale.txt").exists());
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "only the plugin folder stays");
    }

    /// QA-STORE-23: the staging a dead install left is removed by the next one, the older
    /// version it had moved aside comes back, and a running process's staging is left alone.
    #[test]
    fn a_dead_installs_staging_is_cleaned_and_its_old_version_restored() {
        let dead = {
            let mut c = std::process::Command::new("true").spawn().unwrap();
            c.wait().unwrap();
            c.id()
        };
        let d = dir();
        // Died between the renames: the older "alice.old" is only in the staging's `old`.
        let s1 = d.join(format!(".stage-alice.old-{dead}"));
        std::fs::create_dir_all(s1.join("old")).unwrap();
        std::fs::write(s1.join("old/plugin.toml"), "kept").unwrap();
        // Died before the swap: the plugin folder is in place, the staging is junk.
        std::fs::create_dir_all(d.join("alice.here")).unwrap();
        let s2 = d.join(format!(".stage-alice.here-{dead}/alice.here"));
        std::fs::create_dir_all(&s2).unwrap();
        // A process still running (init) owns this one.
        let live = d.join(".stage-bob.x-1");
        std::fs::create_dir_all(&live).unwrap();
        let tar = wolplug(&plugin_files("alice.demo"), Some(&SEED));
        let v = verify_wolplug(&tar, &[public_of(&SEED)]).unwrap();
        install_plugin(&v, &d).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("alice.old/plugin.toml")).unwrap(),
            "kept"
        );
        assert!(d.join("alice.here").is_dir());
        assert!(!s1.exists() && !s2.exists());
        assert!(live.exists());
    }

    /// QA-STORE-24: a plugin that does not fit says the disk is full, not "os error 28".
    #[test]
    fn a_full_disk_is_named_when_a_plugin_is_written() {
        let full = std::io::Error::from_raw_os_error(28);
        let m = plugin_write_error(full);
        assert!(m.contains("no room left on the disk"), "{m}");
        assert!(!m.contains("os error"), "{m}");
        let other = plugin_write_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(
            other.starts_with("The plugin could not be written"),
            "{other}"
        );
    }

    #[test]
    fn unsigned_and_foreign_wolplugs_are_refused_for_the_index() {
        let files = plugin_files("alice.demo");
        let keys = [public_of(&SEED)];
        let v = verify_wolplug(&wolplug(&files, None), &keys).unwrap();
        assert_eq!(v.trust, Trust::Unsigned);
        assert!(require_signed(v).unwrap_err().contains("not signed"));
        let e = verify_wolplug(&wolplug(&files, Some(&OTHER)), &keys).unwrap_err();
        assert!(e.contains("not signed by a trusted key"), "{e}");
        assert!(verify_wolplug(&wolplug(&files, Some(&SEED)), &[]).is_err());
    }

    #[test]
    fn the_signature_is_checked_before_anything_inside_is_parsed() {
        let mut files = plugin_files("alice.demo");
        let toml = files
            .iter_mut()
            .find(|f| f.0.ends_with("plugin.toml"))
            .unwrap();
        toml.1 = b"schema = \"not a number\"\n[[[ broken".to_vec();
        // A broken manifest from an unknown key: the answer is about the key, not the TOML.
        let e = verify_wolplug(&wolplug(&files, Some(&OTHER)), &[public_of(&SEED)]).unwrap_err();
        assert!(e.contains("trusted key"), "{e}");
        // Signed by the right key, the manifest is then read and refused.
        assert!(verify_wolplug(&wolplug(&files, Some(&SEED)), &[public_of(&SEED)]).is_err());
        // A file changed after signing fails its hash.
        let mut tar = wolplug(&plugin_files("alice.demo"), Some(&SEED));
        let at = tar.windows(9).position(|w| w == b"schema = ").unwrap();
        tar[at + 9] ^= 1;
        let e = verify_wolplug(&tar, &[public_of(&SEED)]).unwrap_err();
        assert!(e.contains("does not match the SHA256 list"), "{e}");
    }

    #[test]
    fn oversized_wolplugs_are_refused() {
        let keys = [public_of(&SEED)];
        let huge = vec![0u8; WOLPLUG_MAX_TAR as usize + 1];
        assert!(
            verify_wolplug(&huge, &keys)
                .unwrap_err()
                .contains("too large")
        );
        let mut one = plugin_files("alice.demo");
        one.push((
            "alice.demo/big.png".into(),
            vec![1; crate::plugins::MAX_FILE as usize + 1],
        ));
        let e = verify_wolplug(&wolplug(&one, Some(&SEED)), &keys).unwrap_err();
        assert!(e.contains("larger than"), "{e}");
        let mut many = plugin_files("alice.demo");
        for i in 0..5 {
            many.push((
                format!("alice.demo/part{i}.png"),
                vec![1; crate::plugins::MAX_FILE as usize],
            ));
        }
        let e = verify_wolplug(&wolplug(&many, Some(&SEED)), &keys).unwrap_err();
        assert!(e.contains("larger than 1 MB"), "{e}");
    }

    #[test]
    fn a_wolplug_holds_one_valid_plugin_folder() {
        let keys = [public_of(&SEED)];
        let bad = |files: Vec<(String, Vec<u8>)>| {
            verify_wolplug(&wolplug(&files, Some(&SEED)), &keys).unwrap_err()
        };
        let mut up = plugin_files("alice.demo");
        up.push(("alice.demo/../evil.sh".into(), b"x".to_vec()));
        assert!(bad(up).contains("not a file inside"));
        let mut two = plugin_files("alice.demo");
        two.extend(plugin_files("bob.other"));
        assert!(bad(two).contains("not a file inside"));
        let renamed: Vec<_> = plugin_files("alice.demo")
            .into_iter()
            .map(|(n, b)| (n.replacen("alice.demo/", "alice.other/", 1), b))
            .collect();
        assert!(bad(renamed).contains("says alice.demo"));
        assert!(bad(plugin_files("org.wolos.fake")).contains("kept for WolOS"));
        let no_toml: Vec<_> = plugin_files("alice.demo")
            .into_iter()
            .filter(|(n, _)| !n.ends_with("plugin.toml"))
            .collect();
        assert!(bad(no_toml).contains("no plugin.toml"));
        let no_theme: Vec<_> = plugin_files("alice.demo")
            .into_iter()
            .filter(|(n, _)| n.ends_with("plugin.toml"))
            .collect();
        assert!(bad(no_theme).contains("which is not in the plugin"));
        // No hash list at the end.
        let refs: Vec<(&str, &[u8])> = vec![("alice.demo/plugin.toml", b"x")];
        let e = verify_wolplug(&ustar(&refs), &keys).unwrap_err();
        assert!(e.contains("SHA256"), "{e}");
    }

    /// A plugin from the hub names its store-page picture and screenshots in plugin.toml (hubcheck
    /// `facts`, WolOS 0.13): they travel in the folder, are checked like every other named file, and a
    /// manifest that uses the keys without min_wolos 0.13 is refused.
    #[test]
    fn a_wolplug_may_carry_its_store_page_pictures() {
        let keys = [public_of(&SEED)];
        let listed = |min: &str, with_files: bool| {
            let mut f = plugin_files("alice.demo");
            let toml = f
                .iter_mut()
                .find(|(n, _)| n.ends_with("plugin.toml"))
                .unwrap();
            let old: String = String::from_utf8(toml.1.clone())
                .unwrap()
                .lines()
                .filter(|l| !l.starts_with("min_wolos"))
                .map(|l| format!("{l}\n"))
                .collect();
            toml.1 = format!(
                "{min}icon = \"icon.png\"\nscreenshots = [\"shots/a.png\"]\nabout = \"One.\\nTwo.\"\n{old}"
            )
            .into_bytes();
            if with_files {
                f.push(("alice.demo/icon.png".into(), b"\x89PNG".to_vec()));
                f.push(("alice.demo/shots/a.png".into(), b"\x89PNG".to_vec()));
            }
            wolplug(&f, Some(&SEED))
        };
        let v = verify_wolplug(&listed("min_wolos = \"0.13\"\n", true), &keys).unwrap();
        assert_eq!(v.manifest.icon, "icon.png");
        assert_eq!(v.manifest.screenshots, ["shots/a.png"]);
        assert_eq!(v.manifest.about, "One.\nTwo.");
        assert!(v.files.iter().any(|(r, _)| r == "shots/a.png"));
        // the picture the manifest names must be in the file
        let e = verify_wolplug(&listed("min_wolos = \"0.13\"\n", false), &keys).unwrap_err();
        assert!(e.contains("not in the plugin"), "{e}");
        // 0.12 systems refuse these keys: the manifest has to say so
        let e = verify_wolplug(&listed("min_wolos = \"0.12\"\n", true), &keys).unwrap_err();
        assert!(e.contains("min_wolos"), "{e}");
        let e = verify_wolplug(&listed("", true), &keys).unwrap_err();
        assert!(e.contains("min_wolos"), "{e}");
        // installed for the account, the folder has the pictures and `wol plugin check` is clean
        let dir = dir();
        let v = verify_wolplug(&listed("min_wolos = \"0.13\"\n", true), &keys).unwrap();
        let at = install_plugin(&v, &dir).unwrap();
        assert!(at.join("shots/a.png").is_file() && at.join("icon.png").is_file());
        assert_eq!(
            crate::plugins::check(&at, false).problems,
            Vec::<String>::new()
        );
    }

    #[test]
    fn damaged_archives_in_memory_are_errors_not_panics() {
        let keys = [public_of(&SEED)];
        let good = wolplug(&plugin_files("alice.demo"), Some(&SEED));
        for cut in [0, 1, 511, 512, 513, good.len() / 2] {
            let _ = members_in(&good[..cut]);
            assert!(verify_wolplug(&good[..cut], &keys).is_err(), "{cut}");
        }
        let mut x: u32 = 1;
        for _ in 0..64 {
            let mut t = good.clone();
            for _ in 0..8 {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let i = (x as usize) % t.len();
                t[i] = (x >> 16) as u8;
            }
            let _ = verify_wolplug(&t, &keys);
        }
        assert_eq!(members_in(&[0u8; 1024]).unwrap(), vec![]);
    }
}
