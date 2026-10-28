//! Install progress for the App Store: pacman (`--noprogressbar`) and `flatpak install
//! --noninteractive` output -> a percentage, published as a small file the Store polls.
//! System installs (wol-admind) write under [`SYSTEM_DIR`]; this account's Flatpak installs under
//! `$XDG_RUNTIME_DIR/wolos-progress`.
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// wol-admind's progress files, readable by every account.
pub const SYSTEM_DIR: &str = "/run/wolos/progress";
/// Under `$XDG_RUNTIME_DIR`.
pub const USER_SUBDIR: &str = "wolos-progress";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pacman,
    Flatpak,
}

/// Reads output line by line and keeps a percentage that only goes up.
#[derive(Clone, Debug)]
pub struct Tracker {
    kind: Kind,
    /// pacman: packages in the transaction ("Packages (N)").
    total: usize,
    downloaded: usize,
    installed: usize,
    percent: u8,
}

impl Tracker {
    pub fn new(kind: Kind) -> Self {
        Tracker {
            kind,
            total: 0,
            downloaded: 0,
            installed: 0,
            percent: 0,
        }
    }

    pub fn percent(&self) -> u8 {
        self.percent
    }

    /// One line (split on `\n` or `\r`); returns the new percentage when it moved.
    pub fn feed(&mut self, line: &str) -> Option<u8> {
        let p = match self.kind {
            Kind::Pacman => self.pacman(line.trim()),
            Kind::Flatpak => flatpak_line(line),
        }?;
        let p = p.min(99);
        (p > self.percent).then(|| {
            self.percent = p;
            p
        })
    }

    /// Downloads fill 0..50, installs 50..95, hooks 95..99.
    fn pacman(&mut self, l: &str) -> Option<u8> {
        if let Some(r) = l.strip_prefix("Packages (") {
            self.total = r.split(')').next()?.trim().parse().ok()?;
            return Some(1);
        }
        let n = self.total.max(1);
        if l.ends_with("downloading...") {
            self.downloaded += 1;
            return Some((self.downloaded.min(n) * 50 / n) as u8);
        }
        if ["installing ", "upgrading ", "reinstalling ", "downgrading "]
            .iter()
            .any(|p| l.starts_with(p))
        {
            self.installed += 1;
            return Some((50 + self.installed.min(n) * 45 / n) as u8);
        }
        l.starts_with(":: Running post-transaction hooks")
            .then_some(95)
    }
}

/// `Installing 2/3… ████  45%  1.2 MB/s` -> overall percent over the steps.
fn flatpak_line(l: &str) -> Option<u8> {
    let pct: u32 = l
        .split_whitespace()
        .rev()
        .find_map(|w| w.strip_suffix('%')?.parse().ok())?;
    let pct = pct.min(100);
    let step = l.split_whitespace().find_map(|w| {
        let w = w.trim_end_matches(|c: char| !c.is_ascii_digit());
        let (a, b) = w.split_once('/')?;
        Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
    });
    Some(match step {
        Some((k, n)) if n > 0 && (1..=n).contains(&k) => (((k - 1) * 100 + pct) / n) as u8,
        _ => pct as u8,
    })
}

/// File name for a validated target (package name or app id).
pub fn file_for(dir: &str, target: &str) -> Option<String> {
    let ok = !target.is_empty()
        && !target.starts_with('.')
        && target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-@".contains(&b));
    ok.then(|| format!("{dir}/{target}"))
}

/// This account's directory, from `$XDG_RUNTIME_DIR`.
pub fn user_dir() -> Option<String> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| d.starts_with('/'))
        .map(|d| format!("{d}/{USER_SUBDIR}"))
}

