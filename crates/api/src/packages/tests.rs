// SPDX-License-Identifier: GPL-2.0-or-later
//! wold's package changes against a fake system.
use super::*;
use shared::patience::Patience;

/// The refusal leads to a real row: the section title Privacy & Security shows for the
/// grant, and the app's name (MANUAL 0.12.0: it named a "Software" section that is not there).
#[test]
fn a_refusal_names_the_privacy_row() {
    for (grant, section) in [
        ("packages-install", "Install Apps"),
        ("packages-remove", "Remove Apps"),
        ("packages-update", "System Updates"),
        ("packages-unreviewed", "Unreviewed Apps"),
    ] {
        let a = denied(grant, "/usr/bin/wopkg");
        assert_eq!(a.code, Some(Code::PermissionDenied));
        let m = a.message.unwrap_or_default();
        assert!(
            m.ends_with(&format!(
                "Turn on wopkg under {section} in System Settings > Privacy & Security."
            )),
            "{m}"
        );
        assert!(!m.contains("Software"), "{m}");
    }
}

#[test]
fn a_pinned_package_is_held_through_a_full_update() {
    // Audit S2: holds are an administrator sheet; wol-admind runs `-Syu --ignore=…`.
    let call = PackageCall::UpdateHolding {
        hold: vec!["firefox".into(), "gimp".into()],
    };
    assert_eq!(
        request(&call, None),
        ApprovalRequest::Packages {
            verb: "update-holding".into(),
            names: vec!["firefox".into(), "gimp".into()]
        }
    );
    let none = PackageCall::UpdateHolding { hold: vec![] };
    assert_eq!(
        request(&none, None),
        ApprovalRequest::Packages {
            verb: "update".into(),
            names: vec![]
        }
    );
    let bad = PackageCall::UpdateHolding {
        hold: vec!["--root=/x".into()],
    };
    assert!(bad.validate().is_err());
    assert_eq!(bad.grant(), "packages-update");
}
use std::cell::RefCell;

#[derive(Default)]
struct Fake {
    ran: RefCell<Vec<Vec<String>>>,
    seed: [u8; 32],
    running: Option<String>,
    /// The sheet's answer; None = no session answered.
    sheet: Option<Sheet>,
    asked: RefCell<Vec<Approval>>,
    discarded: RefCell<Vec<String>>,
    staging: Staging,
    /// Exit code of every run (0 = it worked).
    code: i32,
    /// Why the staged copy cannot be read (None: it reads as [`facts`]).
    unreadable: Option<String>,
    /// Set: the disk has no room for the copy (the refusal shown).
    no_room: Option<String>,
    /// How many times the fake was asked to copy a file.
    staged_calls: std::cell::Cell<u32>,
}

/// What the fake reads from every staged copy.
fn facts() -> PkgFacts {
    PkgFacts {
        name: "x".into(),
        version: "1-1".into(),
        sha256: "ab".repeat(32),
    }
}

/// The sheet for the fake's staged copy of `/home/a/x-1-1-x86_64.pkg.tar.zst`.
fn file_sheet() -> ApprovalRequest {
    let f = facts();
    ApprovalRequest::PackageFile {
        staged: format!("{STAGING}/n/x-1-1-x86_64.pkg.tar.zst"),
        name: f.name,
        version: f.version,
        sha256: f.sha256,
    }
}

fn yes(seed: [u8; 32]) -> Fake {
    Fake {
        seed,
        sheet: Some(Sheet::AllowOnce),
        ..Default::default()
    }
}
impl System for Fake {
    fn run(&self, a: &[String]) -> i32 {
        self.ran.borrow_mut().push(a.to_vec());
        self.code
    }
    fn room(&self, _: &str) -> Result<(), String> {
        self.no_room.clone().map_or(Ok(()), Err)
    }
    fn stage(&self, p: &str, uid: u32) -> Option<String> {
        self.staged_calls.set(self.staged_calls.get() + 1);
        (uid == 1000).then(|| format!("{STAGING}/n/{}", p.rsplit('/').next().unwrap()))
    }
    fn discard(&self, staged: &str) {
        self.discarded.borrow_mut().push(staged.into());
    }
    fn inspect(&self, _: &str) -> Result<PkgFacts, String> {
        self.unreadable.clone().map_or_else(|| Ok(facts()), Err)
    }
    fn keys(&self) -> Vec<[u8; 32]> {
        vec![wol_core::package::public_of(&self.seed)]
    }
    fn running_release(&self) -> Option<String> {
        self.running.clone()
    }
    fn staging(&self) -> &Staging {
        &self.staging
    }
    fn ask(&self, _uid: u32, a: &Approval) -> Option<Sheet> {
        self.asked.borrow_mut().push(a.clone());
        // "Yes" on an administrator sheet: the shell ran it and answers Done.
        match (&self.sheet, a.request.is_admin()) {
            (Some(Sheet::AllowOnce), true) => Some(Sheet::Done),
            (s, _) => s.clone(),
        }
    }
}

