// SPDX-License-Identifier: BSD-2-Clause
//! The TEST certificate authority: a root pair, three intermediates and five leaves, all derived
//! from public seeds, so anyone can rebuild the same bytes. It exists for the test vectors and for
//! tests in other crates. Its roots carry the subject label `test-...`, so [`crate::Store::load`]
//! refuses them unless asked, and the image build refuses a trust folder that holds one.
use crate::cert::{Body, Cert, KeysText, Kind, Level, Subject, Usage};
use crate::make::{self, Secret};
use sha2::{Digest, Sha512};

/// The clock of the vectors: 2026-02-02 00:00:00 UTC.
pub const NOW: u64 = 1_770_000_000;
/// One day in seconds.
pub const DAY: u64 = 86_400;

/// One test identity: its secret, its body and its `.wolcert` bytes.
pub struct Ident {
    /// Signing keys.
    pub secret: Secret,
    /// The signed body.
    pub body: Body,
    /// The `.wolcert` file.
    pub file: Vec<u8>,
}

/// The whole test authority.
pub struct Pki {
    /// Active root.
    pub root_a: Ident,
    /// Standby root.
    pub root_b: Ident,
    /// Authors CA (issues people and organisations), by root A.
    pub authors: Ident,
    /// Packages CA (issues WolOS signers), by root A.
    pub packages: Ident,
    /// Index signer (signs the index and the revocation list), by root A.
    pub index: Ident,
    /// A person (`alice.`), by the Authors CA.
    pub alice: Ident,
    /// An organisation (`examplelabs.`), by the Authors CA.
    pub org: Ident,
    /// A WolOS signer (`org.wolos.`), by the Packages CA.
    pub signer: Ident,
    /// A person (`ana.`) who also publishes for the organisation `anaco`, verified by the hub on a
    /// given day: the origin fields of ADR 0046.
    pub ana: Ident,
    /// Alice's trusted certificate (`class: trusted`, after an administrator approved her): its own
    /// keys; [`Pki::alice`] is her origin certificate, named in `origin_cert`.
    pub alice_trusted: Ident,
}

/// 32 bytes derived from a public name.
pub fn seed(name: &str) -> [u8; 32] {
    let d = Sha512::digest(format!("woltrust/testseed/{name}"));
    let mut out = [0u8; 32];
    out.copy_from_slice(&d[..32]);
    out
}

/// 16 hex characters derived from a public name.
pub fn serial(name: &str) -> String {
    let d = Sha512::digest(format!("woltrust/serial/{name}"));
    d[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Everything that varies between test identities.
pub struct Spec<'a> {
    /// Name, from which seed and serial derive.
    pub name: &'a str,
    /// Kind.
    pub kind: Kind,
    /// Subject.
    pub subject: Subject,
    /// Namespaces.
    pub namespaces: &'a [&'a str],
    /// Usage.
    pub usage: &'a [Usage],
    /// Issuer serial (the own serial for a root).
    pub issuer: String,
    /// Validity start.
    pub not_before: u64,
    /// Validity end.
    pub not_after: u64,
    /// Level.
    pub level: Level,
}

/// The body of `spec` for the keys of `secret`.
pub fn body(spec: &Spec, secret: &Secret) -> Body {
    let keys: KeysText = make::keys_text(secret);
    Body {
        serial: serial(spec.name),
        kind: spec.kind,
        subject: spec.subject.clone(),
        namespaces: spec.namespaces.iter().map(|s| (*s).to_owned()).collect(),
        usage: spec.usage.to_vec(),
        keys,
        issuer: spec.issuer.clone(),
        not_before: spec.not_before,
        not_after: spec.not_after,
        level: spec.level,
        verified_at: None,
        class: None,
        origin_cert: None,
        terms: None,
    }
}

fn label(l: &str) -> Subject {
    Subject {
        label: Some(l.to_owned()),
        ..Subject::default()
    }
}

fn ident(spec: &Spec, issuer: Option<&Secret>) -> Ident {
    let secret = Secret::from_seed(&seed(spec.name), spec.kind == Kind::Root);
    let body = body(spec, &secret);
    let file = make::issue(&body, issuer.unwrap_or(&secret));
    Ident { secret, body, file }
}

/// Alice's trusted certificate: her own keys for it (`test-alice-trusted`), `class: trusted`,
/// naming her origin certificate, issued by the Authors CA.
fn trusted_of(alice: &Ident, authors: &Ident) -> Ident {
    let secret = Secret::from_seed(&seed("test-alice-trusted"), false);
    let mut b = alice.body.clone();
    b.serial = serial("test-alice-trusted");
    b.keys = make::keys_text(&secret);
    b.class = Some(crate::cert::Class::Trusted);
    b.origin_cert = Some(alice.body.serial.clone());
    let file = make::issue(&b, &authors.secret);
    Ident {
        secret,
        body: b,
        file,
    }
}

