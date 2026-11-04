// SPDX-License-Identifier: BSD-2-Clause
//! `woltrust`: the command line the hub pipeline calls (hub-index README, "Tooling contract"),
//! over the pure crate. Output goes to stdout, a reason to stderr; exit 0 on success, 1 on a
//! refusal, 2 on a usage error. Key material comes from `--key-env NAME` (a CI secret) or
//! `--key-file F`, never from an argument: either a `.wolkey` (locked; the passphrase comes from
//! `--passphrase-env NAME`, else a prompt on the terminal) or plain key material.
//!
//! ```text
//! woltrust sign   --purpose index|revocation|package|plugin|extension --cert SERIAL [--origin ORIGIN.json] KEY FILE
//! woltrust issue  --body BODY.json [--proof PROOF.json] [--issuer-cert ISSUER.wolcert] KEY
//!                 [--cosign-file ROOT_B.wolkey [--cosign-passphrase-env NAME]]  (intermediates)
//! woltrust revoke --source revocations.json [--previous revoked.json] [--now UNIX]
//! woltrust verify --purpose P --wsig F [--chain F] --roots DIR [--intermediates DIR]
//!                 [--list revoked.json] [--id APP_ID] [--first-seen UNIX] [--now UNIX] [--allow-test] FILE
//! woltrust keygen [--root]
//! woltrust public KEY
//! woltrust init --out DIR --subject-a TEXT --subject-b TEXT [--passphrase-env NAME] [--now UNIX]
//! woltrust init --rotate-intermediates --backup FILE --root root-a|root-b --subject-new TEXT --out DIR
//! woltrust emergency --root A|B --roots DIR --revoke ID [--revoke ID...] --reason key-compromise
//!                    --out FILE.rkdr KEY   (KEY = the OTHER root's key; asks to type each id back)
//! woltrust rkdb make --source revoked.json|revocations.json [--previous revoked.rkdb]
//!                    [--root-section F.rkdr] [--now UNIX] --cert INDEX_ID KEY
//! woltrust rkdb delta BASE.rkdb TARGET.rkdb | rkdb apply BASE.rkdb DELTA.rkdd
//! woltrust rkdb verify --roots DIR [--intermediates DIR] [--floor N] [--now UNIX] [--allow-test] FILE
//!                      (FILE = a .rkdb, or a bare root section .rkdr)
//! woltrust rkdb export FILE | rkdb checkpoint FILE
//! woltrust witness sign --cert WITNESS_ID [--now UNIX] KEY FILE      (prints FILE.wsig)
//! woltrust witness verify --roots DIR --intermediates DIR [--now UNIX] FILE [FILE.wsig]
//! woltrust extract --backup FILE --role root-a|root-b|authors-ca|packages-ca|index [--certificate]
//! woltrust seal --to PUBLIC [--label L] FILE|-
//! woltrust open [--serial S] [--label L] KEY FILE|-
//!   PUBLIC = a .wolcert with encryption keys, or the output of `woltrust public` (serial = key id)
//!   L defaults to wolos/file/1; `-` reads stdin
//!   KEY = (--key-env NAME | --key-file F) [--passphrase-env NAME]
//! ```
#![forbid(unsafe_code)]
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use woltrust::cert::{Body, Kind};
use woltrust::crypto::Purpose;
use woltrust::key::KeyMaterial;
use woltrust::revoked::{Entry, ListText};
use woltrust::{Store, init, make, request, rkdb, wolkey};

const FLAGS: [&str; 5] = [
    "allow-test",
    "root",
    "certificate",
    "rotate-intermediates",
    "line",
];

struct Args {
    many: Vec<String>,
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
    rest: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        many: Vec::new(),
        opts: BTreeMap::new(),
        flags: Vec::new(),
        rest: Vec::new(),
    };
    let mut it = argv.iter();
    while let Some(x) = it.next() {
        if let Some(name) = x.strip_prefix("--") {
            if FLAGS.contains(&name) {
                a.flags.push(name.to_owned());
            } else {
                let v = it.next().ok_or(format!("--{name} needs a value"))?;
                if name == "revoke" {
                    a.many.push(v.clone());
                } else {
                    a.opts.insert(name.to_owned(), v.clone());
                }
            }
        } else {
            a.rest.push(x.clone());
        }
    }
    Ok(a)
}

impl Args {
    fn get(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }
    fn need(&self, k: &str) -> Result<&str, Fail> {
        self.get(k)
            .ok_or_else(|| Fail::Usage(format!("--{k} is required")))
    }
    fn file(&self) -> Result<&str, Fail> {
        match &self.rest[..] {
            [f] => Ok(f),
            _ => Err(Fail::Usage("exactly one FILE".into())),
        }
    }
    fn num(&self, k: &str) -> Result<Option<u64>, Fail> {
        self.get(k)
            .map(|v| {
                v.parse()
                    .map_err(|_| Fail::Usage(format!("--{k} is a number")))
            })
            .transpose()
    }
}