/// Audit S4-11: the one change wold runs itself (a signed newer release) is stopped at the
/// deadline when it hangs (TIMED_OUT from reaper::run), and the app hears so.
#[test]
fn a_system_update_past_its_deadline_says_so() {
    let seed = [3u8; 32];
    let f = Fake {
        code: crate::reaper::TIMED_OUT,
        running: Some(manifest("2027/01/10")),
        ..yes(seed)
    };
    let all = |_: &str, _: &str| true;
    let newer = manifest("2027/03/01");
    let sig: String = wol_core::package::sign(&seed, newer.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let call = PackageCall::SystemUpdate {
        manifest: newer,
        signature: sig,
    }
    .encode();
    let a = handle(&call, Some((1000, APP)), &all, &f);
    assert!(!a.ok);
    assert!(
        a.message.as_deref().unwrap().contains("took too long"),
        "{a:?}"
    );
    assert_eq!(f.ran.borrow().len(), 1);
}

/// An administrator sheet answers only after wol-admind ran it, so wold waits for its
/// deadlines too; an Allow sheet only for the click.
#[test]
fn administrator_sheets_are_waited_for_as_long_as_they_may_run() {
    let click = Duration::from_secs(shared::shell::APPROVAL_WAIT_SECS.into());
    let change = ApprovalRequest::Change {
        capability: "packages-update".into(),
        what: "x".into(),
    };
    assert_eq!(ask_wait(&change, Patience::Normal), click);
    let file = file_sheet();
    let long = Duration::from_secs(wol_core::admin::LONG_STEP_SECS.into());
    assert!(ask_wait(&file, Patience::Normal) >= click + long);
    assert!(ask_wait(&ApprovalRequest::SystemRollback, Patience::Normal) >= click + long);
    let three = ApprovalRequest::Packages {
        verb: "install".into(),
        names: vec!["a".into(), "b".into(), "c".into()],
    };
    assert!(ask_wait(&three, Patience::Normal) >= click + long * 3);
    let chan = ApprovalRequest::SystemChannel {
        channel: "beta".into(),
    };
    assert!(ask_wait(&chan, Patience::Normal) > click);
}

/// QA-ACC-6: with "Give me more time" the sheet stays open x2, x5 or until [`shared::patience::NEVER_SECS`],
/// and wold waits for it (10 s more for the answer to arrive), so a click on a late sheet is not lost.
#[test]
fn an_account_that_asked_for_more_time_gets_a_longer_wait() {
    let change = ApprovalRequest::Change {
        capability: "packages-update".into(),
        what: "x".into(),
    };
    let secs = |p| ask_wait(&change, p).as_secs();
    assert_eq!(secs(Patience::Normal), 60);
    assert_eq!(secs(Patience::Double), 110);
    assert_eq!(secs(Patience::Five), 260);
    assert_eq!(
        secs(Patience::Never),
        u64::from(shared::patience::NEVER_SECS) + 10
    );
    // The administrator's steps come after the click, whatever the click took.
    let file = file_sheet();
    assert!(
        ask_wait(&file, Patience::Five)
            >= ask_wait(&file, Patience::Normal) + Duration::from_secs(200)
    );
}

/// Audit S4-13: a failed administrator sheet of several names makes no "nothing changed"
/// promise (one wol-admind request per name in the shell) and passes on the shell's reason,
/// bounded and on one line; one name keeps the promise (one pacman transaction).
#[test]
fn a_failed_multi_name_change_does_not_claim_nothing_changed() {
    let all = |_: &str, _: &str| true;
    let failed = |m: &str| Fake {
        sheet: Some(Sheet::Failed { message: m.into() }),
        ..Default::default()
    };
    let many = r#"{"call":"package.change","verb":"install","names":["a","b","c"]}"#;
    let f = failed("b could not be installed\nrm -rf");
    let a = handle(many, Some((1000, APP)), &all, &f);
    let m = a.message.unwrap();
    assert!(!m.contains("Nothing was changed"), "{m}");
    assert!(m.contains("some of it may already be done"), "{m}");
    assert!(m.contains("b could not be installed"), "{m}");
    assert!(!m.contains('\n'), "{m}");
    let long = "x".repeat(5000);
    let a = handle(many, Some((1000, APP)), &all, &failed(&long));
    assert!(a.message.unwrap().len() < 400);
    let one = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
    let a = handle(one, Some((1000, APP)), &all, &failed("x"));
    assert!(a.message.unwrap().contains("Nothing was changed"));
    // Update Now: what wol-admind said (through the sheet) is what wopkg hears, word for word.
    let said = "The update did not finish. The restore point wolos-a is kept.";
    for call in [
        r#"{"call":"package.change","verb":"update","names":[]}"#,
        r#"{"call":"package.update-holding","hold":["gimp"]}"#,
    ] {
        let a = handle(call, Some((1000, APP)), &all, &failed(said));
        assert_eq!(
            (a.code, a.message.as_deref()),
            (Some(Code::Internal), Some(said))
        );
        let a = handle(call, Some((1000, APP)), &all, &failed(""));
        assert!(a.message.unwrap().contains("Nothing was changed"), "{call}");
    }
    let named = r#"{"call":"package.change","verb":"update","names":["firefox"]}"#;
    let a = handle(named, Some((1000, APP)), &all, &failed(said));
    assert!(a.message.unwrap().starts_with("The change did not finish"));
}

/// Audit S4-2: what an app names for install-file is opened without becoming wold's
/// terminal and checked on the descriptor: a device or a FIFO is refused, and at once.
#[test]
fn staging_refuses_devices_and_fifos_without_waiting() {
    let bus = Bus::default();
    let asks = Asks::default();
    let host = Host {
        bus: &bus,
        asks: &asks,
    };
    let me = shared::sys::euid();
    let t = std::time::Instant::now();
    assert!(host.stage("/dev/null", me).is_none());
    assert!(host.stage("/dev/tty", me).is_none());
    let fifo = std::env::temp_dir().join(format!("wol-stage-fifo-{}", std::process::id()));
    let _ = std::fs::remove_file(&fifo);
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if made.is_ok_and(|s| s.success()) {
        assert!(host.stage(fifo.to_str().unwrap(), me).is_none());
        let _ = std::fs::remove_file(&fifo);
    }
    assert!(t.elapsed() < std::time::Duration::from_secs(1));
}

/// QA-100 L1: wold's staging judges the folders above the file as wol-admind's copy does:
/// owning a file inside another account's private folder is not reaching it.
#[test]
fn stage_reach_check() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = (0, 0o40755);
    // Ann's file, left inside Bob's private folder: owner matches, the folder does not open.
    assert!(!may_stage(1000, &[(1001, 0o40700), root], 1000));
    assert!(
        !may_stage(1000, &[(1001, 0o40750), root], 1000),
        "group is not granted"
    );
    assert!(may_stage(1000, &[(1000, 0o40700), root], 1000));
    assert!(
        may_stage(1000, &[(1001, 0o40711), root], 1000),
        "o+x is enough"
    );
    assert!(!may_stage(1001, &[(1000, 0o40700), root], 1000), "not hers");
    // The folders are those of the opened descriptor, from the parent up to `/`.
    let dir = std::env::temp_dir().join(format!("wol-stage-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = dir.join("app.pkg.tar.zst");
    std::fs::write(&file, b"x").unwrap();
    let f = std::fs::File::open(&file).unwrap();
    let dirs = crate::admind::dirs_above(&f).unwrap();
    let md = std::fs::metadata(&dir).unwrap();
    assert_eq!(dirs[0], (md.uid(), md.mode()));
    assert_eq!(dirs.len(), file.ancestors().count() - 1, "{dirs:?}");
    let me = shared::sys::euid();
    assert!(may_stage(me, &dirs, me));
    assert!(!may_stage(me, &dirs, me + 1), "another account");
    let _ = std::fs::remove_dir_all(&dir);
}

/// R2C-8: a second staged install of the same account is refused while the first waits;
/// the cap for all accounts holds; the place is given back when the call ends.
#[test]
fn staged_installs_are_limited_per_account() {
    let t = Staging::new();
    let a = t.enter(1000).unwrap();
    assert!(t.enter(1000).is_none(), "one per account");
    let b = t.enter(1001).unwrap();
    assert!(t.enter(1002).is_none(), "the cap for all accounts");
    drop(a);
    assert!(t.enter(1000).is_some());
    drop(b);

    // Through handle: uid 1000 already has one in flight.
    let f = yes([0; 32]);
    let held = f.staging.enter(1000).unwrap();
    let call = r#"{"call":"package.install-file","path":"/home/a/x.pkg.tar.zst"}"#;
    let all = |_: &str, _: &str| true;
    let a = handle(call, Some((1000, APP)), &all, &f);
    assert_eq!(a.code, Some(Code::Unavailable), "{a:?}");
    assert!(f.asked.borrow().is_empty() && f.ran.borrow().is_empty());
    drop(held);
    assert!(handle(call, Some((1000, APP)), &all, &f).ok, "free again");
}

/// Audit P6-5: a copy that would leave the disk under its reserve is refused with a clear
/// message before any file is copied or any sheet shown.
#[test]
fn a_staged_install_is_refused_below_the_free_space_reserve() {
    let call = r#"{"call":"package.install-file","path":"/home/a/x.pkg.tar.zst"}"#;
    let all = |_: &str, _: &str| true;
    let full = Fake {
        no_room: Some(no_room_message()),
        ..yes([0; 32])
    };
    let a = handle(call, Some((1000, APP)), &all, &full);
    assert!(!a.ok);
    assert_eq!(a.code, Some(Code::Unavailable), "{a:?}");
    assert!(
        a.message.as_deref().unwrap().contains("free space"),
        "{a:?}"
    );
    assert_eq!(full.staged_calls.get(), 0, "nothing was copied");
    assert!(full.asked.borrow().is_empty() && full.ran.borrow().is_empty());
    assert!(
        full.staging.enter(1000).is_some() && full.staging.enter(1001).is_some(),
        "the place was given back"
    );
    let roomy = yes([0; 32]);
    assert!(handle(call, Some((1000, APP)), &all, &roomy).ok);
    assert_eq!(roomy.staged_calls.get(), 1);
}

#[test]
fn the_reserve_is_a_tenth_of_the_disk_and_at_least_a_gigabyte() {
    const G: u64 = 1 << 30;
    assert_eq!(staging_reserve(5 * G), G, "small disk: the floor");
    assert_eq!(staging_reserve(500 * G), 50 * G, "big disk: a tenth");
    assert!(
        enough_room(10 * G, 100 * G, 0),
        "exactly the reserve is kept"
    );
    assert!(enough_room(15 * G, 100 * G, 5 * G));
    assert!(!enough_room(15 * G, 100 * G, 6 * G), "would dip into it");
    assert!(
        !enough_room(2 * G, 100 * G, 4 * G),
        "bigger than what is free"
    );
    assert!(!enough_room(0, 0, 0), "an unreadable disk keeps nothing");
}

/// The real host reads the real disk: a temp-dir filesystem answers, with its free space.
#[test]
fn the_space_of_a_real_directory_is_read() {
    let (free, total) = crate::os::space_at(&std::env::temp_dir().join("no/such/dir")).unwrap();
    assert!(total > 0 && free <= total);
}

/// R2C-8: at start wold removes the copies a previous run left, never wol-admind's.
#[test]
fn stale_staging_directories_are_swept_at_start() {
    let root = std::env::temp_dir().join(format!("wol-staging-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mine = root.join("0123456789abcdef0123456789abcdef");
    let admind = root.join("77-123-0123456789abcdef0123456789abcdef");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("x.pkg.tar.zst"), b"x").unwrap();
    std::fs::create_dir_all(&admind).unwrap();
    std::fs::write(root.join("0123456789abcdef0123456789abcdee"), b"a file").unwrap();
    assert_eq!(sweep_staging(&root), 1);
    assert!(!mine.exists());
    assert!(admind.exists());
    assert_eq!(sweep_staging(&root.join("missing")), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_grant_only_lets_the_app_ask_and_the_person_decides() {
    let inst = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
    let all = |_: &str, _: &str| true;
    for sheet in [
        None,
        Some(Sheet::Deny),
        Some(Sheet::TimedOut),
        Some(Sheet::Busy),
        Some(Sheet::AllowAlways),
        Some(Sheet::Failed {
            message: "x".into(),
        }),
    ] {
        let f = Fake {
            sheet: sheet.clone(),
            ..Default::default()
        };
        let a = handle(inst, Some((1000, APP)), &all, &f);
        assert!(!a.ok, "{sheet:?}");
        assert!(f.ran.borrow().is_empty(), "{sheet:?}: nothing ran");
        assert_eq!(f.asked.borrow().len(), 1);
    }
    let f = yes([0; 32]);
    assert!(handle(inst, Some((1000, APP)), &all, &f).ok);
    assert!(
        f.ran.borrow().is_empty(),
        "the shell ran it through wol-admind"
    );
    let asked = f.asked.borrow();
    assert_eq!(
        asked[0].helper, APP,
        "the sheet names the kernel's executable"
    );
    assert_eq!(
        asked[0].request,
        ApprovalRequest::Packages {
            verb: "install".into(),
            names: vec!["firefox".into()]
        }
    );
    drop(asked);
    // A local file is an administrator sheet on wold's staged copy (C7-1, ADR 0034): the
    // shell ran it through wol-admind; wold runs nothing and drops its copy.
    let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
    assert!(handle(file, Some((1000, APP)), &all, &f).ok);
    assert!(f.ran.borrow().is_empty());
    assert_eq!(f.asked.borrow()[1].request, file_sheet());
    // Not granted: refused without bothering the person.
    let f = yes([0; 32]);
    let none = |_: &str, _: &str| false;
    assert!(!handle(inst, Some((1000, APP)), &none, &f).ok);
    assert!(f.asked.borrow().is_empty());
}

/// Audit S2: an "update" with names, or with holds, installs as root, so it is an
/// administrator sheet the shell runs through wol-admind. Allow Once is not enough: wold runs
/// nothing itself.
#[test]
fn updates_with_names_or_holds_need_the_administrator_sheet() {
    let all = |_: &str, _: &str| true;
    for (line, verb, names) in [
        (
            r#"{"call":"package.change","verb":"update","names":["firefox"]}"#,
            "update",
            vec!["firefox".to_string()],
        ),
        (
            r#"{"call":"package.update-holding","hold":["gimp"]}"#,
            "update-holding",
            vec!["gimp".to_string()],
        ),
    ] {
        // A forged or confused Allow Once on an administrator sheet: refused, nothing ran.
        let f = Fake {
            sheet: Some(Sheet::AllowOnce),
            ..Default::default()
        };
        struct Plain<'a>(&'a Fake);
        impl System for Plain<'_> {
            fn run(&self, a: &[String]) -> i32 {
                self.0.run(a)
            }
            fn stage(&self, p: &str, u: u32) -> Option<String> {
                self.0.stage(p, u)
            }
            fn discard(&self, s: &str) {
                self.0.discard(s)
            }
            fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
                self.0.inspect(s)
            }
            fn keys(&self) -> Vec<[u8; 32]> {
                self.0.keys()
            }
            fn running_release(&self) -> Option<String> {
                self.0.running_release()
            }
            fn ask(&self, _: u32, a: &Approval) -> Option<Sheet> {
                self.0.asked.borrow_mut().push(a.clone());
                Some(Sheet::AllowOnce)
            }
            fn staging(&self) -> &Staging {
                &self.0.staging
            }
        }
        let a = handle(line, Some((1000, APP)), &all, &Plain(&f));
        assert!(!a.ok, "{line}: Allow Once does not run pacman");
        assert!(f.ran.borrow().is_empty(), "{line}");
        assert_eq!(
            f.asked.borrow()[0].request,
            ApprovalRequest::Packages {
                verb: verb.into(),
                names: names.clone()
            },
            "{line}"
        );
        // The password sheet: the shell ran it (Done); wold still runs nothing.
        let f = yes([0; 32]);
        assert!(handle(line, Some((1000, APP)), &all, &f).ok, "{line}");
        assert!(f.ran.borrow().is_empty(), "{line}");
    }
}

/// Audit C7-1: a local file, the channel and Undo Update are administrator sheets the shell
/// runs through wol-admind (which checks the administrator itself). An Allow Once from the
/// caller's own shell, forged or confused, runs nothing as root; wold never runs them.
#[test]
fn root_changes_need_the_administrator_sheet() {
    let all = |_: &str, _: &str| true;
    for (line, want) in [
        (
            r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#,
            file_sheet(),
        ),
        (
            r#"{"call":"system.rollback"}"#,
            ApprovalRequest::SystemRollback,
        ),
        (
            r#"{"call":"system.channel","channel":"beta"}"#,
            ApprovalRequest::SystemChannel {
                channel: "beta".into(),
            },
        ),
    ] {
        let call = PackageCall::decode(line.as_bytes()).unwrap();
        assert!(
            request(
                &call,
                Some((&format!("{STAGING}/n/x-1-1-x86_64.pkg.tar.zst"), &facts()))
            )
            .is_admin()
        );
        for answer in [Sheet::AllowOnce, Sheet::AllowAlways] {
            struct Raw(Fake, Sheet);
            impl System for Raw {
                fn run(&self, a: &[String]) -> i32 {
                    self.0.run(a)
                }
                fn stage(&self, p: &str, u: u32) -> Option<String> {
                    self.0.stage(p, u)
                }
                fn discard(&self, s: &str) {
                    self.0.discard(s)
                }
                fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
                    self.0.inspect(s)
                }
                fn keys(&self) -> Vec<[u8; 32]> {
                    vec![]
                }
                fn running_release(&self) -> Option<String> {
                    None
                }
                fn ask(&self, _: u32, a: &Approval) -> Option<Sheet> {
                    self.0.asked.borrow_mut().push(a.clone());
                    Some(self.1.clone())
                }
                fn staging(&self) -> &Staging {
                    &self.0.staging
                }
            }
            let f = Raw(Fake::default(), answer.clone());
            let a = handle(line, Some((1000, APP)), &all, &f);
            assert!(!a.ok, "{line} {answer:?}");
            assert_eq!(a.code, Some(Code::PermissionDenied), "{line}");
            assert!(f.0.ran.borrow().is_empty(), "{line}: nothing ran as root");
            assert_eq!(f.0.asked.borrow()[0].request, want, "{line}");
            let staged = matches!(want, ApprovalRequest::PackageFile { .. });
            assert_eq!(f.0.discarded.borrow().len(), usize::from(staged), "{line}");
        }
        // The password sheet: wol-admind ran it in the shell (Done); wold still runs nothing.
        let f = yes([0; 32]);
        assert!(handle(line, Some((1000, APP)), &all, &f).ok, "{line}");
        assert!(f.ran.borrow().is_empty(), "{line}");
    }
}

/// Audit S1: the sheet goes to the account's shower over the bus and only a
/// `session.answer` releases it. A socket somebody of the account bound where the shell's
/// socket lives is never dialled, so its "allow-once" counts for nothing.
#[test]
fn a_fake_shell_socket_cannot_approve() {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixListener;
    let dir = std::env::temp_dir().join(format!("wol-fake-shell-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = UnixListener::bind(dir.join("wol-shell.sock")).unwrap();
    fake.set_nonblocking(true).unwrap();
    let bus: Bus<UnixStream> = Bus::default();
    let asks = Asks::default();
    let (shower, mine) = UnixStream::pair().unwrap();
    assert!(bus.subscribe(Some(1000), Some((1000, 1)), shower));
    struct ViaBus<'a> {
        fake: Fake,
        bus: &'a Bus<UnixStream>,
        asks: &'a Asks,
    }
    impl System for ViaBus<'_> {
        fn run(&self, a: &[String]) -> i32 {
            self.fake.run(a)
        }
        fn stage(&self, p: &str, u: u32) -> Option<String> {
            self.fake.stage(p, u)
        }
        fn discard(&self, s: &str) {
            self.fake.discard(s)
        }
        fn inspect(&self, s: &str) -> Result<PkgFacts, String> {
            self.fake.inspect(s)
        }
        fn keys(&self) -> Vec<[u8; 32]> {
            vec![]
        }
        fn running_release(&self) -> Option<String> {
            None
        }
        fn ask(&self, uid: u32, a: &Approval) -> Option<Sheet> {
            ask_shower(self.bus, self.asks, uid, a, Duration::from_millis(300))
        }
        fn staging(&self) -> &Staging {
            &self.fake.staging
        }
    }
    let sys = ViaBus {
        fake: Fake::default(),
        bus: &bus,
        asks: &asks,
    };
    let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
    let all = |_: &str, _: &str| true;
    let a = handle(file, Some((1000, APP)), &all, &sys);
    assert!(!a.ok, "nobody answered on the bus: nothing runs");
    assert!(sys.fake.ran.borrow().is_empty());
    assert_eq!(sys.fake.discarded.borrow().len(), 1, "the copy is dropped");
    assert!(
        fake.accept().is_err(),
        "the fake shell socket was never dialled"
    );
    // The ask reached the shower, naming the kernel's executable.
    let mut line = String::new();
    std::io::BufReader::new(&mine).read_line(&mut line).unwrap();
    let Some(shared::events::Event::SessionAsk {
        uid: 1000,
        ask: Ask::Approval { approval },
        ..
    }) = shared::events::Event::parse(&line)
    else {
        panic!("{line}")
    };
    assert_eq!(approval.helper, APP);
    assert!(matches!(
        approval.request,
        ApprovalRequest::PackageFile { .. }
    ));
    let _ = (&mine).flush();
    let _ = std::fs::remove_dir_all(&dir);
}

fn manifest(snapshot: &str) -> String {
    format!(
        r#"{{"version":"1.1.0","snapshot":"{snapshot}","wolos_repo":"r","kernel":"k","image":"i","sha256":"{}"}}"#,
        "0".repeat(64)
    )
}

const APP: &str = "/usr/bin/wopkg";

#[test]
fn only_package_and_system_calls_are_ours() {
    for (line, ours) in [
        (
            r#"{"call":"package.change","verb":"install","names":["a"]}"#,
            true,
        ),
        (r#"{"call":"system.rollback"}"#, true),
        (r#"{"call":"notification.post","title":"x"}"#, false),
        ("not json", false),
    ] {
        assert_eq!(is_package_call(line), ours, "{line}");
    }
}

#[test]
fn decisions() {
    let f = yes([7; 32]);
    let all = |a: &str, _: &str| a == APP;
    let only_install = |a: &str, g: &str| a == APP && g == "packages-install";
    let inst = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
    assert!(handle(inst, Some((1000, APP)), &only_install, &f).ok);
    assert!(
        f.ran.borrow().is_empty(),
        "installs run in the shell (wol-admind)"
    );
    let rm = r#"{"call":"package.change","verb":"remove","names":["vim"]}"#;
    let a = handle(rm, Some((1000, APP)), &only_install, &f);
    assert_eq!(
        a.code,
        Some(Code::PermissionDenied),
        "each verb has its own grant"
    );
    assert!(a.message.unwrap().contains("Privacy & Security"));
    assert_eq!(
        handle(inst, None, &all, &f).code,
        Some(Code::PermissionDenied)
    );
    assert_eq!(
        handle(inst, Some((1000, "/usr/bin/other")), &all, &f).code,
        Some(Code::PermissionDenied)
    );
    let bad = r#"{"call":"package.change","verb":"install","names":["-Syu"]}"#;
    assert_eq!(
        handle(bad, Some((1000, APP)), &all, &f).code,
        Some(Code::InvalidArgument)
    );
    assert_eq!(f.ran.borrow().len(), 0, "nothing else ran");
    let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
    assert!(handle(file, Some((1000, APP)), &all, &f).ok);
    assert!(f.ran.borrow().is_empty(), "wol-admind installed the copy");
    assert_eq!(
        f.discarded.borrow().len(),
        1,
        "the copy is forgotten after the install"
    );
    assert_eq!(
        handle(file, Some((1001, APP)), &all, &f).code,
        Some(Code::PermissionDenied),
        "not the caller's file"
    );
    assert!(
        handle(
            r#"{"call":"system.channel","channel":"beta"}"#,
            Some((1000, APP)),
            &all,
            &f
        )
        .ok,
        "wol-admind wrote the channel after the password"
    );
    assert!(f.ran.borrow().is_empty());
}

#[test]
fn system_update_needs_a_signed_newer_release() {
    let seed = [9u8; 32];
    let f = Fake {
        running: Some(manifest("2027/01/10")),
        ..yes(seed)
    };
    let all = |_: &str, _: &str| true;
    let call = |m: &str, sig: &[u8]| {
        let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
        PackageCall::SystemUpdate {
            manifest: m.into(),
            signature: hex,
        }
        .encode()
    };
    let newer = manifest("2027/03/01");
    let good = call(&newer, &wol_core::package::sign(&seed, newer.as_bytes()));
    assert!(handle(&good, Some((1000, APP)), &all, &f).ok);
    assert_eq!(
        f.ran.borrow()[0],
        s(&[WOL, "update", "apply", "--release", "2027/03/01", "--yes"])
    );
    let forged = call(&newer, &wol_core::package::sign(&[1; 32], newer.as_bytes()));
    assert_eq!(
        handle(&forged, Some((1000, APP)), &all, &f).code,
        Some(Code::PermissionDenied)
    );
    // CORE-7: a signature that is not hex (multi-byte text) is refused, never a panic.
    let odd = PackageCall::SystemUpdate {
        manifest: newer.clone(),
        signature: "aé".into(),
    }
    .encode();
    assert!(!handle(&odd, Some((1000, APP)), &all, &f).ok);
    let older = manifest("2026/12/01");
    let down = call(&older, &wol_core::package::sign(&seed, older.as_bytes()));
    assert_eq!(
        handle(&down, Some((1000, APP)), &all, &f).code,
        Some(Code::InvalidArgument),
        "no downgrades"
    );
    assert_eq!(f.ran.borrow().len(), 1);
    // The running slot's own release file (wolos-base writes it, no image hash) counts too.
    let g = Fake {
        running: Some(r#"{"version":"2027.01","snapshot":"2027/01/10"}"#.into()),
        ..yes(seed)
    };
    assert_eq!(
        handle(&down, Some((1000, APP)), &all, &g).code,
        Some(Code::InvalidArgument),
        "no downgrades from a hash-less release file"
    );
}

/// One tar header of `kind` for `name`, then the data, padded to whole blocks.
fn tar_member(name: &str, kind: u8, data: &[u8]) -> Vec<u8> {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000644");
    h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut out = h.to_vec();
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

/// A pax extended header holding `records`.
fn pax(records: &[(&str, &str)]) -> Vec<u8> {
    let mut d = Vec::new();
    for (k, v) in records {
        let body = format!(" {k}={v}\n");
        // The length counts its own digits.
        let mut n = body.len() + 1;
        while format!("{n}{body}").len() != n {
            n += 1;
        }
        d.extend_from_slice(format!("{n}{body}").as_bytes());
    }
    tar_member("PaxHeader", b'x', &d)
}

const INFO: &[u8] =
    b"# Generated by makepkg\npkgname = hello\npkgbase = hello\npkgver = 1:2.0-3\nsize = 10\n";

/// ADR 0034: the sheet's name and version come from the copy's .PKGINFO, found the way the
/// package manager names members (pax path, long-name header), with every header checked.
#[test]
fn the_pkginfo_is_found_like_the_package_manager_finds_it() {
    let plain = [
        tar_member(".BUILDINFO", b'0', b"format = 2\n"),
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("usr/bin/hello", b'0', b"\x7fELF"),
    ]
    .concat();
    assert_eq!(pkginfo_in(&plain).unwrap(), INFO);
    assert_eq!(
        parse_pkginfo(std::str::from_utf8(INFO).unwrap()).unwrap(),
        ("hello".to_owned(), "1:2.0-3".to_owned())
    );
    // A pax path renames the member that follows it: that one is the .PKGINFO, and a header
    // called .PKGINFO that pax renames is not.
    let renamed = [
        pax(&[("path", "usr/share/x")]),
        tar_member(".PKGINFO", b'0', b"pkgname = fake\npkgver = 9-9\n"),
        pax(&[("path", ".PKGINFO"), ("mtime", "1.5")]),
        tar_member("whatever", b'0', INFO),
    ]
    .concat();
    assert_eq!(pkginfo_in(&renamed).unwrap(), INFO);
    let long = [
        tar_member("././@LongLink", b'L', b".PKGINFO\0"),
        tar_member("trunc", b'0', INFO),
    ]
    .concat();
    assert_eq!(pkginfo_in(&long).unwrap(), INFO);
    // Refusals: missing, damaged, not a file, too large, cut off.
    let none = [tar_member("usr/bin/hello", b'0', b"x"), vec![0; 1024]].concat();
    assert!(pkginfo_in(&none).unwrap_err().contains(".PKGINFO"));
    let mut damaged = plain.clone();
    damaged[1024 + 3] ^= 1;
    assert!(pkginfo_in(&damaged).unwrap_err().contains("damaged"));
    let link = tar_member(".PKGINFO", b'2', b"");
    assert!(pkginfo_in(&link).unwrap_err().contains("not a plain file"));
    let big = tar_member(".PKGINFO", b'0', &vec![b'#'; MAX_PKGINFO + 1]);
    assert!(pkginfo_in(&big).unwrap_err().contains("too large"));
    let cut = &plain[..512 * 3];
    assert!(pkginfo_in(cut).is_err(), "the .PKGINFO data is cut off");
    assert!(pkginfo_in(b"not a tar at all").is_err());
    assert!(pkginfo_in(&[]).is_err());
}

/// The values a sheet shows are short, plain and unambiguous; anything else is refused.
#[test]
fn pkginfo_values_are_checked_before_they_reach_a_sheet() {
    let p = parse_pkginfo;
    assert!(
        p("pkgname = a\npkgver = 1-1\npkgname = b\n")
            .unwrap_err()
            .contains("twice")
    );
    assert!(p("pkgname = a\n").unwrap_err().contains("no version"));
    assert!(p("pkgver = 1-1\n").unwrap_err().contains("no package name"));
    for bad in ["a b", "-a", ".a", "a\u{202e}b", "a/b", ""] {
        assert!(
            p(&format!("pkgname = {bad}\npkgver = 1-1\n")).is_err(),
            "{bad:?}"
        );
    }
    let long = "a".repeat(MAX_PKG_NAME + 1);
    assert!(p(&format!("pkgname = {long}\npkgver = 1-1\n")).is_err());
    let long = "1".repeat(MAX_PKG_VERSION + 1);
    for bad in ["1 2", "1;2", "", long.as_str()] {
        assert!(
            p(&format!("pkgname = a\npkgver = {bad}\n")).is_err(),
            "{bad:?}"
        );
    }
    assert_eq!(
        p("pkgname = lib32-gcc_libs+x@y\npkgver = 2:14.2.1+r134~g1-2.1\n").unwrap(),
        ("lib32-gcc_libs+x@y".into(), "2:14.2.1+r134~g1-2.1".into())
    );
}

/// A package stream: `members`, then the end-of-archive blocks.
fn package(members: &[Vec<u8>]) -> Vec<u8> {
    [members.concat(), vec![0; 1024]].concat()
}

/// ADR 0034: every tar header of the package is walked before the sheet, so a second
/// .PKGINFO (anywhere, under any name that resolves to it) or a damaged or cut-off archive
/// is refused, and a normal package passes.
#[test]
fn a_package_with_a_second_pkginfo_or_a_damaged_archive_is_refused() {
    let scan = |b: &[u8]| scan_package(&mut &b[..], MAX_UNPACKED);
    let body = tar_member("usr/share/big", b'0', &vec![7u8; 2 * PKGINFO_HEAD]);
    let normal = package(&[
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("usr/bin/", b'5', b""),
        body.clone(),
        tar_member("usr/bin/hi", b'2', b""),
    ]);
    assert_eq!(scan(&normal).unwrap(), INFO);
    let fake: &[u8] = b"pkgname = other\npkgver = 9-9\n";
    for (what, late) in [
        ("plain", vec![tar_member(".PKGINFO", b'0', fake)]),
        ("dot-slash", vec![tar_member("./.PKGINFO", b'0', fake)]),
        (
            "pax path",
            vec![pax(&[("path", ".PKGINFO")]), tar_member("x", b'0', fake)],
        ),
        (
            "long-name header",
            vec![
                tar_member("././@LongLink", b'L', b"./.PKGINFO\0"),
                tar_member("x", b'0', fake),
            ],
        ),
    ] {
        let mut m = vec![tar_member(".PKGINFO", b'0', INFO), body.clone()];
        m.extend(late);
        let p = package(&m);
        assert!(p.len() > PKGINFO_HEAD, "{what}: past the first MiB");
        let e = scan(&p).unwrap_err();
        assert!(e.contains("more than one"), "{what}: {e}");
    }
    // Cut off in a body, or with no end-of-archive block.
    let e = scan(&normal[..PKGINFO_HEAD + 700]).unwrap_err();
    assert!(e.contains("cut off"), "{e}");
    let e = scan(&normal[..normal.len() - 1024]).unwrap_err();
    assert!(e.contains("cut off"), "{e}");
    let mut damaged = normal.clone();
    damaged[1024 + 512 + 3] ^= 1;
    assert!(scan(&damaged).unwrap_err().contains("damaged"));
    // A link carrying data, a kind the walk does not know, more than the cap.
    let odd = package(&[
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("usr/bin/hi", b'2', b"hidden"),
    ]);
    assert!(scan(&odd).unwrap_err().contains("damaged"));
    let sparse = package(&[
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("s", b'S', b""),
    ]);
    assert!(scan(&sparse).unwrap_err().contains("kind of entry"));
    let e = scan_package(&mut &normal[..], 2 << 20).unwrap_err();
    assert!(e.contains("unpacks to more than"), "{e}");
    // The first .PKGINFO still has to be in the first MiB.
    let late = package(&[body.clone(), tar_member(".PKGINFO", b'0', INFO)]);
    assert!(scan(&late).unwrap_err().contains("at its start"));
}

/// End to end on a real .pkg.tar.zst (when the system zstd is there): name, version and the
/// SHA-256 of the whole file, with a payload larger than the first MiB; a package with a
/// second .PKGINFO past it, a damaged or a missing file is refused.
#[test]
fn a_staged_package_is_read_with_the_system_zstd() {
    let dir = std::env::temp_dir().join(format!("wol-pkginfo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let tar = [
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("usr/bin/hello", b'0', &vec![7u8; 3 * PKGINFO_HEAD]),
        vec![0; 1024],
    ]
    .concat();
    std::fs::write(dir.join("x.tar"), &tar).unwrap();
    let zst = dir.join("hello-2.0-3-x86_64.pkg.tar.zst");
    let made = std::process::Command::new("zstd")
        .args(["-q", "-f", "-o"])
        .arg(&zst)
        .arg(dir.join("x.tar"))
        .status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("no zstd on this machine: end-to-end read skipped");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let f = inspect_staged(zst.to_str().unwrap()).unwrap();
    assert_eq!((f.name.as_str(), f.version.as_str()), ("hello", "1:2.0-3"));
    assert_eq!(f.sha256, wol_core::package::sha256_file(&zst).unwrap());
    assert_eq!(f.sha256.len(), 64);
    let twice = [
        tar_member(".PKGINFO", b'0', INFO),
        tar_member("usr/bin/hello", b'0', &vec![7u8; 3 * PKGINFO_HEAD]),
        tar_member(".PKGINFO", b'0', b"pkgname = other\npkgver = 9-9\n"),
        vec![0; 1024],
    ]
    .concat();
    std::fs::write(dir.join("x.tar"), &twice).unwrap();
    let made = std::process::Command::new("zstd")
        .args(["-q", "-f", "-o"])
        .arg(&zst)
        .arg(dir.join("x.tar"))
        .status()
        .unwrap();
    assert!(made.success());
    let e = inspect_staged(zst.to_str().unwrap()).unwrap_err();
    assert!(e.contains("more than one"), "{e}");
    std::fs::write(&zst, b"(\xb5/\xfd garbage").unwrap();
    assert!(inspect_staged(zst.to_str().unwrap()).is_err());
    assert!(inspect_staged(dir.join("gone.pkg.tar.zst").to_str().unwrap()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR 0034: a copy whose .PKGINFO cannot be read is refused with the reason before any
/// sheet shows, and the copy is dropped.
#[test]
fn an_unreadable_package_is_refused_before_the_sheet() {
    let all = |_: &str, _: &str| true;
    let f = Fake {
        unreadable: Some("its .PKGINFO has no version".into()),
        ..yes([0; 32])
    };
    let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
    let a = handle(file, Some((1000, APP)), &all, &f);
    assert!(!a.ok);
    assert_eq!(a.code, Some(Code::InvalidArgument));
    assert_eq!(
        a.message.as_deref(),
        Some(
            "That package file can't be read (its .PKGINFO has no version), so it was not installed."
        )
    );
    assert!(f.asked.borrow().is_empty(), "no sheet");
    assert!(f.ran.borrow().is_empty());
    assert_eq!(f.discarded.borrow().len(), 1, "the copy is dropped");
}
