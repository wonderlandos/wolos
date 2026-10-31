//! wold system package service (ADR 0021): wopkg asks, wold decides with the caller's executable
//! (from the kernel) and the consent grant the call names, then runs the sealed-system pacman
//! (`wol pkg --`, `controller::pkg`: live layer, pending slot for kernel/initramfs/init) or the A/B
//! system update. wopkg itself never runs as root.
use shared::packages::{PackageCall, Verb};
use shared::portal::{Answer, Code};

pub const WOL: &str = "/usr/bin/wol";
/// Root-only copy of a locally built package, so the caller cannot swap it after the check.
pub const STAGING: &str = "/var/cache/wolos/install";
pub const CHANNEL_FILE: &str = "/etc/wolos/channel";

/// What wold does on the machine (fake in tests).
pub trait System {
    /// Run `argv` as root; exit code.
    fn run(&self, argv: &[String]) -> i32;
    /// Copy `path` into [`STAGING`] if it is a regular file owned by `uid`; the copy's path.
    fn stage(&self, path: &str, uid: u32) -> Option<String>;
    fn write(&self, path: &str, text: &str) -> bool;
    /// Trusted release keys and the running release manifest's text.
    fn keys(&self) -> Vec<[u8; 32]>;
    fn running_release(&self) -> Option<String>;
}

/// The real machine (wold runs as root).
pub struct Host;

impl System for Host {
    fn run(&self, argv: &[String]) -> i32 {
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(127)
    }

    fn stage(&self, path: &str, uid: u32) -> Option<String> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        // O_NOFOLLOW + fstat on the opened file: what is checked is what is copied.
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .ok()?;
        let md = f.metadata().ok()?;
        if !md.is_file() || md.uid() != uid {
            return None;
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(STAGING)
            .ok()?;
        let name = path.rsplit('/').next()?;
        let dest = format!("{STAGING}/{name}");
        let mut out = std::fs::File::create(&dest).ok()?;
        std::io::copy(&mut f, &mut out).ok()?;
        Some(dest)
    }

    fn write(&self, path: &str, text: &str) -> bool {
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, path))
            .is_ok()
    }

    fn keys(&self) -> Vec<[u8; 32]> {
        wol_core::package::read_keys(std::path::Path::new(shared::package::KEYS_DIR))
    }

    fn running_release(&self) -> Option<String> {
        std::fs::read_to_string(wol_core::update::check::RELEASE_FILE).ok()
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

/// The command a change runs.
pub fn argv(verb: Verb, names: &[String]) -> Vec<String> {
    let mut a = s(&[WOL, "pkg", "--"]);
    a.extend(s(match (verb, names.is_empty()) {
        (Verb::Install, _) => &["-S", "--noconfirm", "--needed"],
        (Verb::Remove, _) => &["-Rns", "--noconfirm"],
        (Verb::Update, true) => &["-Syu", "--noconfirm"],
        (Verb::Update, false) => &["-S", "--noconfirm"],
    }));
    // `--` again: names are positional even if a check were ever missed.
    if !names.is_empty() {
        a.push("--".into());
        a.extend(names.iter().cloned());
    }
    a
}

/// Is this call line one of wopkg's (`package.*`, `system.*`)? Everything else is a libwol call.
pub fn is_package_call(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json.trim())
        .ok()
        .and_then(|v| v.get("call")?.as_str().map(str::to_owned))
        .is_some_and(|c| c.starts_with("package.") || c.starts_with("system."))
}

fn denied(grant: &str, app: &str) -> Answer {
    let what = match grant {
        "packages-install" => "install system apps",
        "packages-remove" => "remove system apps",
        "packages-update" => "update the system",
        _ => "install apps built on this computer",
    };
    let name = app.rsplit('/').next().unwrap_or(app);
    Answer::err(
        Code::PermissionDenied,
        format!(
            "{name} is not allowed to {what}. Allow it in System Settings > Privacy & Security > Software."
        ),
    )
}

