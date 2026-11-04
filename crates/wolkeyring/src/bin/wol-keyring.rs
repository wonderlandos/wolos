// SPDX-License-Identifier: BSD-2-Clause
//! `wol-keyring` (`wol keyring` in the command tree): the WolOS keyring at the command line
//! (ADR 0054). It opens the store itself, under the same write lock as wol-keyringd, so both see
//! each other's writes. Secrets and passwords come on stdin (or the terminal with echo off),
//! never as arguments.
//!
//! Exit codes: 0 done, 1 refused or failed (the line on stderr says why), 2 usage.
use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;
use zeroize::Zeroizing;

use wolkeyring::audit::{self, Event};
use wolkeyring::bundle::{self, PublicId, To, With};
use wolkeyring::factor::Kind;
use wolkeyring::item::Class;
use wolkeyring::kdf::Kdf;
use wolkeyring::pairing::{self, Receiver, Sender};
use wolkeyring::rand::OsRandom;
use wolkeyring::service::Session;
use wolkeyring::{Error, io};

const USAGE: &str = "usage: wol keyring [--name NAME] VERB
  init                       make the keyring (password on stdin; prints the recovery code once)
  status                     locked or not, items, factors, policy
  id                         this keyring's public identity (give it to whoever exports to you)
  list                       items (label, attributes; no secrets)
  set LABEL ATTR=VALUE...    store a secret (password line, then the secret, on stdin)
  get ATTR=VALUE...          print the secret of the item with these attributes
  delete ATTR=VALUE...       delete the item with these attributes
  export FILE [--to IDFILE | IDFILE | --passphrase]   a signed, encrypted bundle of every item
  import FILE                read a bundle (merges; newer wins)
  move receive [--port N | N]  receive a whole keyring from another WolOS machine on this network
  move send HOST[:PORT]      send this keyring to a machine running 'move receive'
  audit [verify] [--last N | N]  the access log; verify checks its chain, its signatures and the rollback record
  audit accept               after a restore from a backup: take the files as they are as the new baseline
  acl list ATTR=VALUE...     the apps on an item's list
  acl allow EXE [ATTR=VALUE...]   put a program on the list of one item, or of every item without attributes (its digest is taken now)
  acl revoke EXE [ATTR=VALUE...]  take it off
  migrate                    copy every item from the old keyring on the Secret Service (gnome-keyring) into this one
  enrol recovery|pin         add a new recovery code (replaces the old) or a PIN (with the chip)
  compact                    rewrite the store without replaced records";

struct Args {
    name: String,
    rest: Vec<String>,
}

fn args() -> Option<Args> {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let mut name = "login".to_string();
    if a.first().is_some_and(|x| x == "--name") {
        a.remove(0);
        if a.is_empty() {
            return None;
        }
        name = a.remove(0);
    }
    (!a.is_empty()).then_some(Args { name, rest: a })
}

fn tty() -> bool {
    std::fs::metadata("/proc/self/fd/0")
        .ok()
        .and_then(|_| std::fs::read_link("/proc/self/fd/0").ok())
        .is_some_and(|p| {
            p.to_string_lossy().starts_with("/dev/pts")
                || p.to_string_lossy().starts_with("/dev/tty")
        })
}

/// One line from stdin; on a terminal the prompt is shown and echo is off while typing.
fn ask(prompt: &str) -> Result<Zeroizing<String>, Error> {
    let on_tty = tty();
    if on_tty {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        let _ = std::process::Command::new("stty")
            .arg("-echo")
            .stdin(std::process::Stdio::inherit())
            .status();
    }
    let mut line = Zeroizing::new(String::new());
    let r = std::io::stdin().lock().read_line(&mut line);
    if on_tty {
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .stdin(std::process::Stdio::inherit())
            .status();
        eprintln!();
    }
    r.map_err(|e| Error::Io(format!("stdin: {e}")))?;
    let t = line.trim_end_matches(['\n', '\r']).to_string();
    Ok(Zeroizing::new(t))
}