enum Fail {
    Usage(String),
    No(String),
}

fn no(e: impl std::fmt::Display) -> Fail {
    Fail::No(e.to_string())
}

fn read(path: &str, cap: u64) -> Result<Vec<u8>, Fail> {
    let f = std::fs::File::open(path).map_err(|e| no(format!("{path}: {e}")))?;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| no(format!("{path}: {e}")))?;
    if u64::try_from(buf.len()).unwrap_or(u64::MAX) > cap {
        return Err(no(format!("{path}: larger than {cap} bytes")));
    }
    Ok(buf)
}

/// The passphrase: `--passphrase-env NAME`, else one line from the terminal with echo off.
fn passphrase(a: &Args, prompt: &str) -> Result<zeroize::Zeroizing<String>, Fail> {
    if let Some(name) = a.get("passphrase-env") {
        return std::env::var(name)
            .map(zeroize::Zeroizing::new)
            .map_err(|_| no(format!("{name} is not set")));
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(no("no passphrase: give --passphrase-env NAME"));
    }
    eprint!("{prompt}: ");
    let echo = |on: bool| {
        let _ = std::process::Command::new("stty")
            .arg(if on { "echo" } else { "-echo" })
            .stdin(std::process::Stdio::inherit())
            .status();
    };
    echo(false);
    let mut line = zeroize::Zeroizing::new(String::new());
    let r = {
        use std::io::BufRead;
        std::io::stdin().lock().take(1024).read_line(&mut line)
    };
    echo(true);
    eprintln!();
    r.map_err(|e| no(format!("passphrase: {e}")))?;
    let p = line.trim_end_matches(['\n', '\r']).to_owned();
    Ok(zeroize::Zeroizing::new(p))
}

/// The key for `name` (the serial it signs as; None takes the name the file says).
fn key(a: &Args, name: Option<&str>) -> Result<KeyMaterial, Fail> {
    let text = zeroize::Zeroizing::new(match (a.get("key-env"), a.get("key-file")) {
        (Some(n), None) => std::env::var(n)
            .map_err(|_| no(format!("{n} is not set")))?
            .into_bytes(),
        (None, Some(f)) => read(f, 4096)?,
        _ => return Err(Fail::Usage("give --key-env NAME or --key-file F".into())),
    });
    match wolkey::name_of(&text) {
        Ok(stored) => {
            let want = name.unwrap_or(&stored).to_owned();
            let pass = passphrase(a, "Key passphrase")?;
            wolkey::unlock(&text, &want, &pass).map_err(|e| no(e.sentence()))
        }
        Err(_) => KeyMaterial::parse(&text).map_err(no),
    }
}

fn purpose(name: &str) -> Result<Purpose, Fail> {
    Ok(match name {
        "index" => Purpose::Index,
        "revocation" => Purpose::Revocation,
        "package" => Purpose::Package,
        "plugin" => Purpose::Plugin,
        "extension" => Purpose::Extension,
        _ => {
            return Err(Fail::Usage(
                "--purpose is index, revocation, package, plugin or extension".into(),
            ));
        }
    })
}

fn now(a: &Args) -> Result<u64, Fail> {
    Ok(a.num("now")?.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }))
}

fn sign(a: &Args) -> Result<Vec<u8>, Fail> {
    let p = purpose(a.need("purpose")?)?;
    let serial = a.need("cert")?;
    if !woltrust::cert::is_serial(serial) {
        return Err(Fail::Usage(
            "--cert is the certificate id (128 lowercase hex characters)".into(),
        ));
    }
    let content = read(a.file()?, 4 << 30)?;
    let secret = key(a, Some(serial))?.secret();
    match a.get("origin") {
        Some(f) => {
            let raw = read(f, 1024)?;
            woltrust::Origin::parse(&raw).map_err(no)?;
            Ok(make::wsig_with_origin(serial, &secret, p, &content, &raw))
        }
        None => Ok(make::wsig(serial, &secret, p, &content)),
    }
}