impl Ident {
    /// The parsed certificate (for tests that inspect it).
    pub fn cert(&self) -> Option<Cert> {
        Cert::parse(&self.file).ok()
    }
}

impl Pki {
    /// Builds the authority. Deterministic: two calls give the same bytes.
    pub fn new() -> Pki {
        let t0 = NOW - 30 * DAY;
        let year = 365 * DAY;
        let root = |name: &'static str| Spec {
            name,
            kind: Kind::Root,
            subject: label(name),
            namespaces: &["*", "org.wolos."],
            usage: &[Usage::SignCerts, Usage::SignRevocations],
            issuer: serial(name),
            not_before: t0 - year,
            not_after: t0 + 20 * year,
            level: Level::Wolos,
        };
        let root_a = ident(&root("test-root-a"), None);
        let root_b = ident(&root("test-root-b"), None);
        let ra = &root_a.secret;
        let ca = |name: &'static str, ns: &'static [&'static str], usage: &'static [Usage]| Spec {
            name,
            kind: Kind::Intermediate,
            subject: label(name),
            namespaces: ns,
            usage,
            issuer: serial("test-root-a"),
            not_before: t0,
            not_after: t0 + 3 * year,
            level: Level::Wolos,
        };
        let authors = ident(&ca("test-authors", &["*"], &[Usage::SignCerts]), Some(ra));
        let packages = ident(
            &ca("test-packages", &["org.wolos."], &[Usage::SignCerts]),
            Some(ra),
        );
        let index = ident(
            &ca(
                "test-index",
                &[],
                &[Usage::SignIndex, Usage::SignRevocations],
            ),
            Some(ra),
        );
        let leaf_usage = &[Usage::SignPackage, Usage::SignPlugin, Usage::SignExtension];
        let alice = ident(
            &Spec {
                name: "test-alice",
                kind: Kind::Person,
                subject: Subject {
                    github_id: Some(1001),
                    login: Some("alice".into()),
                    name: Some("Alice Doe".into()),
                    label: None,
                    org: None,
                    text: None,
                },
                namespaces: &["alice."],
                usage: leaf_usage,
                issuer: serial("test-authors"),
                not_before: t0,
                not_after: t0 + year,
                level: Level::Verified,
            },
            Some(&authors.secret),
        );
        let org = ident(
            &Spec {
                name: "test-examplelabs",
                kind: Kind::Org,
                subject: Subject {
                    github_id: Some(2002),
                    login: Some("examplelabs".into()),
                    name: Some("Example Labs".into()),
                    label: None,
                    org: None,
                    text: None,
                },
                namespaces: &["examplelabs."],
                usage: leaf_usage,
                issuer: serial("test-authors"),
                not_before: t0,
                not_after: t0 + year,
                level: Level::Verified,
            },
            Some(&authors.secret),
        );
        let signer = ident(
            &Spec {
                name: "test-signer",
                kind: Kind::Signer,
                subject: label("test-signer"),
                namespaces: &["org.wolos."],
                usage: leaf_usage,
                issuer: serial("test-packages"),
                not_before: t0,
                not_after: t0 + year,
                level: Level::Wolos,
            },
            Some(&packages.secret),
        );
        let ana = {
            let spec = Spec {
                name: "test-ana",
                kind: Kind::Person,
                subject: Subject {
                    github_id: Some(4004),
                    login: Some("ana".into()),
                    name: Some("Ana Lima".into()),
                    label: None,
                    org: Some(crate::cert::OrgRef {
                        github_id: 5005,
                        login: "anaco".into(),
                    }),
                    text: None,
                },
                namespaces: &["ana."],
                usage: leaf_usage,
                issuer: serial("test-authors"),
                not_before: t0,
                not_after: t0 + year,
                level: Level::Verified,
            };
            let secret = Secret::from_seed(&seed(spec.name), false);
            let mut b = body(&spec, &secret);
            b.verified_at = Some(t0 - DAY);
            let file = make::issue(&b, &authors.secret);
            Ident {
                secret,
                body: b,
                file,
            }
        };
        let alice_trusted = trusted_of(&alice, &authors);
        Pki {
            root_a,
            root_b,
            authors,
            packages,
            index,
            alice,
            org,
            signer,
            ana,
            alice_trusted,
        }
    }

    /// `[leaf, issuer]` as a `CHAIN` member.
    pub fn chain(parts: &[&Ident]) -> Vec<u8> {
        chain_of(&parts.iter().map(|i| i.file.as_slice()).collect::<Vec<_>>())
    }
}

impl Default for Pki {
    fn default() -> Self {
        Pki::new()
    }
}

/// A `CHAIN` member from `.wolcert` files: a JSON array of the certificate objects.
pub fn chain_of(files: &[&[u8]]) -> Vec<u8> {
    let items: Vec<serde_json::Value> = files
        .iter()
        .filter_map(|f| serde_json::from_slice(f).ok())
        .collect();
    serde_json::to_vec(&items).unwrap_or_default()
}