fn attrs(list: &[String]) -> Result<BTreeMap<String, String>, Error> {
    list.iter()
        .map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or(Error::Malformed("ATTR=VALUE"))
        })
        .collect()
}

fn open_unlocked(name: &str) -> Result<Session, Error> {
    let mut s = Session::open(name)?;
    let pw = ask("Password: ")?;
    s.unlock(
        &[(Kind::Password, pw.as_str()), (Kind::Recovery, pw.as_str())],
        false,
        "wol-keyring",
    )?;
    for w in &s.warnings {
        eprintln!("WARNING: {w}");
    }
    Ok(s)
}

fn one(s: &Session, q: &BTreeMap<String, String>) -> Result<String, Error> {
    let hits = s.keyring.search(q);
    match hits.as_slice() {
        [m] => Ok(m.id.clone()),
        [] => Err(Error::NotFound),
        _ => Err(Error::Malformed(
            "more than one item has these attributes; give more",
        )),
    }
}

fn payload_of(s: &Session) -> Result<bundle::Payload, Error> {
    s.keyring.export_payload(io::now(), None)
}

fn frame_write(t: &mut TcpStream, b: &[u8]) -> Result<(), Error> {
    let n = u32::try_from(b.len()).map_err(|_| Error::Limit("message"))?;
    t.write_all(&n.to_be_bytes())
        .and_then(|()| t.write_all(b))
        .map_err(|e| Error::Io(format!("network: {e}")))
}

fn frame_read(t: &mut TcpStream, max: usize) -> Result<Vec<u8>, Error> {
    let mut n = [0u8; 4];
    t.read_exact(&mut n)
        .map_err(|e| Error::Io(format!("network: {e}")))?;
    let n = u32::from_be_bytes(n) as usize;
    if n > max {
        return Err(Error::Limit("message"));
    }
    let mut v = vec![0u8; n];
    t.read_exact(&mut v)
        .map_err(|e| Error::Io(format!("network: {e}")))?;
    Ok(v)
}

fn confirm(sas: &pairing::Sas) -> Result<(), Error> {
    eprintln!(
        "Check that the other computer shows the same code: {}",
        sas.0
    );
    let a = ask("Same code on both screens? Type yes: ")?;
    if a.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(Error::Policy(
            "the codes were not confirmed; nothing was sent",
        ))
    }
}