fn issue(a: &Args) -> Result<Vec<u8>, Fail> {
    let body_bytes = read(a.need("body")?, 64 * 1024)?;
    let (body, keys, _) = Body::parse(&body_bytes).map_err(no)?;
    // Rule "IDS": the serial is the id of the keys (Body::parse checked it); a 16-hex serial is
    // the TEST authority's only.
    if woltrust::cert::is_legacy_serial(&body.serial) && !a.flags.iter().any(|f| f == "allow-test")
    {
        return Err(no(
            "the serial must be the id of the body's keys (128 hex); 16 hex is for the test authority",
        ));
    }
    let km = key(a, Some(&body.issuer))?;
    if body.kind == Kind::Root && body.keys != make::keys_text(&km.secret()) {
        return Err(no(
            "a root signs its own body: the key is not the body's key",
        ));
    }
    // Audit 2 L6: with the issuer's certificate, the verifier's rule runs before anything is
    // signed (kind, usage sign-certs, namespaces inside the issuer's), and the key is the
    // certificate's.
    if let Some(f) = a.get("issuer-cert") {
        let issuer = woltrust::Cert::parse(&read(f, 64 * 1024)?).map_err(no)?;
        if issuer.body.serial != body.issuer {
            return Err(no(
                "--issuer-cert is not the certificate the body names as issuer",
            ));
        }
        if issuer.keys != *km.secret().public() {
            return Err(no("the key is not the key of --issuer-cert"));
        }
        if !woltrust::verify::may_issue_body(&issuer.body, &body) {
            return Err(no(
                "the issuer may not issue this certificate (kind, usage or namespaces)",
            ));
        }
    }
    match (a.get("proof"), body.kind) {
        (Some(f), _) => {
            let proof: request::Proof =
                serde_json::from_slice(&read(f, 64 * 1024)?).map_err(|_| no("proof damaged"))?;
            request::check(&body, &keys, &proof).map_err(no)?;
        }
        (None, Kind::Person | Kind::Org) => {
            return Err(no("a person or organisation certificate needs --proof"));
        }
        (None, _) => {}
    }
    let sig = km.secret().sign(Purpose::Cert, &body_bytes);
    // Every intermediate carries both roots' signatures (lead decision 2026-11-04 03:17Z).
    let cosigs = match (body.kind, a.get("cosign-file")) {
        (Kind::Intermediate, Some(f)) => {
            let text = zeroize::Zeroizing::new(read(f, 4096)?);
            let name = wolkey::name_of(&text).map_err(|e| no(e.sentence()))?;
            let pass = match a.get("cosign-passphrase-env") {
                Some(n) => zeroize::Zeroizing::new(
                    std::env::var(n).map_err(|_| no(format!("{n} is not set")))?,
                ),
                None => passphrase(a, "Second root's passphrase")?,
            };
            let other = wolkey::unlock(&text, &name, &pass).map_err(|e| no(e.sentence()))?;
            if !other.root || name == body.issuer {
                return Err(no("the second signature comes from the other root"));
            }
            vec![woltrust::cert::CoSig {
                issuer: name,
                sig: other.secret().sign(Purpose::Cert, &body_bytes),
            }]
        }
        (Kind::Intermediate, None) if !woltrust::cert::is_legacy_serial(&body.serial) => {
            return Err(no(
                "an intermediate needs both roots: give the other root's key with --cosign-file",
            ));
        }
        _ => Vec::new(),
    };
    Ok(woltrust::cert::encode_with(&body_bytes, &sig, &cosigs))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    format: u32,
    entries: Vec<Entry>,
}

fn revoke(a: &Args) -> Result<Vec<u8>, Fail> {
    let src: Source = serde_json::from_slice(&read(a.need("source")?, 1 << 20)?)
        .map_err(|_| no("revocations source damaged"))?;
    if src.format != 1 {
        return Err(no("revocations source: unknown format"));
    }
    let prev = match a.get("previous").filter(|p| Path::new(p).exists()) {
        Some(p) => {
            woltrust::revoked::parse(&read(p, 1 << 20)?)
                .map_err(no)?
                .serial
        }
        None => 0,
    };
    let t = now(a)?;
    let mut entries = src.entries;
    entries.sort_by(|x, y| x.at.cmp(&y.at).then_with(|| x.serial.cmp(&y.serial)));
    // Lists are valid 24 hours and republished hourly (lead decision "RKDB").
    let list = ListText {
        format: 1,
        serial: prev + 1,
        issued: t,
        expires: t + woltrust::rkdb::LIFETIME,
        entries,
    };
    let bytes = serde_json::to_vec(&list).map_err(no)?;
    woltrust::revoked::parse(&bytes).map_err(no)?;
    Ok(bytes)
}

fn certs_in(dir: Option<&str>) -> Result<Vec<Vec<u8>>, Fail> {
    let Some(dir) = dir else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| no(format!("{dir}: {e}")))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wolcert"))
        .collect();
    names.sort();
    for p in names.iter().take(64) {
        out.push(read(&p.to_string_lossy(), woltrust::MAX_CERT_FILE as u64)?);
    }
    Ok(out)
}

