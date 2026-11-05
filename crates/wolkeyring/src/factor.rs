// SPDX-License-Identifier: BSD-2-Clause
//! Unlock factors and the policy (ADR 0054 section 3). Each factor gives a 32-byte factor
//! secret; a rule is a set of factors that together unlock; the policy is a list of rules, any one
//! of which unlocks. For every rule the header keeps one wrap of the master key, sealed under
//! `SHA-512(len-prefixed FORMAT "/wrap", store id, rule id, fresh salt, descriptor bytes, then
//! each factor id and secret in factor order)`. The descriptors of the rule's factors (kind, KDF,
//! salts, credential ids) are the associated data, so a changed KDF parameter or salt makes the
//! wrap fail to open instead of deriving a weaker key.
//!
//! Policy invariants, checked when a policy is made and again when a header is read:
//! a `pin` only in a rule that also has `tpm`; `tpm` and `fingerprint` only where the machine
//! sealed them; every rule has at least one factor; at most [`MAX_FACTORS`] factors and
//! [`MAX_RULES`] rules. Export, move and policy changes need a rule with a knowledge or
//! possession factor ([`Kind::strong`]): never the fingerprint alone, never the TPM alone.
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::kdf::Kdf;
use crate::wire;

/// Most factors one keyring may have.
pub const MAX_FACTORS: usize = 8;
/// Most rules (8 factors, any 3: 56; any 4: 70).
pub const MAX_RULES: usize = 80;
/// Shortest login password or recovery input accepted for a new factor, in characters.
pub const MIN_PASSWORD: usize = 8;
/// Shortest PIN, in digits.
pub const MIN_PIN: usize = 6;

/// What kind of factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// The login password, through the KDF.
    Password,
    /// A PIN, through the KDF; only together with `tpm`.
    Pin,
    /// 32 random bytes sealed by the TPM2 (`systemd-creds --with-key=tpm2`).
    Tpm,
    /// 32 random bytes sealed by the TPM2, released after an fprintd match.
    Fingerprint,
    /// A FIDO2 security key's hmac-secret output.
    Fido2,
    /// A 128-bit recovery code, through the KDF.
    Recovery,
}

impl Kind {
    /// Knowledge or possession a person shows on purpose: a rule needs one of these for export,
    /// move and policy changes (a fingerprint or the machine's TPM alone never suffice).
    pub fn strong(self) -> bool {
        matches!(self, Kind::Password | Kind::Fido2 | Kind::Recovery)
    }
    /// Whether the factor secret comes from something typed, through the KDF.
    pub fn typed(self) -> bool {
        matches!(self, Kind::Password | Kind::Pin | Kind::Recovery)
    }
    /// Whether the factor secret is a sealed 32-byte blob the system opens.
    pub fn sealed(self) -> bool {
        matches!(self, Kind::Tpm | Kind::Fingerprint)
    }
}

/// One enrolled factor, as the header stores it (no secret in it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Factor {
    /// 1..=255, unique in the keyring.
    pub id: u8,
    pub kind: Kind,
    /// What Settings shows ("Login password", "YubiKey 5C").
    pub label: String,
    /// Typed factors: the KDF and its salt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf: Option<Kdf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    /// TPM and fingerprint: the `systemd-creds` blob (base64url).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
    /// FIDO2: credential id and hmac salt (base64url).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hmac_salt: Option<String>,
}

