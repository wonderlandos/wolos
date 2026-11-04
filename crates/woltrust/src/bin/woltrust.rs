// SPDX-License-Identifier: BSD-2-Clause
//! `woltrust`: the command line the hub pipeline calls (hub-index README, "Tooling contract"),
//! over the pure crate. Output goes to stdout, a reason to stderr; exit 0 on success, 1 on a
//! refusal, 2 on a usage error. Key material comes from `--key-env NAME` (a CI secret) or
//! `--key-file F`, never from an argument: either a `.wolkey` (locked; the passphrase comes from
//! `--passphrase-env NAME`, else a prompt on the terminal) or plain key material.
//!
//! ```text
//! woltrust sign   --purpose index|revocation|package|plugin|extension --cert SERIAL [--origin ORIGIN.json] KEY FILE
//! woltrust issue  --body BODY.json [--proof PROOF.json] KEY
//! woltrust revoke --source revocations.json [--previous revoked.json] [--now UNIX]
//! woltrust verify --purpose P --wsig F [--chain F] --roots DIR [--intermediates DIR]
//!                 [--list revoked.json] [--id APP_ID] [--first-seen UNIX] [--now UNIX] [--allow-test] FILE
//! woltrust keygen [--root]
//! woltrust public KEY
//! woltrust init --out DIR [--passphrase-env NAME] [--now UNIX]
//! woltrust extract --backup FILE --role root-a|root-b|authors-ca|packages-ca|index [--certificate]
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
use woltrust::{LIST_LIFETIME, Store, init, make, request, wolkey};

const FLAGS: [&str; 3] = ["allow-test", "root", "certificate"];

struct Args {
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
    rest: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
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
                a.opts.insert(name.to_owned(), v.clone());
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
        return Err(Fail::Usage("--cert is 16 lowercase hex characters".into()));
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
    let km = key(a, Some(&body.issuer))?;
    if body.kind == Kind::Root && body.keys != make::keys_text(&km.secret()) {
        return Err(no(
            "a root signs its own body: the key is not the body's key",
        ));
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
    Ok(woltrust::cert::encode(&body_bytes, &sig))
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
    let list = ListText {
        format: 1,
        serial: prev + 1,
        issued: t,
        expires: t + LIST_LIFETIME,
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
        out.push(read(&p.to_string_lossy(), 24 * 1024)?);
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
        woltrust::Trust::Wolos | woltrust::Trust::Verified
    );
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
    let pass = passphrase(a, "Backup passphrase (at least 8 characters)")?;
    if a.get("passphrase-env").is_none() {
        let again = passphrase(a, "Again")?;
        if *again != *pass {
            return Err(no("the two passphrases differ"));
        }
    }
    let mut failed = false;
    let mut rng = |b: &mut [u8]| failed |= random(b).is_err();
    let out = init::init(now(a)?, &pass, &mut rng).map_err(|e| no(e.sentence()))?;
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
    write_private(&dir.join("wolos-trust.wolbackup"), &out.backup)?;
    let mut text = init::instructions(&out, &dir.to_string_lossy()).join("\n");
    text.push('\n');
    Ok(text.into_bytes())
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

fn run(argv: &[String]) -> Result<(Vec<u8>, bool), Fail> {
    let Some((cmd, rest)) = argv.split_first() else {
        return Err(Fail::Usage(
            "init|sign|issue|revoke|verify|keygen|public|extract".into(),
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
                "usage: woltrust init|sign|issue|revoke|verify|keygen|public|extract ... (see the crate docs)"
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