fn verify(a: &Args) -> Result<(Vec<u8>, bool), Fail> {
    let p = purpose(a.need("purpose")?)?;
    let content = read(a.file()?, 4 << 30)?;
    let wsig = read(a.need("wsig")?, 64 * 1024)?;
    let chain = a.get("chain").map(|c| read(c, 128 * 1024)).transpose()?;
    let roots = certs_in(Some(a.need("roots")?))?;
    let inter = certs_in(a.get("intermediates"))?;
    let lists = match a.get("list") {
        Some(l) => vec![(read(l, 1 << 20)?, read(&format!("{l}.wsig"), 64 * 1024)?)],
        None => Vec::new(),
    };
    let t = now(a)?;
    let store = Store::load(
        &roots,
        &inter,
        &lists,
        t,
        a.flags.iter().any(|f| f == "allow-test"),
    );
    let out = woltrust::verify(
        &store,
        &woltrust::Input {
            purpose: p,
            content: &content,
            app_id: a.get("id"),
            wsig: Some(&wsig),
            chain: chain.as_deref(),
            first_seen: a.num("first-seen")?,
            now: t,
        },
    );
    let ok = matches!(
        out.trust,
        woltrust::Trust::Wolos | woltrust::Trust::Trusted | woltrust::Trust::Origin
    );
    // The signature line (lead decision "SIGNATURE LINES"): with --line it is the whole output,
    // else it goes to stderr beside the report.
    let line = out.line.clone().unwrap_or_default();
    if a.flags.iter().any(|f| f == "line") {
        return Ok((format!("{line}\n").into_bytes(), ok));
    }
    if !line.is_empty() {
        eprintln!("{line}");
    }
    let report = out.report(woltrust::Verdict::Pass, Vec::new()).to_json();
    Ok((report.into_bytes(), ok))
}

fn random(buf: &mut [u8]) -> Result<(), Fail> {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .map_err(|e| no(format!("no random bytes: {e}")))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Fail> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| no(format!("{}: {e}", path.display())))
}

fn init_cmd(a: &Args) -> Result<Vec<u8>, Fail> {
    let dir = Path::new(a.need("out")?);
    if dir.exists() {
        return Err(no(format!("{} exists; give a new folder", dir.display())));
    }
    if a.flags.iter().any(|f| f == "rotate-intermediates") {
        return rotate_cmd(a, dir);
    }
    // Required, no default: the text each root carries, signed and public (who vouches).
    let subject_a = a.need("subject-a")?;
    let subject_b = a.need("subject-b")?;
    init::check_subject(subject_a).map_err(no)?;
    init::check_subject(subject_b).map_err(no)?;
    let pass = passphrase(a, "Backup passphrase (at least 8 characters)")?;
    if a.get("passphrase-env").is_none() {
        let again = passphrase(a, "Again")?;
        if *again != *pass {
            return Err(no("the two passphrases differ"));
        }
    }
    let mut failed = false;
    let mut rng = |b: &mut [u8]| failed |= random(b).is_err();
    let out =
        init::init(now(a)?, subject_a, subject_b, &pass, &mut rng).map_err(|e| no(e.sentence()))?;
    if failed {
        return Err(no("no random bytes"));
    }
    for sub in ["ci", "public/roots", "public/intermediates"] {
        std::fs::create_dir_all(dir.join(sub))
            .map_err(|e| no(format!("{}: {e}", dir.display())))?;
    }
    for m in &out.made {
        let sub = if m.role.starts_with("root") {
            "roots"
        } else {
            "intermediates"
        };
        let p = dir.join(format!("public/{sub}/{}-{}.wolcert", m.role, m.serial));
        std::fs::write(&p, &m.cert).map_err(|e| no(format!("{}: {e}", p.display())))?;
    }
    for (name, k) in &out.ci_keys {
        write_private(&dir.join("ci").join(name), k)?;
    }
    write_private(&dir.join("ci/passphrase"), out.ci_passphrase.as_bytes())?;
    write_authority(dir, &out)?;
    let d = dir.to_string_lossy();
    let mut text = init::instructions(&out, &d).join("\n");
    text.push('\n');
    Ok(text.into_bytes())
}

fn write_authority(dir: &Path, out: &init::Output) -> Result<(), Fail> {
    write_private(&dir.join("wolos-trust.wolbackup"), &out.backup)
}

fn rotate_cmd(a: &Args, dir: &Path) -> Result<Vec<u8>, Fail> {
    let backup = read(a.need("backup")?, init::MAX_BACKUP as u64)?;
    let root = a.need("root")?;
    let subject = a.need("subject-new")?;
    init::check_subject(subject).map_err(no)?;
    let pass = passphrase(a, "Backup passphrase")?;
    let mut failed = false;
    let mut rng = |b: &mut [u8]| failed |= random(b).is_err();
    let out = init::rotate(&backup, root, subject, &pass, now(a)?, &mut rng)
        .map_err(|e| no(e.sentence()))?;
    if failed {
        return Err(no("no random bytes"));
    }
    for sub in ["ci", "public/roots", "public/intermediates"] {
        std::fs::create_dir_all(dir.join(sub))
            .map_err(|e| no(format!("{}: {e}", dir.display())))?;
    }
    let mut lines = Vec::new();
    for m in &out.made {
        let sub = if m.role.starts_with("root") {
            "roots"
        } else {
            "intermediates"
        };
        let p = dir.join(format!("public/{sub}/{}-{}.wolcert", m.role, m.serial));
        std::fs::write(&p, &m.cert).map_err(|e| no(format!("{}: {e}", p.display())))?;
        if let Ok(c) = woltrust::Cert::parse(&m.cert) {
            lines.push(woltrust::line::cert(&c, &[]));
        }
    }
    for (name, k) in &out.ci_keys {
        write_private(&dir.join("ci").join(name), k)?;
    }
    write_private(&dir.join("ci/passphrase"), out.ci_passphrase.as_bytes())?;
    write_authority(dir, &out)?;
    lines.push(format!(
        "The new root and intermediates are in {}/public. Ship the new root in the next image, \
         replace the CI secrets (lines 1-3 of woltrust init), and re-issue every author \
         certificate with the new Authors CA.",
        dir.display()
    ));
    Ok((lines.join("\n") + "\n").into_bytes())
}