impl Factor {
    /// The bytes a wrap binds for this factor (its descriptor as stored).
    pub fn descriptor(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Checks the descriptor has exactly the fields its kind uses.
    pub fn check(&self) -> crate::Result<()> {
        if self.id == 0 || self.label.len() > 128 {
            return Err(crate::Error::Malformed("factor id or label"));
        }
        let typed = self.kdf.is_some() && self.salt.is_some();
        let sealed = self.sealed.is_some();
        let fido = self.credential.is_some() && self.hmac_salt.is_some();
        let ok = match self.kind {
            k if k.typed() => typed && !sealed && !fido,
            k if k.sealed() => sealed && !typed && !fido,
            _ => fido && !typed && !sealed,
        };
        if !ok {
            return Err(crate::Error::Malformed(
                "factor fields do not match its kind",
            ));
        }
        if let Some(k) = &self.kdf {
            k.check()?;
        }
        Ok(())
    }

    /// The factor secret from what was presented: typed input through the KDF; a sealed blob's
    /// or a FIDO2 key's 32 bytes through a labelled hash. The kinds must match.
    pub fn secret(&self, given: &Presented) -> crate::Result<Zeroizing<[u8; 32]>> {
        match (self.kind, given) {
            (k, Presented::Typed(t)) if k.typed() => {
                let kdf = self.kdf.as_ref().ok_or(crate::Error::Malformed("kdf"))?;
                let salt = wire::unb64(self.salt.as_deref().unwrap_or(""), 64)?;
                let t = normalize(k, t).ok_or(crate::Error::WrongFactors)?;
                kdf.derive(t.as_bytes(), &salt)
            }
            (k, Presented::Bytes(b)) if !k.typed() => {
                let label = format!("{}/factor/{}", crate::FORMAT, kind_name(k));
                let h = wire::derive(&[label.as_bytes(), &b[..]]);
                let mut s = Zeroizing::new([0u8; 32]);
                s.copy_from_slice(&h[..32]);
                Ok(s)
            }
            _ => Err(crate::Error::WrongFactors),
        }
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Password => "password",
        Kind::Pin => "pin",
        Kind::Tpm => "tpm",
        Kind::Fingerprint => "fingerprint",
        Kind::Fido2 => "fido2",
        Kind::Recovery => "recovery",
    }
}

/// A recovery code is read case-blind and without the dashes and spaces people type between its
/// groups; other typed input is taken as typed.
fn normalize(k: Kind, t: &str) -> Option<Zeroizing<String>> {
    match k {
        Kind::Recovery => read_recovery_code(t),
        _ => Some(Zeroizing::new(t.to_string())),
    }
}

/// What a person or the system presented for one factor.
pub enum Presented {
    /// Typed: password, PIN, recovery code.
    Typed(Zeroizing<String>),
    /// 32 bytes from the TPM (tpm, fingerprint) or a FIDO2 key.
    Bytes(Zeroizing<[u8; 32]>),
}

/// The recovery code shown once: 128 random bits in Crockford base32, 26 characters in groups
/// of five (`XXXXX-XXXXX-XXXXX-XXXXX-XXXXX-X`).
pub fn recovery_code(raw: &[u8; 16]) -> Zeroizing<String> {
    const A: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut n = u128::from_be_bytes(*raw);
    let mut chars = [0u8; 26];
    for c in chars.iter_mut().rev() {
        *c = A[(n & 31) as usize];
        n >>= 5;
    }
    let mut out = Zeroizing::new(String::with_capacity(31));
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % 5 == 0 {
            out.push('-');
        }
        out.push(char::from(*c));
    }
    out
}

/// A recovery code as typed, normalised (upper case, no dashes or spaces, I/L read as 1, O as 0),
/// or None when it is not 26 base32 characters.
pub fn read_recovery_code(typed: &str) -> Option<Zeroizing<String>> {
    let mut s = Zeroizing::new(String::new());
    for c in typed.chars() {
        let c = c.to_ascii_uppercase();
        let c = match c {
            '-' | ' ' => continue,
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        };
        if !"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c) {
            return None;
        }
        s.push(c);
    }
    (s.len() == 26).then(|| {
        let mut out = Zeroizing::new(String::new());
        for (i, c) in s.chars().enumerate() {
            if i > 0 && i % 5 == 0 {
                out.push('-');
            }
            out.push(c);
        }
        out
    })
}

/// One rule: the factors that together open one wrap of the master key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// 1..=255, unique.
    pub id: u8,
    /// Factor ids, ascending, no repeats.
    pub factors: Vec<u8>,
    /// Fresh 32-byte salt of this wrap (base64url).
    pub salt: String,
    /// The master key sealed under the rule's key (base64url, 32 + 16 bytes).
    pub wrapped: String,
}

