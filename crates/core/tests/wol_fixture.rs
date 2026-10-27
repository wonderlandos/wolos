//! Cross-check (ADR 0011): files made by the factory's `wol-port pack`/`sign` (openssl key)
//! verify with `wol_core::package`. Regenerate with wol-packages `tools/wol-port`.
use std::path::Path;
use wol_core::package::{Trust, public_key, verify};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wol")
        .join(name)
}

#[test]
fn factory_files_verify() {
    let key = public_key(&std::fs::read_to_string(fixture("fixture.pub")).unwrap()).unwrap();
    let v = verify(&fixture("fixture-signed.wol"), &[key]).unwrap();
    assert_eq!(v.trust, Trust::Signed(0));
    assert_eq!(
        (
            v.info.id.as_str(),
            v.info.package.as_str(),
            v.info.version.as_str()
        ),
        ("org.wolos.Fixture", "wol-fixture", "1.0")
    );
    assert_eq!(v.info.capabilities, ["files"]);
    assert!(v.icon.is_some());
    let v = verify(&fixture("fixture-unsigned.wol"), &[key]).unwrap();
    assert_eq!(v.trust, Trust::Unsigned);
    // Same signed file, key not trusted here: refused.
    assert!(verify(&fixture("fixture-signed.wol"), &[[1; 32]]).is_err());
}

/// The image trusts the WolOS key: every shipped `.pub` parses (a broken file would refuse
/// every signed package).
#[test]
fn shipped_keys_parse() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/overlay")
        .join(shared::package::KEYS_DIR.trim_start_matches('/'));
    let files = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
        .count();
    assert!(files >= 1, "no key in {}", dir.display());
    assert_eq!(wol_core::package::read_keys(&dir).len(), files);
}