/// Root certificates of `--roots DIR`, by their role (`root-a`, `root-b`, from the label).
fn roots_by_role(dir: &str) -> Result<Vec<(String, woltrust::Cert)>, Fail> {
    let mut out = Vec::new();
    for raw in certs_in(Some(dir))? {
        let c = woltrust::Cert::parse(&raw).map_err(no)?;
        if c.body.kind != Kind::Root {
            continue;
        }
        let role = c.body.subject.label.as_deref().unwrap_or_default();
        let role = if role.ends_with("root-a") {
            "A"
        } else if role.ends_with("root-b") {
            "B"
        } else {
            continue;
        };
        out.push((role.to_owned(), c));
    }
    Ok(out)
}

/// `woltrust emergency`: the other root withdraws a root and/or intermediates, offline. Prints
/// each entry's line and the consequence, and signs only after each id was typed back.
fn emergency(a: &Args) -> Result<Vec<u8>, Fail> {
    let affected = a.need("root")?;
    if !matches!(affected, "A" | "B") {
        return Err(Fail::Usage(
            "--root is A or B: the root whose trust is cut".into(),
        ));
    }
    let reason = match a.need("reason")? {
        "key-compromise" => woltrust::Reason::KeyCompromise,
        "abuse" => woltrust::Reason::Abuse,
        _ => return Err(Fail::Usage("--reason is key-compromise or abuse".into())),
    };
    if a.many.is_empty() {
        return Err(Fail::Usage("name what to withdraw with --revoke ID".into()));
    }
    let roots = roots_by_role(a.need("roots")?)?;
    let other = if affected == "A" { "B" } else { "A" };
    let (_, signer) = roots
        .iter()
        .find(|(r, _)| r == other)
        .ok_or_else(|| no(format!("root {other} is not in --roots")))?;
    let target = roots.iter().find(|(r, _)| r == affected).map(|(_, c)| c);
    for id in &a.many {
        if !woltrust::cert::is_serial(id) {
            return Err(no(format!("{id}: not a certificate id")));
        }
        if *id == signer.body.serial {
            return Err(no("a root never withdraws itself"));
        }
    }
    let out_path = a.need("out")?;
    let km = key(a, Some(&signer.body.serial))?;
    if !km.root || km.secret().public() != &signer.keys {
        return Err(no(format!("the key is not root {other}'s")));
    }
    let t = now(a)?;
    let by = woltrust::line::ca_name(signer);
    let mut text = String::new();
    for id in &a.many {
        let what = match target {
            Some(r) if r.body.serial == *id => {
                format!("root {affected} ({})", woltrust::line::ca_name(r))
            }
            _ => format!("the certificate authority {id}"),
        };
        text.push_str(&woltrust::line::revocation(id, t, reason, &by));
        text.push('\n');
        eprintln!(
            "This withdraws {what}. Every certificate issued under it stops validating \
             everywhere, at once and for good; only a new image brings a replacement root. \
             Type the id to confirm:"
        );
        let mut typed = String::new();
        use std::io::BufRead as _;
        std::io::stdin()
            .lock()
            .take(512)
            .read_line(&mut typed)
            .map_err(|e| no(format!("stdin: {e}")))?;
        if typed.trim() != id {
            return Err(no("the id typed does not match; nothing was signed"));
        }
    }
    let entries: Vec<rkdb::Entry> = a
        .many
        .iter()
        .filter_map(|id| {
            Some(rkdb::Entry {
                id: rkdb::id_bytes(id)?,
                at: u32::try_from(t).ok()?,
                reason,
            })
        })
        .collect();
    let section = rkdb::RootSection::make(
        u32::try_from(t).map_err(no)?,
        &entries,
        &signer.body.serial,
        &km.secret(),
    )
    .map_err(|e| no(e.as_str()))?;
    write_private(Path::new(out_path), &section)?;
    text.push_str(&format!(
        "Signed by root {other}. Put {out_path} beside the next revoked.rkdb (rkdb make --root-section) and run wol trust update on a machine to check it.\n"
    ));
    Ok(text.into_bytes())
}