/// Checks a whole policy: the factors and rules are well formed and keep the invariants above.
pub fn check_policy(factors: &[Factor], rules: &[Rule]) -> crate::Result<()> {
    if factors.is_empty() || factors.len() > MAX_FACTORS {
        return Err(crate::Error::Policy("between 1 and 8 factors"));
    }
    if rules.is_empty() || rules.len() > MAX_RULES {
        return Err(crate::Error::Policy("between 1 and 80 rules"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for f in factors {
        f.check()?;
        if !ids.insert(f.id) {
            return Err(crate::Error::Malformed("two factors with one id"));
        }
    }
    let mut rids = std::collections::BTreeSet::new();
    for r in rules {
        if r.id == 0 || !rids.insert(r.id) {
            return Err(crate::Error::Malformed("rule id"));
        }
        let kinds = rule_kinds(factors, &r.factors)?;
        if kinds.contains(&Kind::Pin) && !kinds.contains(&Kind::Tpm) {
            return Err(crate::Error::Policy(
                "a PIN unlocks only together with the security chip",
            ));
        }
    }
    Ok(())
}

/// The kinds of a rule's factors; the ids must be ascending, unique and enrolled.
pub fn rule_kinds(factors: &[Factor], ids: &[u8]) -> crate::Result<Vec<Kind>> {
    if ids.is_empty() || ids.windows(2).any(|w| w[0] >= w[1]) {
        return Err(crate::Error::Malformed("rule factor list"));
    }
    ids.iter()
        .map(|id| {
            factors
                .iter()
                .find(|f| f.id == *id)
                .map(|f| f.kind)
                .ok_or(crate::Error::Malformed(
                    "rule names a factor that is not enrolled",
                ))
        })
        .collect()
}

/// Whether a rule may authorise export, move and policy changes.
pub fn rule_is_strong(factors: &[Factor], ids: &[u8]) -> bool {
    rule_kinds(factors, ids).is_ok_and(|k| k.iter().any(|k| k.strong()))
}

/// The wrap's AEAD parts and associated data for a rule.
fn wrap_input(
    store: &[u8; 16],
    rule_id: u8,
    salt: &[u8; 32],
    factors: &[Factor],
    ids: &[u8],
    secrets: &[&[u8; 32]],
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut ad = Vec::new();
    ad.extend_from_slice(crate::FORMAT.as_bytes());
    ad.extend_from_slice(b"/wrap\0");
    ad.extend_from_slice(store);
    ad.push(rule_id);
    let mut parts: Vec<Vec<u8>> = vec![
        format!("{}/wrap", crate::FORMAT).into_bytes(),
        store.to_vec(),
        vec![rule_id],
        salt.to_vec(),
    ];
    for (id, s) in ids.iter().zip(secrets) {
        let d = factors
            .iter()
            .find(|f| f.id == *id)
            .map(Factor::descriptor)
            .unwrap_or_default();
        ad.extend_from_slice(&(d.len() as u32).to_be_bytes());
        ad.extend_from_slice(&d);
        parts.push(vec![*id]);
        parts.push(s.to_vec());
    }
    parts.push(ad.clone());
    (parts, ad)
}

/// Seals the master key for a rule. `secrets` are the factor secrets in the order of `ids`.
pub fn wrap(
    rng: &mut impl crate::rand::Random,
    store: &[u8; 16],
    rule_id: u8,
    factors: &[Factor],
    ids: &[u8],
    secrets: &[&[u8; 32]],
    mk: &[u8; 32],
) -> crate::Result<Rule> {
    rule_kinds(factors, ids)?;
    if secrets.len() != ids.len() {
        return Err(crate::Error::WrongFactors);
    }
    let salt: [u8; 32] = rng.bytes()?;
    let (parts, ad) = wrap_input(store, rule_id, &salt, factors, ids, secrets);
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let sealed = wire::seal(&refs, &ad, mk);
    Ok(Rule {
        id: rule_id,
        factors: ids.to_vec(),
        salt: wire::b64(&salt),
        wrapped: wire::b64(&sealed),
    })
}

/// Opens a rule's wrap with the factor secrets (in the order of the rule's ids).
pub fn unwrap(
    store: &[u8; 16],
    rule: &Rule,
    factors: &[Factor],
    secrets: &[&[u8; 32]],
) -> crate::Result<Zeroizing<[u8; 32]>> {
    if secrets.len() != rule.factors.len() {
        return Err(crate::Error::WrongFactors);
    }
    let salt: [u8; 32] = wire::unb64_n(&rule.salt)?;
    let sealed = wire::unb64(&rule.wrapped, 48)?;
    let (parts, ad) = wrap_input(store, rule.id, &salt, factors, &rule.factors, secrets);
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let mk = wire::open(&refs, &ad, &sealed).ok_or(crate::Error::WrongFactors)?;
    let mut out = Zeroizing::new([0u8; 32]);
    if mk.len() != 32 {
        return Err(crate::Error::Damaged("wrapped key length"));
    }
    out.copy_from_slice(&mk);
    Ok(out)
}

/// Every `n`-subset of `ids` (ascending), for an "n of m" policy.
pub fn n_of_m(ids: &[u8], n: usize) -> Vec<Vec<u8>> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = Vec::new();
    let mut pick = Vec::new();
    fn go(ids: &[u8], n: usize, from: usize, pick: &mut Vec<u8>, out: &mut Vec<Vec<u8>>) {
        if pick.len() == n {
            out.push(pick.clone());
            return;
        }
        for i in from..ids.len() {
            pick.push(ids[i]);
            go(ids, n, i + 1, pick, out);
            pick.pop();
        }
    }
    if n >= 1 && n <= sorted.len() {
        go(&sorted, n, 0, &mut pick, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rand::Counter;

    fn typed(id: u8, kind: Kind) -> Factor {
        Factor {
            id,
            kind,
            label: format!("{kind:?}"),
            kdf: Some(Kdf::Pbkdf2 {
                rounds: wolcrypt::kdf::MIN_ROUNDS,
            }),
            salt: Some(wire::b64(&[id; 16])),
            sealed: None,
            credential: None,
            hmac_salt: None,
        }
    }
    fn tpm(id: u8) -> Factor {
        Factor {
            id,
            kind: Kind::Tpm,
            label: "chip".into(),
            kdf: None,
            salt: None,
            sealed: Some(wire::b64(b"blob")),
            credential: None,
            hmac_salt: None,
        }
    }

    #[test]
    fn a_pin_alone_is_refused() {
        let f = vec![typed(1, Kind::Pin), tpm(2)];
        let r = |ids: Vec<u8>| Rule {
            id: 1,
            factors: ids,
            salt: String::new(),
            wrapped: String::new(),
        };
        assert_eq!(
            check_policy(&f, &[r(vec![1])]),
            Err(crate::Error::Policy(
                "a PIN unlocks only together with the security chip"
            ))
        );
        assert!(check_policy(&f, &[r(vec![1, 2])]).is_ok());
        assert!(
            check_policy(&f, &[r(vec![2, 1])]).is_err(),
            "ids must ascend"
        );
    }

    #[test]
    fn strong_rules() {
        let mut fp = tpm(3);
        fp.kind = Kind::Fingerprint;
        let f = vec![typed(1, Kind::Password), tpm(2), fp];
        assert!(rule_is_strong(&f, &[1]));
        assert!(!rule_is_strong(&f, &[2]));
        assert!(!rule_is_strong(&f, &[3]));
        assert!(!rule_is_strong(&f, &[2, 3]));
        assert!(rule_is_strong(&f, &[1, 3]));
    }

    #[test]
    fn wrap_opens_only_with_every_factor_and_the_same_descriptors() {
        let mut rng = Counter::new(1);
        let store = [7u8; 16];
        let f = vec![typed(1, Kind::Password), tpm(2)];
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let mk = [9u8; 32];
        let rule = wrap(&mut rng, &store, 1, &f, &[1, 2], &[&a, &b], &mk).unwrap();
        assert_eq!(*unwrap(&store, &rule, &f, &[&a, &b]).unwrap(), mk);
        assert_eq!(
            unwrap(&store, &rule, &f, &[&a, &a]),
            Err(crate::Error::WrongFactors)
        );
        assert_eq!(
            unwrap(&store, &rule, &f, &[&a]),
            Err(crate::Error::WrongFactors)
        );
        assert_eq!(
            unwrap(&[8u8; 16], &rule, &f, &[&a, &b]),
            Err(crate::Error::WrongFactors)
        );
        // A cheaper KDF written into the header (a downgrade) breaks the wrap.
        let mut g = f.clone();
        g[0].kdf = Some(Kdf::Pbkdf2 {
            rounds: wolcrypt::kdf::MIN_ROUNDS + 1,
        });
        assert_eq!(
            unwrap(&store, &rule, &g, &[&a, &b]),
            Err(crate::Error::WrongFactors)
        );
    }

    #[test]
    fn n_of_m_lists_subsets() {
        assert_eq!(
            n_of_m(&[3, 1, 2], 2),
            vec![vec![1, 2], vec![1, 3], vec![2, 3]]
        );
        assert_eq!(n_of_m(&[1, 2, 3, 4, 5, 6], 3).len(), 20);
        assert!(n_of_m(&[1], 2).is_empty());
    }

    #[test]
    fn recovery_codes_round_trip_as_typed() {
        let c = recovery_code(&[0xA5; 16]);
        assert_eq!(c.len(), 31);
        let typed = c.to_lowercase().replace('-', " ").replace('1', "l");
        assert_eq!(read_recovery_code(&typed).unwrap().as_str(), c.as_str());
        assert!(read_recovery_code("too short").is_none());
    }

    #[test]
    fn kinds_must_match_presented() {
        let f = typed(1, Kind::Password);
        assert_eq!(
            f.secret(&Presented::Bytes(Zeroizing::new([0; 32]))).err(),
            Some(crate::Error::WrongFactors)
        );
        let t = tpm(2);
        assert_eq!(
            t.secret(&Presented::Typed(Zeroizing::new("x".into())))
                .err(),
            Some(crate::Error::WrongFactors)
        );
        assert!(t.secret(&Presented::Bytes(Zeroizing::new([0; 32]))).is_ok());
    }
}