/// A progress file's percentage.
pub fn read(path: &str) -> Option<u8> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn publish(path: &str, p: u8) {
    let tmp = format!("{path}.tmp");
    if std::fs::write(&tmp, format!("{p}\n")).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Runs `argv` (stdout and stderr merged), publishing the percentage to `path` as it moves.
/// The file is removed at the end. True when the command succeeded within `timeout`.
pub fn run_tracked(argv: &[&str], kind: Kind, path: &str, timeout: Duration) -> bool {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let (prog, args) = match argv.split_first() {
        Some(x) => x,
        None => return false,
    };
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(dir);
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    }
    let Ok((mut reader, writer)) = std::io::pipe() else {
        return false;
    };
    let Ok(w2) = writer.try_clone() else {
        return false;
    };
    let child = Command::new(prog)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(w2)
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    publish(path, 0);
    let deadline = Instant::now() + timeout;
    let pid = child.id();
    let watchdog = std::thread::spawn(move || {
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            // The reader ends when the child exits; the watchdog only kills a hung one.
            if unsafe { libc::kill(pid as i32, 0) } != 0 {
                return;
            }
        }
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    });
    let mut t = Tracker::new(kind);
    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if let Some(p) = t.feed(&String::from_utf8_lossy(&line)) {
                    publish(path, p);
                }
                line.clear();
            } else if line.len() < 4096 {
                line.push(b);
            }
        }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    let _ = watchdog.join();
    let _ = std::fs::remove_file(path);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(kind: Kind, out: &str) -> Vec<u8> {
        let mut t = Tracker::new(kind);
        out.split(['\n', '\r']).filter_map(|l| t.feed(l)).collect()
    }

    #[test]
    fn pacman_downloads_then_installs() {
        let out = "resolving dependencies...\nlooking for conflicting packages...\n\n\
            Packages (2) babl-0.1.110-1  gimp-3.0.4-1\n\nTotal Download Size:   25.14 MiB\n\
            :: Proceed with installation? [Y/n] \n:: Retrieving packages...\n\
             babl-0.1.110-1-x86_64 downloading...\n gimp-3.0.4-1-x86_64 downloading...\n\
            checking keyring...\n:: Processing package changes...\ninstalling babl...\n\
            installing gimp...\n:: Running post-transaction hooks...\n(1/3) Updating icon theme caches...\n";
        assert_eq!(feed(Kind::Pacman, out), [1, 25, 50, 72, 95]);
    }

    #[test]
    fn flatpak_steps_and_percent() {
        let out = "Looking for matches…\nInstalling 1/2… ████     20%  1.0 MB/s  00:10\r\
            Installing 1/2… ████████ 100%  1.0 MB/s  00:00\nInstalling 2/2… ██   50%\r\
            Installing 2/2… ████ 100%\nInstallation complete.\n";
        assert_eq!(feed(Kind::Flatpak, out), [10, 50, 75, 99]);
        assert_eq!(feed(Kind::Flatpak, "Installing… 42%\n"), [42]);
        assert_eq!(feed(Kind::Flatpak, "Downloading 100% done\nfoo 3%\n"), [99]);
    }

    #[test]
    fn progress_files_only_for_plain_targets() {
        assert_eq!(
            file_for(SYSTEM_DIR, "org.gimp.GIMP").as_deref(),
            Some("/run/wolos/progress/org.gimp.GIMP")
        );
        assert_eq!(
            file_for(SYSTEM_DIR, "gtk+3").unwrap(),
            "/run/wolos/progress/gtk+3"
        );
        for bad in ["", "../etc/passwd", "a/b", ".hidden", "a b"] {
            assert_eq!(file_for(SYSTEM_DIR, bad), None, "{bad}");
        }
    }

    #[test]
    fn run_tracked_publishes_and_cleans_up() {
        let d = std::env::temp_dir().join(format!("wol-progress-{}", std::process::id()));
        let path = d.join("x").display().to_string();
        let script = format!(
            "echo 'Installing 1/1… 40%'; sleep 0.4; cat {p} > {p}.seen; echo 'Installing 1/1… 100%'",
            p = path
        );
        assert!(run_tracked(
            &["sh", "-c", &script],
            Kind::Flatpak,
            &path,
            Duration::from_secs(10)
        ));
        assert_eq!(read(&format!("{path}.seen")), Some(40));
        assert!(!std::path::Path::new(&path).exists());
        assert!(!run_tracked(
            &["false"],
            Kind::Flatpak,
            &path,
            Duration::from_secs(5)
        ));
        let _ = std::fs::remove_dir_all(d);
    }
}