/// `rkdb make`: a new database from a source list (or `revoked.json`), on top of `--previous`
/// (never dropping an entry), with an optional root section, signed by the Index key.
fn rkdb_make(a: &Args) -> Result<Vec<u8>, Fail> {
    let src = read(a.need("source")?, 1 << 30)?;
    let list: Vec<woltrust::revoked::Entry> = match woltrust::revoked::parse(&src) {
        Ok(l) => l.entries,
        Err(_) => {
            let s: Source = serde_json::from_slice(&src).map_err(|_| no("source damaged"))?;
            s.entries
        }
    };
    let entries: Vec<rkdb::Entry> = list
        .iter()
        .map(|e| {
            Ok(rkdb::Entry {
                id: rkdb::id_bytes(&e.serial)
                    .ok_or_else(|| no(format!("{}: not an id", e.serial)))?,
                at: u32::try_from(e.at).map_err(no)?,
                reason: e.reason,
            })
        })
        .collect::<Result<_, Fail>>()?;
    let mut set = rkdb::Entries::from_list(&entries);
    let (serial, prev_hash) = match a.get("previous").filter(|p| Path::new(p).exists()) {
        Some(p) => {
            let old = rkdb::Db::parse(&read(p, 1 << 30)?).map_err(|e| no(e.as_str()))?;
            // Never dropping: the new file holds every entry of the previous one.
            set = rkdb::merge(&old.entries, &set);
            (old.serial + 1, old.hash)
        }
        None => (1, [0u8; 32]),
    };
    let root = a
        .get("root-section")
        .map(|f| read(f, 1 << 20))
        .transpose()?;
    let t = u32::try_from(now(a)?).map_err(no)?;
    let cert = a.need("cert")?;
    let secret = key(a, Some(cert))?.secret();
    rkdb::make(
        &rkdb::Make {
            serial,
            issued: t,
            expires: t.saturating_add(u32::try_from(rkdb::LIFETIME).map_err(no)?),
            prev_hash,
            entries: &set,
            root: root.as_deref(),
        },
        cert,
        &secret,
    )
    .map_err(|e| no(e.as_str()))
}

/// The revocation lines of one root section, by the root that signed it.
fn section_lines(text: &mut String, store: &Store, r: &woltrust::store::HeldRoot) {
    let by = store
        .roots
        .iter()
        .find(|c| c.body.serial == r.signer)
        .map_or_else(|| r.signer.clone(), woltrust::line::ca_name);
    for e in r.section.entries.iter() {
        text.push_str(&woltrust::line::revocation(
            &rkdb::id_text(&e.id),
            u64::from(e.at),
            e.reason,
            &by,
        ));
        text.push('\n');
    }
}

/// `rkdb verify`: checks a `.rkdb` (or a bare `.rkdr` root section) against the roots and prints
/// what it holds.
fn rkdb_verify(a: &Args) -> Result<Vec<u8>, Fail> {
    let bytes = read(one(a)?, 1 << 30)?;
    let roots = certs_in(Some(a.need("roots")?))?;
    let inter = certs_in(a.get("intermediates"))?;
    let t = now(a)?;
    let none: [(&[u8], &[u8]); 0] = [];
    let mut store = Store::load(
        &roots,
        &inter,
        &none,
        t,
        a.flags.iter().any(|f| f == "allow-test"),
    );
    store.floor = a.num("floor")?.unwrap_or(0);
    // A bare root section (`.rkdr`, `woltrust emergency`): checked on its own.
    if rkdb::kind_of(&bytes) == Some(rkdb::FileKind::RootSection) {
        store
            .load_root_section(&bytes, t)
            .map_err(|e| no(e.as_str()))?;
        let mut text = String::new();
        for r in &store.root_sections {
            text.push_str(&format!(
                "root section issued {} entries {}\n",
                woltrust::line::date(u64::from(r.section.issued)),
                r.section.entries.len()
            ));
            section_lines(&mut text, &store, r);
        }
        return Ok(text.into_bytes());
    }
    let (held, root) = store.check_rkdb(&bytes, t).map_err(|e| no(e.as_str()))?;
    rkdb::accept(&held.db, None, store.floor).map_err(|e| no(e.as_str()))?;
    let index = store
        .intermediates
        .iter()
        .find(|c| c.body.serial == held.signer)
        .map_or_else(|| "WolOS Index".to_owned(), woltrust::line::ca_name);
    let mut text = format!(
        "serial {} issued {} expires {} entries {}\n",
        held.db.serial,
        woltrust::line::date(u64::from(held.db.issued)),
        woltrust::line::date(u64::from(held.db.expires)),
        held.db.entries.len()
    );
    if let Some(r) = &root {
        section_lines(&mut text, &store, r);
    }
    for e in held.db.entries.iter().take(1000) {
        text.push_str(&woltrust::line::revocation(
            &rkdb::id_text(&e.id),
            u64::from(e.at),
            e.reason,
            &index,
        ));
        text.push('\n');
    }
    Ok(text.into_bytes())
}