/// One call line's JSON -> answer. `caller` = (uid, canonical executable), from the kernel.
pub fn handle(
    json: &str,
    caller: Option<(u32, &str)>,
    granted: &dyn Fn(&str, &str) -> bool,
    sys: &dyn System,
) -> Answer {
    let call = match PackageCall::decode(json.as_bytes()) {
        Ok(c) => c,
        Err(m) => return Answer::err(Code::InvalidArgument, m),
    };
    let Some((uid, app)) = caller else {
        return Answer::err(
            Code::PermissionDenied,
            "WolOS could not tell which app asked, so it did nothing.",
        );
    };
    if !granted(app, call.grant()) {
        return denied(call.grant(), app);
    }
    let ran = |argv: Vec<String>, what: &str| {
        if sys.run(&argv) == 0 {
            Answer::ok()
        } else {
            Answer::err(
                Code::Internal,
                format!("{what} did not finish. Nothing was changed; try again."),
            )
        }
    };
    match call {
        PackageCall::Change { verb, names } => ran(argv(verb, &names), "The change"),
        PackageCall::InstallFile { path } => match sys.stage(&path, uid) {
            Some(copy) => ran(
                s(&[WOL, "pkg", "--", "-U", "--noconfirm", "--", &copy]),
                "The install",
            ),
            None => Answer::err(
                Code::PermissionDenied,
                "That package file is not yours, so it was not installed.",
            ),
        },
        PackageCall::SystemUpdate {
            manifest,
            signature,
        } => {
            let sig = (0..signature.len() / 2)
                .filter_map(|i| u8::from_str_radix(&signature[2 * i..2 * i + 2], 16).ok())
                .collect::<Vec<u8>>();
            let m = match wol_core::update::check::Manifest::verify(
                manifest.as_bytes(),
                &sig,
                &sys.keys(),
            ) {
                Ok(m) => m,
                Err(_) => {
                    return Answer::err(
                        Code::PermissionDenied,
                        "That release is not signed by WolOS, so it was not installed.",
                    );
                }
            };
            let running = sys
                .running_release()
                .and_then(|t| wol_core::update::check::Manifest::parse(&t));
            if running.is_some_and(|r| r.snapshot >= m.snapshot) {
                return Answer::err(
                    Code::InvalidArgument,
                    "This computer already runs that release or a newer one. To go back, use Undo Update.",
                );
            }
            ran(
                s(&[WOL, "update", "apply", "--release", &m.snapshot, "--yes"]),
                "The system update",
            )
        }
        PackageCall::SystemChannel { channel } => {
            if sys.write(CHANNEL_FILE, &format!("{channel}\n")) {
                Answer::ok()
            } else {
                Answer::err(Code::Internal, "The update channel could not be changed.")
            }
        }
        PackageCall::SystemRollback {} => ran(s(&[WOL, "update", "rollback"]), "Undo Update"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        ran: RefCell<Vec<Vec<String>>>,
        wrote: RefCell<Vec<(String, String)>>,
        seed: [u8; 32],
        running: Option<String>,
    }
    impl System for Fake {
        fn run(&self, a: &[String]) -> i32 {
            self.ran.borrow_mut().push(a.to_vec());
            0
        }
        fn stage(&self, p: &str, uid: u32) -> Option<String> {
            (uid == 1000).then(|| format!("{STAGING}/{}", p.rsplit('/').next().unwrap()))
        }
        fn write(&self, p: &str, t: &str) -> bool {
            self.wrote.borrow_mut().push((p.into(), t.into()));
            true
        }
        fn keys(&self) -> Vec<[u8; 32]> {
            vec![wol_core::package::public_of(&self.seed)]
        }
        fn running_release(&self) -> Option<String> {
            self.running.clone()
        }
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
        let f = Fake {
            seed: [7; 32],
            ..Default::default()
        };
        let all = |a: &str, _: &str| a == APP;
        let only_install = |a: &str, g: &str| a == APP && g == "packages-install";
        let inst = r#"{"call":"package.change","verb":"install","names":["firefox"]}"#;
        assert!(handle(inst, Some((1000, APP)), &only_install, &f).ok);
        assert_eq!(
            f.ran.borrow()[0],
            s(&[
                WOL,
                "pkg",
                "--",
                "-S",
                "--noconfirm",
                "--needed",
                "--",
                "firefox"
            ])
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
        assert_eq!(f.ran.borrow().len(), 1, "nothing else ran");
        let file = r#"{"call":"package.install-file","path":"/home/a/x-1-1-x86_64.pkg.tar.zst"}"#;
        assert!(handle(file, Some((1000, APP)), &all, &f).ok);
        assert_eq!(
            f.ran.borrow()[1].last().unwrap(),
            &format!("{STAGING}/x-1-1-x86_64.pkg.tar.zst")
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
            .ok
        );
        assert_eq!(f.wrote.borrow()[0], (CHANNEL_FILE.into(), "beta\n".into()));
    }

    #[test]
    fn system_update_needs_a_signed_newer_release() {
        let seed = [9u8; 32];
        let f = Fake {
            seed,
            running: Some(manifest("2027/01/10")),
            ..Default::default()
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
        let older = manifest("2026/12/01");
        let down = call(&older, &wol_core::package::sign(&seed, older.as_bytes()));
        assert_eq!(
            handle(&down, Some((1000, APP)), &all, &f).code,
            Some(Code::InvalidArgument),
            "no downgrades"
        );
        assert_eq!(f.ran.borrow().len(), 1);
    }
}
