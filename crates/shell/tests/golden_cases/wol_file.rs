//! Builds real `.wol` files (ADR 0011) for the install page tests and goldens.
use shared::package::{ICON, MANIFEST, PAYLOAD, SIGNATURE};
use std::path::{Path, PathBuf};
use wol_core::package;

/// Signing seed of the test developer; [`trusted`] is its public key.
pub const SEED: [u8; 32] = [7; 32];

pub fn trusted() -> [u8; 32] {
    package::public_of(&SEED)
}

/// Fresh directory for one test's files.
pub fn dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "wol-store-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `<dir>/<name>`: Info.json, optional icon.png, payload, manifest, signature when `signed`.
pub fn write(dir: &Path, name: &str, info: &str, icon: Option<&[u8]>, signed: bool) -> PathBuf {
    let mut m: Vec<(String, Vec<u8>)> = vec![("Info.json".into(), info.into())];
    if let Some(i) = icon {
        m.push((ICON.into(), i.to_vec()));
    }
    m.push((PAYLOAD.into(), b"pkg payload".to_vec()));
    let refs: Vec<(&str, &[u8])> = m.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let man = package::manifest(&refs);
    if signed {
        let sig = package::sign(&SEED, man.as_bytes()).to_vec();
        m.push((MANIFEST.into(), man.into_bytes()));
        m.push((SIGNATURE.into(), sig));
    } else {
        m.push((MANIFEST.into(), man.into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = m.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let p = dir.join(name);
    std::fs::write(&p, package::ustar(&refs)).unwrap();
    p
}

/// File Shredder (ADBeveridge/raider), as the wol-packages factory builds it.
pub const SHREDDER: &str = r#"{"id":"com.github.ADBeveridge.Raider","version":"3.1.0","name":"File Shredder","summary":"Permanently delete your files","icon":"com.github.ADBeveridge.Raider","categories":["Work"],"capabilities":["files"],"package":"raider","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/raider"}]}"#;

/// Wike (hugolabe/Wike).
pub const WIKE: &str = r#"{"id":"com.github.hugolabe.Wike","version":"3.1.1","name":"Wike","summary":"Search and read Wikipedia articles","icon":"com.github.hugolabe.Wike","categories":["Work"],"capabilities":["network","notifications"],"package":"wike","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/wike"}]}"#;

/// A copy of `p` with one payload byte changed after signing.
pub fn tampered(p: &Path) -> PathBuf {
    let mut b = std::fs::read(p).unwrap();
    let at = b
        .windows(11)
        .position(|w| w == b"pkg payload")
        .expect("payload bytes");
    b[at] = b'P';
    let out = p.with_file_name("Raider.wol");
    std::fs::write(&out, b).unwrap();
    out
}