fn rkdb_cmd(a: &Args) -> Result<(Vec<u8>, bool), Fail> {
    let ok = |b: Vec<u8>| (b, true);
    match a.rest.first().map(String::as_str) {
        Some("make") => rkdb_make(a).map(ok),
        Some("delta") => {
            let (b, t) = two(a)?;
            rkdb::delta(&read(b, 1 << 30)?, &read(t, 1 << 30)?)
                .map(ok)
                .map_err(|e| no(e.as_str()))
        }
        Some("apply") => {
            let (b, d) = two(a)?;
            rkdb::apply(&read(b, 1 << 30)?, &read(d, 1 << 30)?)
                .map(ok)
                .map_err(|e| no(e.as_str()))
        }
        Some("export") => {
            let db = rkdb::Db::parse(&read(one(a)?, 1 << 30)?).map_err(|e| no(e.as_str()))?;
            serde_json::to_vec(&db.to_list()).map(ok).map_err(no)
        }
        Some("checkpoint") => {
            let db = rkdb::Db::parse(&read(one(a)?, 1 << 30)?).map_err(|e| no(e.as_str()))?;
            Ok(ok((rkdb::Checkpoint::of(&db).to_json() + "\n").into_bytes()))
        }
        Some("verify") => rkdb_verify(a).map(ok),
        _ => Err(Fail::Usage(
            "rkdb make|delta|apply|verify|export|checkpoint".into(),
        )),
    }
}

fn witness_cmd(a: &Args) -> Result<(Vec<u8>, bool), Fail> {
    match a.rest.first().map(String::as_str) {
        Some("sign") => {
            let serial = a.need("cert")?;
            let file = read(one(a)?, 16 << 30)?;
            let km = key(a, Some(serial))?;
            Ok((
                woltrust::witness::sign(serial, &km.secret(), &file, now(a)?),
                true,
            ))
        }
        Some("verify") => {
            let (f, sig) = match &a.rest[1..] {
                [f] => (f.as_str(), format!("{f}.wsig")),
                [f, s] => (f.as_str(), s.clone()),
                _ => return Err(Fail::Usage("witness verify FILE [FILE.wsig]".into())),
            };
            let file = read(f, 16 << 30)?;
            let wsig = read(&sig, woltrust::MAX_WSIG as u64)?;
            let roots = certs_in(Some(a.need("roots")?))?;
            let inter = certs_in(a.get("intermediates"))?;
            let t = now(a)?;
            let none: [(&[u8], &[u8]); 0] = [];
            let store = Store::load(
                &roots,
                &inter,
                &none,
                t,
                a.flags.iter().any(|x| x == "allow-test"),
            );
            let out = woltrust::witness::verify(&store, &file, &wsig, t);
            let ok = out.trust == woltrust::Trust::Wolos;
            let text = match (&out.line, ok) {
                (Some(l), true) => format!("{l}\n"),
                _ => format!("not verified: {}\n", out.code.as_str()),
            };
            Ok((text.into_bytes(), ok))
        }
        _ => Err(Fail::Usage("witness sign|verify".into())),
    }
}

fn one(a: &Args) -> Result<&str, Fail> {
    match &a.rest[1..] {
        [f] => Ok(f),
        _ => Err(Fail::Usage("give one file".into())),
    }
}

fn two(a: &Args) -> Result<(&str, &str), Fail> {
    match &a.rest[1..] {
        [x, y] => Ok((x, y)),
        _ => Err(Fail::Usage("give two files".into())),
    }
}

fn extract(a: &Args) -> Result<Vec<u8>, Fail> {
    let b = read(a.need("backup")?, 256 * 1024)?;
    let (_, key, cert) = init::extract(&b, a.need("role")?).map_err(|e| no(e.sentence()))?;
    Ok(if a.flags.iter().any(|f| f == "certificate") {
        cert
    } else {
        key
    })
}

fn keygen(a: &Args) -> Result<Vec<u8>, Fail> {
    let mut seed = [0u8; 32];
    random(&mut seed)?;
    let km = KeyMaterial::new(seed, a.flags.iter().any(|f| f == "root"));
    seed.fill(0);
    Ok(km.to_text().to_vec())
}

fn public(a: &Args) -> Result<Vec<u8>, Fail> {
    serde_json::to_vec(&key(a, None)?.keys_text()).map_err(no)
}

/// FILE, or stdin for `-`, up to `cap` bytes.
fn input(a: &Args, cap: u64) -> Result<Vec<u8>, Fail> {
    let f = a.file()?;
    if f != "-" {
        return read(f, cap);
    }
    let mut buf = Vec::new();
    std::io::stdin()
        .lock()
        .take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| no(format!("stdin: {e}")))?;
    if u64::try_from(buf.len()).unwrap_or(u64::MAX) > cap {
        return Err(no(format!("stdin: larger than {cap} bytes")));
    }
    Ok(buf)
}

fn label(a: &Args) -> &str {
    a.get("label").unwrap_or(woltrust::seal::FILE_LABEL)
}