fn run(a: Args) -> Result<(), Error> {
    let verb: Vec<&str> = a.rest.iter().map(String::as_str).collect();
    let name = a.name.as_str();
    match verb.as_slice() {
        ["init"] => {
            let pw = ask("New password: ")?;
            if tty() {
                let again = ask("Again: ")?;
                if *again != *pw {
                    return Err(Error::Policy("the two passwords differ"));
                }
            }
            let (s, code) = Session::create(name, &pw)?;
            println!(
                "Keyring '{}' made: {} ways to unlock.",
                s.name,
                s.keyring.header().rules.len()
            );
            println!(
                "Recovery code (shown once, write it down): {}",
                code.as_str()
            );
        }
        ["status"] => {
            let s = Session::open(name)?;
            let h = s.keyring.header();
            println!("keyring {} format {} epoch {}", h.name, h.format, h.epoch);
            for f in &h.factors {
                println!("factor {} {:?} {}", f.id, f.kind, f.label);
            }
            for r in &h.rules {
                println!("rule {} needs factors {:?}", r.id, r.factors);
            }
            println!(
                "lock after {} s, on sleep {}, with screen {}",
                h.settings.lock_after_secs, h.settings.lock_on_sleep, h.settings.lock_with_screen
            );
        }
        ["id"] => {
            let s = open_unlocked(name)?;
            let p = s.keyring.identity()?.public();
            println!("{}", p.to_line());
            eprintln!("fingerprint {}", p.fingerprint_text());
        }
        ["list"] => {
            let s = open_unlocked(name)?;
            for m in s.keyring.all() {
                let at: Vec<String> = m.attrs.iter().map(|(k, v)| format!("{k}={v}")).collect();
                println!(
                    "{}  {}  {}  {:?}{}",
                    m.id,
                    m.label,
                    at.join(" "),
                    m.class,
                    if m.device_only {
                        " this-device-only"
                    } else {
                        ""
                    }
                );
            }
        }
        ["set", label, rest @ ..] => {
            let q = attrs(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
            let mut s = open_unlocked(name)?;
            let secret = ask("Secret: ")?;
            let id = s.set(None, label, q, secret.as_bytes(), Class::WhenUnlocked)?;
            println!("{id}");
        }
        ["get", rest @ ..] => {
            let q = attrs(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
            let mut s = open_unlocked(name)?;
            let id = one(&s, &q)?;
            let v = s.get(None, &id)?;
            std::io::stdout()
                .write_all(&v)
                .map_err(|e| Error::Io(e.to_string()))?;
            println!();
        }
        ["delete", rest @ ..] => {
            let q = attrs(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
            let mut s = open_unlocked(name)?;
            let id = one(&s, &q)?;
            s.delete(None, &id)?;
        }
        ["export", file, how @ ..] => {
            let mut s = open_unlocked(name)?;
            let p = payload_of(&s)?;
            let id = s.keyring.identity()?;
            let mut rng = OsRandom;
            let b = match how {
                ["--to", idfile] | [idfile]
                    if *idfile != "--passphrase" && !idfile.starts_with("--") =>
                {
                    let text = std::fs::read_to_string(idfile)
                        .map_err(|e| Error::Io(format!("{idfile}: {e}")))?;
                    let to = PublicId::from_line(&text)?;
                    eprintln!("Encrypting to {}", to.fingerprint_text());
                    bundle::seal(&mut rng, &id, &p, To::Recipient(&to), b"")?
                }
                ["--passphrase"] => {
                    let pass = ask("Passphrase for the file (12 characters or more): ")?;
                    bundle::seal(
                        &mut rng,
                        &id,
                        &p,
                        To::Passphrase(&pass, Kdf::default_new()),
                        b"",
                    )?
                }
                _ => return Err(Error::Malformed("export FILE --to IDFILE | --passphrase")),
            };
            io::replace(std::path::Path::new(file), &b)?;
            s.audit(
                Event::Export,
                Some("wol-keyring"),
                None,
                Some(&format!("{} items", p.items.len())),
            )?;
            println!(
                "{} items exported, signed by {}",
                p.items.len(),
                wolkeyring::bundle::group_hex(&id.public().fingerprint())
            );
        }
        ["import", file] => {
            let b =
                io::read(std::path::Path::new(file), bundle::MAX_BUNDLE)?.ok_or(Error::NotFound)?;
            let mut s = open_unlocked(name)?;
            let opened = if bundle::needs_passphrase(&b)? {
                let pass = ask("Passphrase of the file: ")?;
                bundle::open(&b, With::Passphrase(&pass), b"")?
            } else {
                let id = s.keyring.identity()?;
                bundle::open(&b, With::Identity(&id), b"")?
            };
            let signer = opened.signer_text();
            eprintln!("Signed by {signer} (keyring '{}')", opened.payload.from);
            let (add, rep, kept) = s.import(&opened.payload, &signer, Event::Import)?;
            println!("{add} added, {rep} replaced, {kept} kept");
        }
        ["move", "receive", rest @ ..] => {
            let port = match rest {
                ["--port", p] | [p] => p.parse().map_err(|_| Error::Malformed("port"))?,
                [] => pairing::PORT,
                _ => return Err(Error::Malformed("move receive [--port N]")),
            };
            let mut s = open_unlocked(name)?;
            let l = TcpListener::bind(("0.0.0.0", port))
                .map_err(|e| Error::Io(format!("port {port}: {e}")))?;
            eprintln!(
                "Waiting on port {port}. On the other computer run: wol keyring move send THIS-COMPUTER:{port}"
            );
            let (mut t, peer) = l.accept().map_err(|e| Error::Io(e.to_string()))?;
            drop(l);
            let d = Some(Duration::from_secs(pairing::DEADLINE_SECS));
            let _ = t.set_read_timeout(d);
            let _ = t.set_write_timeout(d);
            eprintln!("Connected from {peer}");
            let mut rng = OsRandom;
            let (mut r, m1) = Receiver::start(&mut rng)?;
            frame_write(&mut t, &m1)?;
            let m2 = frame_read(&mut t, 64)?;
            let (m3, sas) = r.reveal(&m2)?;
            frame_write(&mut t, &m3)?;
            confirm(&sas)?;
            let b = frame_read(&mut t, bundle::MAX_BUNDLE)?;
            let opened = r.open(&b)?;
            let signer = opened.signer_text();
            eprintln!("Signed by {signer} (keyring '{}')", opened.payload.from);
            let (add, rep, kept) = s.import(&opened.payload, &signer, Event::MoveIn)?;
            println!("{add} added, {rep} replaced, {kept} kept");
        }
        ["move", "send", host] => {
            let addr = if host.contains(':') {
                host.to_string()
            } else {
                format!("{host}:{}", pairing::PORT)
            };
            let mut s = open_unlocked(name)?;
            let p = payload_of(&s)?;
            let mut t = TcpStream::connect(&addr).map_err(|e| Error::Io(format!("{addr}: {e}")))?;
            let d = Some(Duration::from_secs(pairing::DEADLINE_SECS));
            let _ = t.set_read_timeout(d);
            let _ = t.set_write_timeout(d);
            let mut rng = OsRandom;
            let m1 = frame_read(&mut t, 128)?;
            let (mut snd, m2) = Sender::start(&mut rng, &m1)?;
            frame_write(&mut t, &m2)?;
            let m3 = frame_read(&mut t, 8192)?;
            let sas = snd.check(&m3)?;
            confirm(&sas)?;
            let id = s.keyring.identity()?;
            let b = snd.seal(&mut rng, &id, &p)?;
            frame_write(&mut t, &b)?;
            s.audit(
                Event::MoveOut,
                Some("wol-keyring"),
                None,
                Some(&format!("{} items to {addr}", p.items.len())),
            )?;
            println!("{} items sent", p.items.len());
        }
        ["audit", "accept"] => {
            let mut s = open_unlocked(name)?;
            s.accept_rollback()?;
            println!("the files as they are now are the new baseline");
        }
        ["audit", rest @ ..] => {
            let verify = rest.first() == Some(&"verify");
            let last: usize = match rest.iter().position(|x| *x == "--last") {
                Some(i) => rest
                    .get(i + 1)
                    .and_then(|n| n.parse().ok())
                    .ok_or(Error::Malformed("--last N"))?,
                None => rest.iter().find_map(|x| x.parse().ok()).unwrap_or(50),
            };
            let s = if verify {
                open_unlocked(name)?
            } else {
                Session::open(name)?
            };
            let text = s.audit_text()?;
            if verify {
                let id = s.keyring.identity()?.public();
                let (epoch, records, _) = s.keyring.position();
                let r = audit::verify(&text, &id.sign, Some((epoch, records)))?;
                println!(
                    "{} lines, chain intact; last signed checkpoint {}; {} lines after it{}",
                    r.lines,
                    r.last_signed
                        .map_or("none".into(), |n| format!("at line {n}")),
                    r.unsigned_after,
                    if r.rollback {
                        "; WARNING: the store is older than its last checkpoint (rolled back)"
                    } else {
                        ""
                    }
                );
                if r.rollback || !s.warnings.is_empty() {
                    return Err(Error::Damaged("the store or the audit log was rolled back"));
                }
            } else {
                let lines = audit::parse(&text)?;
                for l in lines.iter().rev().take(last).rev() {
                    println!(
                        "{} {:>4} {:<14} {} {} {}",
                        l.time,
                        l.seq,
                        serde_json::to_value(l.event)
                            .ok()
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        l.app.as_deref().unwrap_or("-"),
                        l.item.as_deref().unwrap_or("-"),
                        l.detail.as_deref().unwrap_or("")
                    );
                }
            }
        }
        ["migrate"] => {
            let found = wolkeyring::migrate::read_all()?;
            let mut s = open_unlocked(name)?;
            let (mut copied, mut skipped) = (0, 0);
            for f in &found {
                // A secret too large or text too long for this store is reported, not cut.
                match s.set(
                    None,
                    &f.label,
                    f.attrs.clone(),
                    &f.secret,
                    Class::WhenUnlocked,
                ) {
                    Ok(_) => copied += 1,
                    Err(e) => {
                        skipped += 1;
                        eprintln!("skipped '{}': {e}", f.label);
                    }
                }
            }
            s.audit(
                Event::Migrate,
                Some("wol-keyring"),
                None,
                Some(&format!("{copied} items copied, {skipped} skipped")),
            )?;
            println!("{copied} items copied, {skipped} skipped; the old keyring is untouched");
            if copied + skipped != found.len() || skipped > 0 {
                return Err(Error::Limit(
                    "some items were not copied (see above); keep the old keyring",
                ));
            }
        }
        ["acl", "list", rest @ ..] => {
            let q = attrs(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
            let s = open_unlocked(name)?;
            let id = one(&s, &q)?;
            let m = s.keyring.meta(&id).ok_or(Error::NotFound)?;
            for a in &m.acl {
                println!(
                    "{}  {}  {}",
                    a.display(),
                    a.exe,
                    a.sha256.as_deref().unwrap_or("no digest")
                );
            }
            if m.acl.is_empty() {
                println!("(owner only: no app is on the list; put one there with 'acl allow')");
            }
        }
        ["acl", verb @ ("allow" | "revoke"), exe, rest @ ..] => {
            let q = attrs(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
            let mut s = open_unlocked(name)?;
            // With attributes: the one item they name. Without: every item.
            let ids: Vec<String> = if q.is_empty() {
                s.keyring.all().map(|m| m.id.clone()).collect()
            } else {
                vec![one(&s, &q)?]
            };
            let canon = std::fs::canonicalize(exe).map_err(|e| Error::Io(format!("{exe}: {e}")))?;
            let exe_s = canon
                .to_str()
                .ok_or(Error::Malformed("program path"))?
                .to_string();
            let sha256 = io::sha256_file(&canon);
            for id in &ids {
                let mut acl = s.keyring.meta(id).ok_or(Error::NotFound)?.acl.clone();
                acl.retain(|a| a.exe != exe_s);
                if *verb == "allow" {
                    acl.push(wolkeyring::item::AppId {
                        exe: exe_s.clone(),
                        sha256: sha256.clone(),
                        app: None,
                        signer: None,
                    });
                }
                s.set_acl(id, acl)?;
            }
            println!(
                "{} {exe_s} on {} items",
                if *verb == "allow" {
                    "allowed"
                } else {
                    "revoked"
                },
                ids.len()
            );
        }
        ["enrol", what] => {
            let mut s = Session::open(name)?;
            let pw = ask("Password: ")?;
            s.unlock(&[(Kind::Password, pw.as_str())], false, "wol-keyring")?;
            match *what {
                "recovery" => {
                    let code = s.enrol(Kind::Recovery, None, &[(Kind::Password, pw.as_str())])?;
                    if let Some(c) = code {
                        println!(
                            "New recovery code (shown once; the old one no longer works): {}",
                            c.as_str()
                        );
                    }
                }
                "pin" => {
                    let pin = ask("New PIN (6 digits or more): ")?;
                    s.enrol(Kind::Pin, Some(&pin), &[(Kind::Password, pw.as_str())])?;
                    println!("PIN added; it unlocks together with this computer's security chip.");
                }
                _ => return Err(Error::Malformed("enrol recovery|pin")),
            }
        }
        ["compact"] => {
            let mut s = open_unlocked(name)?;
            s.compact()?;
            println!("compacted: {} items", s.keyring.len());
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn main() {
    let Some(a) = args() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    if let Err(e) = run(a) {
        eprintln!("wol keyring: {e}");
        std::process::exit(1);
    }
}
