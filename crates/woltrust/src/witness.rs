// SPDX-License-Identifier: BSD-2-Clause
//! The witness (lead decisions 2026-11-04 03:2xZ): detached post-quantum signatures on published
//! files, the two root certificates, checkpoints, revocation databases, release manifests and
//! images. A witness is an intermediate with usage `witness` alone, hybrid Ed25519 + ML-DSA-65,
//! signed by both roots; there is one per root (`wolos-witness-a`, `wolos-witness-b`). Its
//! signature sits beside the file as `<file>.wsig` (the `.wsig` shape with `"at"`), over
//! `"wolos/witness/1" 0x00 || SHA-512(file) || SHA-512(at as decimal text)`. The verifier refuses a
//! witness as the signer of anything else, and a withdrawn witness touches no app.
use crate::crypto::{Purpose, Wsig};
use crate::store::Store;
use crate::verify::{Input, Outcome};

/// The signed message of a witness signature made at `at`.
pub fn message(file: &[u8], at: u64) -> Vec<u8> {
    wolcrypt::sign::message_with(Purpose::Witness.label(), file, &[at.to_string().as_bytes()])
}

/// `<file>.wsig` by the witness `serial`, signed with `secret` at `at`.
pub fn sign(serial: &str, secret: &crate::make::Secret, file: &[u8], at: u64) -> Vec<u8> {
    let s = secret.sign_message(&message(file, at));
    Wsig {
        cert: serial.to_owned(),
        ed25519: crate::b64::encode(&s.ed25519),
        second: crate::b64::encode(&s.second),
        origin: None,
        at: Some(at),
    }
    .to_bytes()
}

/// A checkpoint (`revoked.checkpoint.json`) that counts only with a witness's signature beside it
/// (`revoked.checkpoint.json.wsig`, audit 2 M2): TLS alone is classical, and one forged
/// checkpoint must not move a machine's floor. None when the signature does not verify as
/// `wolos` or the text is not a checkpoint.
pub fn checkpoint(
    store: &Store,
    file: &[u8],
    wsig: &[u8],
    now: u64,
) -> Option<crate::rkdb::Checkpoint> {
    let out = verify(store, file, wsig, now);
    (out.trust == crate::report::Trust::Wolos).then_some(())?;
    crate::rkdb::Checkpoint::parse(file)
}

/// Checks `wsig` over `file` against the store (roots pinned, the witness among the
/// intermediates). Valid when the outcome's trust is `wolos`; its `line` is the witness line. A
/// signature without `at` is not a witness signature and does not verify.
pub fn verify(store: &Store, file: &[u8], wsig: &[u8], now: u64) -> Outcome {
    crate::verify::verify(
        store,
        &Input {
            purpose: Purpose::Witness,
            content: file,
            app_id: None,
            wsig: Some(wsig),
            chain: None,
            first_seen: None,
            now,
        },
    )
}