/// The encryption keys and serial `seal --to F` seals to: a certificate (its serial), or the keys
/// `woltrust public` prints (the key id of its Ed25519 key, as `open` assumes by default).
fn recipient(path: &str) -> Result<(woltrust::crypto::EncPublic, String), Fail> {
    use woltrust::crypto::{ED_PK, EncPublic, KEM_PK, X_PK};
    let raw = read(path, woltrust::MAX_CERT_FILE as u64)?;
    if let Ok(c) = woltrust::Cert::parse(&raw) {
        let enc = c
            .enc
            .ok_or_else(|| no(woltrust::seal::SealError::NoEncryptionKey.sentence()))?;
        return Ok((enc, c.body.serial));
    }
    let k: woltrust::cert::KeysText = serde_json::from_slice(&raw)
        .map_err(|_| no(format!("{path}: not a certificate or public keys")))?;
    let bad = || no(format!("{path}: encryption keys missing or malformed"));
    let ed: [u8; ED_PK] = woltrust::b64::decode(&k.ed25519, ED_PK)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    let x: [u8; X_PK] = k
        .x25519
        .as_deref()
        .and_then(|s| woltrust::b64::decode(s, X_PK))
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    let m: [u8; KEM_PK] = k
        .ml_kem_768
        .as_deref()
        .and_then(|s| woltrust::b64::decode(s, KEM_PK))
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    let enc = EncPublic {
        x25519: x,
        ml_kem_768: Box::new(m),
    };
    let _ = ed;
    // Rule "IDS": the recipient is named by the id of all its public keys.
    let keys = woltrust::cert::decode_keys(&k).map_err(|_| bad())?;
    Ok((enc.clone(), woltrust::cert::id_of(&keys, Some(&enc))))
}

fn seal_cmd(a: &Args) -> Result<Vec<u8>, Fail> {
    let (to, serial) = recipient(a.need("to")?)?;
    let plain = zeroize::Zeroizing::new(input(a, woltrust::seal::MAX_PLAINTEXT as u64)?);
    let mut rnd = zeroize::Zeroizing::new([0u8; 64]);
    random(&mut rnd[..])?;
    woltrust::seal::seal_to(&to, &serial, label(a), &plain, &rnd).map_err(|e| no(e.sentence()))
}

fn open_cmd(a: &Args) -> Result<Vec<u8>, Fail> {
    a.file()?;
    let k = key(a, None)?;
    let sealed = input(a, (woltrust::seal::MAX_PLAINTEXT + 4096) as u64)?;
    // Without --serial: the id of the key (wolos/seal/2), or its 16-hex key id of before rule
    // "IDS" when the message names that (wolos/seal/1).
    let serial = match a.get("serial") {
        Some(s) => s.to_owned(),
        None => {
            let legacy = request::key_id(&k.secret().public().ed25519);
            match woltrust::seal::peek(&sealed) {
                Ok(h) if h.recipient == legacy => legacy,
                _ => k.key_id(),
            }
        }
    };
    woltrust::seal::open(&k.enc(), &serial, label(a), &sealed)
        .map(|p| p.to_vec())
        .map_err(|e| no(e.sentence()))
}

fn run(argv: &[String]) -> Result<(Vec<u8>, bool), Fail> {
    let Some((cmd, rest)) = argv.split_first() else {
        return Err(Fail::Usage(
            "init|sign|issue|revoke|verify|keygen|public|extract|seal|open|emergency|rkdb|witness"
                .into(),
        ));
    };
    let a = parse_args(rest).map_err(Fail::Usage)?;
    let ok = |b: Vec<u8>| (b, true);
    match cmd.as_str() {
        "sign" => sign(&a).map(ok),
        "issue" => issue(&a).map(ok),
        "revoke" => revoke(&a).map(ok),
        "verify" => verify(&a),
        "keygen" => keygen(&a).map(ok),
        "public" => public(&a).map(ok),
        "init" => init_cmd(&a).map(ok),
        "extract" => extract(&a).map(ok),
        "seal" => seal_cmd(&a).map(ok),
        "open" => open_cmd(&a).map(ok),
        "emergency" => emergency(&a).map(ok),
        "rkdb" => rkdb_cmd(&a),
        "witness" => witness_cmd(&a),
        _ => Err(Fail::Usage(format!("unknown command {cmd}"))),
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&argv) {
        Ok((out, good)) => {
            let w = std::io::stdout()
                .write_all(&out)
                .and_then(|()| std::io::stdout().flush());
            if w.is_err() {
                1
            } else if good {
                0
            } else {
                1
            }
        }
        Err(Fail::Usage(m)) => {
            eprintln!("woltrust: {m}");
            eprintln!(
                "usage: woltrust init|sign|issue|revoke|verify|keygen|public|extract|seal|open|emergency|rkdb|witness ... (see the crate docs)"
            );
            2
        }
        Err(Fail::No(m)) => {
            eprintln!("woltrust: {m}");
            1
        }
    };
    std::process::exit(code);
}
