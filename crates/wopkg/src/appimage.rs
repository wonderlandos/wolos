// SPDX-License-Identifier: GPL-2.0-or-later
//! AppImages as first-class apps: adopt or install into `~/Applications`, read the embedded update
//! information (`.upd_info`) and signature (`.sha256_sig`, `.sig_key`), update by zsync delta,
//! keep the previous file for rollback. Runs as the person; the AppImage itself is never executed.
use crate::context::Context;
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// ELF section name -> (file offset, size).
pub fn sections(b: &[u8]) -> Option<BTreeMap<String, (usize, usize)>> {
    if b.get(..4)? != b"\x7fELF" || *b.get(4)? != 2 || *b.get(5)? != 1 {
        return None; // 64-bit little-endian only (x86_64, aarch64)
    }
    let u16_at = |o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as usize);
    let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize);
    let u64_at = |o: usize| Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?) as usize);
    let shoff = u64_at(0x28)?;
    let (shent, shnum, shstr) = (u16_at(0x3a)?, u16_at(0x3c)?, u16_at(0x3e)?);
    if shent < 64 || shnum > 4096 {
        return None;
    }
    let sh = |i: usize| shoff.checked_add(i.checked_mul(shent)?);
    let str_off = u64_at(sh(shstr)? + 0x18)?;
    let mut out = BTreeMap::new();
    for i in 0..shnum {
        let h = sh(i)?;
        let name_at = str_off.checked_add(u32_at(h)?)?;
        let name_end = name_at + b.get(name_at..)?.iter().position(|c| *c == 0)?;
        let name = String::from_utf8_lossy(&b[name_at..name_end]).into_owned();
        out.insert(name, (u64_at(h + 0x18)?, u64_at(h + 0x20)?));
    }
    Some(out)
}

/// Type 2 AppImage: ELF with `AI\x02` at offset 8.
pub fn is_appimage(b: &[u8]) -> bool {
    b.starts_with(b"\x7fELF") && b.get(8..11) == Some(b"AI\x02")
}

/// Section contents up to the first NUL.
fn section_text(b: &[u8], s: &BTreeMap<String, (usize, usize)>, name: &str) -> Option<String> {
    let (o, n) = *s.get(name)?;
    let raw = b.get(o..o.checked_add(n)?)?;
    let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
    let t = String::from_utf8_lossy(&raw[..end]).trim().to_owned();
    (!t.is_empty()).then_some(t)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UpdateInfo {
    Zsync {
        url: String,
    },
    GhReleasesZsync {
        owner: String,
        repo: String,
        tag: String,
        file: String,
    },
}

impl UpdateInfo {
    /// The AppImage update information string (`zsync|URL`, `gh-releases-zsync|o|r|tag|glob`).
    pub fn parse(s: &str) -> Option<UpdateInfo> {
        let f: Vec<&str> = s.trim().split('|').collect();
        match f[..] {
            ["zsync", url] => Some(UpdateInfo::Zsync { url: url.into() }),
            ["gh-releases-zsync", o, r, t, file]
                if [o, r, t, file]
                    .iter()
                    .all(|x| !x.is_empty() && !x.contains('/')) =>
            {
                Some(UpdateInfo::GhReleasesZsync {
                    owner: o.into(),
                    repo: r.into(),
                    tag: t.into(),
                    file: file.into(),
                })
            }
            _ => None,
        }
    }
}

/// `*` wildcard match (AppImage file globs).
pub fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else { return false };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

/// What wopkg learns from an AppImage without running it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inspect {
    pub appimage: bool,
    pub sha256: String,
    pub sha1: String,
    pub size: u64,
    pub update: Option<UpdateInfo>,
    pub update_raw: Option<String>,
    pub signed: bool,
    /// SHA-256 of the embedded public key (pinned at install).
    pub key_pin: Option<String>,
    /// The digest the embedded signature signs (hex SHA-256 with the signature sections zeroed).
    pub signed_digest: String,
}

pub fn inspect(path: &Path) -> Result<Inspect> {
    let b = std::fs::read(path).map_err(|_| {
        Error::new(
            crate::ffi::Status::NotFound,
            "That AppImage cannot be read.",
        )
    })?;
    let s = sections(&b).unwrap_or_default();
    let sig = section_text(&b, &s, ".sha256_sig");
    let key = section_text(&b, &s, ".sig_key");
    let mut zeroed = b.clone();
    for n in [".sha256_sig", ".sig_key"] {
        if let Some(&(o, len)) = s.get(n)
            && let Some(r) = o.checked_add(len).and_then(|e| zeroed.get_mut(o..e))
        {
            r.fill(0);
        }
    }
    let raw = section_text(&b, &s, ".upd_info");
    Ok(Inspect {
        appimage: is_appimage(&b),
        sha256: crate::fetch::hex(&Sha256::digest(&b)),
        sha1: crate::fetch::hex(&sha1::Sha1::digest(&b)),
        size: b.len() as u64,
        update: raw.as_deref().and_then(UpdateInfo::parse),
        update_raw: raw,
        signed: sig.is_some() && key.is_some(),
        key_pin: key.map(|k| crate::fetch::hex(&Sha256::digest(k.as_bytes()))),
        signed_digest: crate::fetch::hex(&Sha256::digest(&zeroed)),
    })
}

/// A private scratch directory (GNUPGHOME and the files gpg reads): `<base>/wopkg-gpg-<nonce>`,
/// created with mode 0700 and never reused; removed with everything in it when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// A new directory under `base`; Err when it cannot be made or a name already exists there.
    fn new(base: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let nonce = shared::jobs::new_nonce()?;
        let dir = base.join(format!("wopkg-gpg-{nonce}"));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let s = Scratch(dir);
        // The umask may have taken bits off; never more than 0700 either way.
        std::fs::set_permissions(&s.0, std::fs::Permissions::from_mode(0o700))?;
        Ok(s)
    }

    /// Write `name` in the directory as a new file (never through something already there).
    fn write(&self, name: &str, data: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.0.join(name))?
            .write_all(data)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Check the embedded signature with gpgv against the embedded key (only after the key matched the
/// pin). The key goes into a throwaway keyring; nothing touches the person's GnuPG home.
pub fn verify_signature(path: &Path, info: &Inspect, pin: Option<&str>) -> Result<bool> {
    match (info.signed, pin) {
        (false, None) => return Ok(false),
        (false, Some(_)) => {
            return Err(Error::untrusted(
                "This update is not signed, but the app was. It was not installed.",
            ));
        }
        (true, Some(p)) if info.key_pin.as_deref() != Some(p) => {
            return Err(Error::untrusted(
                "This update is signed with a different key than the installed app. It was not installed.",
            ));
        }
        _ => {}
    }
    let b = std::fs::read(path).map_err(|_| Error::unavailable("That AppImage cannot be read."))?;
    let s = sections(&b).unwrap_or_default();
    let (sig, key) = (
        section_text(&b, &s, ".sha256_sig").unwrap_or_default(),
        section_text(&b, &s, ".sig_key").unwrap_or_default(),
    );
    // Audit S4-15: a fresh directory only this process made (unpredictable name, create_dir
    // fails on an existing one, 0700), files created new, removed on every path by the guard.
    let scratch = Scratch::new(&std::env::temp_dir())
        .map_err(|_| Error::unavailable("The signature cannot be checked."))?;
    let dir = scratch.0.clone();
    let w = |n: &str, t: &[u8]| scratch.write(n, t);
    w("key.asc", key.as_bytes())
        .and_then(|_| w("sig.asc", sig.as_bytes()))
        .and_then(|_| w("digest", info.signed_digest.as_bytes()))
        .map_err(|_| Error::unavailable("The signature cannot be checked."))?;
    let run = |argv: &[&str]| {
        std::process::Command::new(argv[0])
            .args(&argv[1..])
            .env("GNUPGHOME", &dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    let d = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let ok = run(&[
        "gpg",
        "--batch",
        "--yes",
        "--dearmor",
        "-o",
        &d("key.gpg"),
        &d("key.asc"),
    ]) && run(&[
        "gpgv",
        "--keyring",
        &d("key.gpg"),
        &d("sig.asc"),
        &d("digest"),
    ]);
    drop(scratch);
    if ok {
        Ok(true)
    } else {
        Err(Error::untrusted(
            "The AppImage's signature does not match. It was not installed.",
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub update: Option<String>,
    #[serde(default)]
    pub key_pin: Option<String>,
    #[serde(default)]
    pub signed: bool,
    /// Where it came from when installed by URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Newest `MTime:` an update's control file had: an older one is refused (audit A14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    /// SHA-256 of the files earlier updates replaced: serving one of them again is a downgrade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded: Vec<String>,
}

/// Replaced versions remembered per app.
const SUPERSEDED: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Registry {
    pub apps: Vec<App>,
}

impl Registry {
    fn path(ctx: &Context) -> PathBuf {
        ctx.state.join("appimages.json")
    }
    pub fn load(ctx: &Context) -> Registry {
        std::fs::read(Self::path(ctx))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, ctx: &Context) -> Result<()> {
        crate::journal::write_atomic(
            &Self::path(ctx),
            serde_json::to_string_pretty(self)
                .unwrap_or_default()
                .as_bytes(),
        )
    }
    pub fn upsert(&mut self, a: App) {
        self.apps.retain(|x| x.name != a.name);
        self.apps.push(a);
        self.apps.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

/// `Foo-1.2.3-x86_64.AppImage` -> `Foo`.
pub fn name_from_file(file: &str) -> String {
    let stem = file
        .strip_suffix(".AppImage")
        .or_else(|| file.strip_suffix(".appimage"))
        .unwrap_or(file);
    let cut = stem
        .char_indices()
        .find(|(i, c)| {
            (*c == '-' || *c == '_' || *c == '.')
                && stem[i + 1..].starts_with(|d: char| {
                    d.is_ascii_digit()
                        || d == 'v' && stem[i + 2..].starts_with(|e: char| e.is_ascii_digit())
                })
        })
        .map(|(i, _)| i)
        .unwrap_or(stem.len());
    let n = stem[..cut].trim_end_matches(['-', '_', '.']).to_owned();
    let n = n
        .strip_suffix("-x86_64")
        .or_else(|| n.strip_suffix("-amd64"))
        .unwrap_or(&n)
        .to_owned();
    if n.is_empty() { stem.to_owned() } else { n }
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('.')
        && n.chars()
            .all(|c| c.is_alphanumeric() || "._- +".contains(c))
}

/// The app's web-policy id: the launcher's slug.
fn policy_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

/// Web-policy apply step for AppImages: the flag rows the app's policy allows (the account's
/// runtime profile, no GPU tier: the launcher cannot know the session's). The image itself is
/// never edited, so a policy's `patches` are not applied here; a refused or missing policy adds
/// nothing. Every line is one `--` word (`webruntime` tests), safe on an `Exec=` line.
pub fn policy_args(policy_dir: &Path, config_home: &Path, name: &str) -> Vec<String> {
    use wol_core::{webpolicy, webruntime};
    let Some(Ok(p)) = webpolicy::load(policy_dir, &policy_id(name)) else {
        return Vec::new();
    };
    let profile = webruntime::load_profile(config_home);
    webruntime::flags_with(
        profile,
        webruntime::Role::App,
        webruntime::Gpu::Soft,
        &|row| webpolicy::allows(&p, row),
    )
}

fn config_home(ctx: &Context) -> PathBuf {
    crate::context::env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| ctx.home.join(".config"))
}

/// Written at install and again after every update, so the policy is re-applied each time.
fn desktop_entry(ctx: &Context, a: &App) -> Result<()> {
    let dir = ctx.home.join(".local/share/applications");
    let file = dir.join(format!("wopkg-appimage-{}.desktop", policy_id(&a.name)));
    let args: String = policy_args(
        Path::new(wol_core::webpolicy::DIR),
        &config_home(ctx),
        &a.name,
    )
    .iter()
    .map(|f| format!(" {f}"))
    .collect();
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"{args} %U\nIcon=application-x-executable\nCategories=Utility;\nX-WolOS-Source=appimage\n",
        a.name,
        a.path.display()
    );
    std::fs::create_dir_all(&dir).ok();
    crate::journal::write_atomic(&file, text.as_bytes())
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct InstallRequest {
    /// Download from here (HTTPS) ...
    pub url: Option<String>,
    /// ... or adopt this file already on disk (moved into `~/Applications` unless already there).
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub app: App,
    pub signed: bool,
    pub journal: String,
}

fn make_exec(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
        .map_err(|_| Error::unavailable("The app cannot be made runnable."))
}

/// The file name an AppImage URL's last part gives, safe as one name in the staging folder:
/// no query or fragment, never `.`/`..`, no `/`, `\\` or NUL, never hidden; else
/// `app.AppImage` (QA-STORE-26).
pub fn url_file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let clean: String = last
        .chars()
        .filter(|c| !matches!(c, '\\' | '\0') && !c.is_control())
        .collect();
    let clean = clean.trim_start_matches('.');
    if clean.is_empty() || clean.len() > 200 {
        "app.AppImage".into()
    } else {
        clean.to_owned()
    }
}

pub fn install(
    ctx: &Context,
    r: &InstallRequest,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Installed> {
    let staging = ctx.apps.join(".wopkg-staging");
    std::fs::create_dir_all(&staging)
        .map_err(|_| Error::unavailable("~/Applications cannot be created."))?;
    let (src, file_name) = match (&r.url, &r.path) {
        (Some(u), None) => {
            let file = url_file_name(u);
            let dest = staging.join(&file);
            let item = crate::fetch::Item {
                url: u.clone(),
                dest: dest.clone(),
                sha256: r.sha256.clone(),
                size: None,
            };
            let d = crate::fetch::many(
                &ctx.net,
                &crate::fetch::Request {
                    items: vec![item],
                    jobs: Some(1),
                },
                progress,
            );
            if let Some(crate::fetch::Outcome::Failed { message }) =
                d.first().map(|d| d.outcome.clone())
            {
                return Err(Error::unavailable(message));
            }
            (dest, file)
        }
        (None, Some(p)) => (
            p.clone(),
            p.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        _ => return Err(Error::invalid("Give a URL or a file to install.")),
    };
    let info = inspect(&src)?;
    if !info.appimage {
        return Err(Error::invalid("That file is not an AppImage."));
    }
    if let Some(w) = &r.sha256
        && !w.eq_ignore_ascii_case(&info.sha256)
    {
        return Err(Error::untrusted(
            "The AppImage does not match its checksum.",
        ));
    }
    let signed = verify_signature(&src, &info, None)?;
    let name = r.name.clone().unwrap_or_else(|| name_from_file(&file_name));
    if !valid_name(&name) {
        return Err(Error::invalid(format!(
            "“{name}” cannot be used as an app name."
        )));
    }
    let mut reg = Registry::load(ctx);
    let dest = if src.parent() == Some(ctx.apps.as_path()) {
        src.clone()
    } else {
        ctx.apps.join(format!("{name}.AppImage"))
    };
    let prev = reg.apps.iter().find(|a| a.name == name).cloned();
    let mut undo = None;
    if dest != src {
        if dest.exists() {
            let p = backup_path(&dest);
            std::fs::rename(&dest, &p)
                .map_err(|_| Error::unavailable("The installed copy cannot be kept for undo."))?;
            undo = Some(p);
        }
        move_file(&src, &dest)?;
    }
    make_exec(&dest)?;
    let app = App {
        name: name.clone(),
        path: dest.clone(),
        sha256: info.sha256.clone(),
        update: info.update_raw.clone(),
        key_pin: info.key_pin.clone(),
        signed,
        url: r.url.clone(),
        mtime: None,
        superseded: Vec::new(),
    };
    reg.upsert(app.clone());
    reg.save(ctx)?;
    desktop_entry(ctx, &app)?;
    let id = crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            if prev.is_some() { "update" } else { "install" },
            &name,
            prev.as_ref().map(|p| p.sha256.clone()),
            Some(info.sha256),
            undo.map(|p| crate::journal::Undo::Restore {
                from: p,
                to: dest.clone(),
            })
            .or(Some(crate::journal::Undo::Remove { path: dest.clone() })),
        ),
    )?;
    Ok(Installed {
        app,
        signed,
        journal: id,
    })
}

fn backup_path(p: &Path) -> PathBuf {
    PathBuf::from(format!("{}.prev", p.display()))
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_then_rename(from, to)
        .and_then(|()| std::fs::remove_file(from))
        .map_err(|_| Error::unavailable("The app cannot be moved into ~/Applications."))
}

/// The cross-device fallback of [`move_file`]: the copy goes to `<to>.tmp`, is fsynced, then renamed
/// over `to`, so a crash or full disk never leaves a truncated file at the final path (audit8
/// stability2 #9: the next install would have renamed it over the `.prev` backup).
fn copy_then_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", to.display()));
    let copied = (|| {
        let mut src = std::fs::File::open(from)?;
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut src, &mut out)?;
        out.set_permissions(src.metadata()?.permissions())?;
        out.sync_all()
    })();
    if let Err(e) = copied.and_then(|()| std::fs::rename(&tmp, to)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Some(dir) = to.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Find the `.zsync` URL for an app's update information.
pub fn zsync_url(ctx: &Context, u: &UpdateInfo, api_base: &str) -> Result<String> {
    match u {
        UpdateInfo::Zsync { url } => Ok(url.clone()),
        UpdateInfo::GhReleasesZsync {
            owner,
            repo,
            tag,
            file,
        } => {
            let path = if tag == "latest" {
                "releases/latest".to_owned()
            } else {
                format!("releases/tags/{tag}")
            };
            let api = format!("{api_base}/repos/{owner}/{repo}/{path}");
            ctx.net.check_url(&api)?;
            let resp = ctx
                .net
                .agent
                .get(&api)
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|_| Error::unavailable("GitHub could not be reached."))?;
            if resp.status().as_u16() != 200 {
                return Err(Error::unavailable(format!(
                    "GitHub has no release for {owner}/{repo}."
                )));
            }
            let mut body = String::new();
            resp.into_body()
                .into_reader()
                .take(4 << 20)
                .read_to_string(&mut body)
                .map_err(|_| Error::unavailable("GitHub's answer was cut off."))?;
            let v: serde_json::Value = serde_json::from_str(&body)
                .map_err(|_| Error::unavailable("GitHub sent an unreadable answer."))?;
            // More than one match happens (a re-spin next to the original); the shortest name is the
            // plain one, which is the file its own control file describes.
            v["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|a| a["name"].as_str().is_some_and(|n| glob(file, n)))
                .min_by_key(|a| a["name"].as_str().map_or(usize::MAX, str::len))
                .and_then(|a| a["browser_download_url"].as_str().map(str::to_owned))
                .ok_or_else(|| {
                    Error::new(
                        crate::ffi::Status::NotFound,
                        format!("The latest {repo} release has no update file."),
                    )
                })
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpdateRequest {
    /// Only this app (default: every registered app).
    pub name: Option<String>,
    #[serde(default)]
    pub check_only: bool,
    /// Also register AppImages found in `~/Applications` that wopkg does not know yet.
    #[serde(default)]
    pub adopt: bool,
    /// GitHub API base (tests).
    pub github_api: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum UpdateState {
    UpToDate,
    Available {
        size: u64,
    },
    Updated {
        reused_bytes: u64,
        fetched_bytes: u64,
        journal: String,
    },
    NoUpdateInfo,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateResult {
    pub name: String,
    #[serde(flatten)]
    pub state: UpdateState,
}

/// Remove an installed AppImage: the file goes to `<file>.prev` (undo puts it back), the menu
/// entry and the registry line go away.
pub fn remove(ctx: &Context, name: &str) -> Result<String> {
    let mut reg = Registry::load(ctx);
    let a = reg
        .apps
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                crate::ffi::Status::NotFound,
                format!("No AppImage called “{name}” is installed."),
            )
        })?;
    let prev = backup_path(&a.path);
    let _ = std::fs::remove_file(&prev);
    std::fs::rename(&a.path, &prev)
        .map_err(|_| Error::unavailable(format!("{name} cannot be moved away.")))?;
    let desktop = ctx.home.join(format!(
        ".local/share/applications/wopkg-appimage-{}.desktop",
        a.name.to_lowercase().replace(' ', "-")
    ));
    let _ = std::fs::remove_file(desktop);
    reg.apps.retain(|x| x.name != name);
    reg.save(ctx)?;
    crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            "remove",
            name,
            Some(a.sha256.clone()),
            None,
            Some(crate::journal::Undo::Restore {
                from: prev,
                to: a.path.clone(),
            }),
        ),
    )
}

/// Register AppImages in `~/Applications` that are not yet known (they get updates too).
pub fn adopt(ctx: &Context) -> Vec<String> {
    let mut reg = Registry::load(ctx);
    let mut added = Vec::new();
    for e in std::fs::read_dir(&ctx.apps).into_iter().flatten().flatten() {
        let p = e.path();
        let f = e.file_name().to_string_lossy().into_owned();
        if !f.to_lowercase().ends_with(".appimage") || reg.apps.iter().any(|a| a.path == p) {
            continue;
        }
        let Ok(info) = inspect(&p) else { continue };
        if !info.appimage {
            continue;
        }
        let name = name_from_file(&f);
        if !valid_name(&name) || reg.apps.iter().any(|a| a.name == name) {
            continue;
        }
        reg.upsert(App {
            name: name.clone(),
            path: p,
            sha256: info.sha256,
            update: info.update_raw,
            key_pin: info.key_pin,
            signed: info.signed,
            url: None,
            mtime: None,
            superseded: Vec::new(),
        });
        added.push(name);
    }
    if !added.is_empty() {
        let _ = reg.save(ctx);
    }
    added
}

pub fn update(
    ctx: &Context,
    r: &UpdateRequest,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Vec<UpdateResult>> {
    if r.adopt {
        adopt(ctx);
    }
    let api = r
        .github_api
        .clone()
        .unwrap_or("https://api.github.com".into());
    let reg = Registry::load(ctx);
    let mut out = Vec::new();
    for a in reg
        .apps
        .iter()
        .filter(|a| r.name.as_deref().is_none_or(|n| n == a.name))
    {
        let state = match update_one(ctx, a, &api, r.check_only, progress) {
            Ok(s) => s,
            Err(e) => UpdateState::Failed { message: e.message },
        };
        progress(&serde_json::json!({"event":"appimage","name":a.name,"result":state}));
        out.push(UpdateResult {
            name: a.name.clone(),
            state,
        });
    }
    if r.name.is_some() && out.is_empty() {
        return Err(Error::new(
            crate::ffi::Status::NotFound,
            "No AppImage with that name is installed.",
        ));
    }
    Ok(out)
}

fn update_one(
    ctx: &Context,
    a: &App,
    api: &str,
    check_only: bool,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<UpdateState> {
    let Some(u) = a.update.as_deref().and_then(UpdateInfo::parse) else {
        return Ok(UpdateState::NoUpdateInfo);
    };
    let zurl = zsync_url(ctx, &u, api)?;
    ctx.net.check_url(&zurl)?;
    let resp = ctx
        .net
        .agent
        .get(&zurl)
        .call()
        .map_err(|_| Error::unavailable("The update server could not be reached."))?;
    if resp.status().as_u16() != 200 {
        return Err(Error::unavailable(format!(
            "The update server answered {}.",
            resp.status().as_u16()
        )));
    }
    let mut zbytes = Vec::new();
    resp.into_body()
        .into_reader()
        .take(64 << 20)
        .read_to_end(&mut zbytes)
        .map_err(|_| Error::unavailable("The update description was cut off."))?;
    let c = crate::zsync::parse(&zbytes)?;
    let cur_sha1 = crate::zsync::sha1_file(&a.path).ok_or_else(|| {
        Error::new(
            crate::ffi::Status::NotFound,
            format!("{} is missing from ~/Applications.", a.name),
        )
    })?;
    if cur_sha1 == c.sha1 {
        return Ok(UpdateState::UpToDate);
    }
    // Version floor: never go back to something older than what an update already brought. Once
    // there is a floor, a control file without MTime counts as older (audit S12): the server
    // chooses what the file says, so leaving the date out must not skip the check. The installed
    // copy matching the control file's SHA-1 (up to date, above) is the only pass without one.
    match (a.mtime, c.mtime) {
        (Some(floor), Some(m)) if m < floor => {
            return Err(Error::untrusted(
                "The update server offers an older version than the one installed. It was not installed.",
            ));
        }
        (Some(_), None) => {
            return Err(Error::untrusted(
                "The update server no longer says how new its version is, so it cannot be told from an older one. It was not installed.",
            ));
        }
        _ => {}
    }
    if check_only {
        return Ok(UpdateState::Available { size: c.length });
    }
    let file_url = crate::zsync::resolve(&zurl, &c.url);
    let staging = ctx.apps.join(".wopkg-staging");
    std::fs::create_dir_all(&staging)
        .map_err(|_| Error::unavailable("~/Applications cannot be written."))?;
    let new = staging.join(format!("{}.new", a.name));
    let stats = crate::zsync::build(&ctx.net, &c, &file_url, &a.path, &new, progress)?;
    let info = inspect(&new)?;
    if !info.appimage {
        let _ = std::fs::remove_file(&new);
        return Err(Error::untrusted(
            "The update is not an AppImage. It was not installed.",
        ));
    }
    if let Err(e) = verify_signature(&new, &info, a.key_pin.as_deref()) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if a.superseded.contains(&info.sha256) {
        let _ = std::fs::remove_file(&new);
        return Err(Error::untrusted(
            "The update is a version this app was already updated from. It was not installed.",
        ));
    }
    make_exec(&new)?;
    let prev = backup_path(&a.path);
    let _ = std::fs::remove_file(&prev);
    std::fs::hard_link(&a.path, &prev)
        .or_else(|_| std::fs::copy(&a.path, &prev).map(|_| ()))
        .map_err(|_| Error::unavailable("The installed copy cannot be kept for undo."))?;
    std::fs::rename(&new, &a.path)
        .map_err(|_| Error::unavailable("The update cannot be put in place."))?;
    let mut reg = Registry::load(ctx);
    let mut na = a.clone();
    na.sha256 = info.sha256.clone();
    na.update = info.update_raw.clone().or(na.update);
    na.mtime = c.mtime.max(a.mtime);
    if !na.superseded.contains(&a.sha256) {
        na.superseded.push(a.sha256.clone());
    }
    let extra = na.superseded.len().saturating_sub(SUPERSEDED);
    na.superseded.drain(..extra);
    if na.key_pin.is_none() {
        na.key_pin = info.key_pin.clone();
        na.signed = info.signed;
    }
    reg.upsert(na.clone());
    reg.save(ctx)?;
    desktop_entry(ctx, &na)?;
    let id = crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            "update",
            &a.name,
            Some(a.sha256.clone()),
            Some(info.sha256),
            Some(crate::journal::Undo::Restore {
                from: prev,
                to: a.path.clone(),
            }),
        ),
    )?;
    Ok(UpdateState::Updated {
        reused_bytes: stats.reused_bytes,
        fetched_bytes: stats.fetched_bytes,
        journal: id,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testutil::{Served, temp};

    /// Web policy: the allowed flag rows reach the launcher; no policy, a refused one, or a
    /// patch list changes nothing (the image is never edited).
    #[test]
    fn policy_args_follow_the_apps_policy() {
        let d = temp("webpolicy");
        let (pol, cfg) = (d.join("webpolicy"), d.join("config"));
        std::fs::create_dir_all(&pol).unwrap();
        assert!(policy_args(&pol, &cfg, "Chat App").is_empty(), "no policy");
        std::fs::write(
            pol.join("chat-app.json"),
            r#"{"heap_mb": 1024, "patches": []}"#,
        )
        .unwrap();
        let args = policy_args(&pol, &cfg, "Chat App");
        let all = wol_core::webruntime::flags(
            wol_core::webruntime::Profile::Balanced,
            wol_core::webruntime::Role::App,
            wol_core::webruntime::Gpu::Soft,
        );
        assert!(args.iter().all(|a| all.contains(a)), "{args:?}");
        assert!(
            args.contains(&"--ozone-platform-hint=auto".to_owned()),
            "{args:?}"
        );
        for a in &args {
            assert!(a.starts_with("--") && !a.contains([' ', '"', '%']), "{a}");
        }
        std::fs::write(pol.join("chat-app.json"), r#"{"heap_mb": 1}"#).unwrap();
        assert!(
            policy_args(&pol, &cfg, "Chat App").is_empty(),
            "refused policy"
        );
    }

    /// Audit S4-15: the gpg scratch directory is new, 0700, never a directory someone made first,
    /// its files are created new, and it is gone after use.
    #[test]
    fn gpg_scratch_is_private_fresh_and_removed() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("wopkg-scratch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let a = Scratch::new(&base).unwrap();
        let b = Scratch::new(&base).unwrap();
        assert_ne!(a.0, b.0, "never the same name twice");
        let mode = std::fs::metadata(&a.0).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        a.write("key.asc", b"k").unwrap();
        assert!(
            a.write("key.asc", b"again").is_err(),
            "files are created new"
        );
        std::os::unix::fs::symlink("/etc/passwd", a.0.join("sig.asc")).unwrap();
        assert!(
            a.write("sig.asc", b"x").is_err(),
            "a planted link is not followed"
        );
        let (pa, pb) = (a.0.clone(), b.0.clone());
        drop(a);
        drop(b);
        assert!(!pa.exists() && !pb.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A minimal 64-bit ELF AppImage with the given sections (name, contents).
    pub fn fake_appimage(payload: &[u8], secs: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[8..11].copy_from_slice(b"AI\x02");
        b.extend_from_slice(payload);
        let mut strtab = vec![0u8];
        let mut hdrs = vec![(0u32, 0usize, 0usize)];
        for (n, data) in secs {
            let name_off = strtab.len() as u32;
            strtab.extend_from_slice(n.as_bytes());
            strtab.push(0);
            hdrs.push((name_off, b.len(), data.len()));
            b.extend_from_slice(data);
        }
        let sh_name = strtab.len() as u32;
        strtab.extend_from_slice(b".shstrtab\0");
        hdrs.push((sh_name, b.len(), strtab.len()));
        b.extend_from_slice(&strtab);
        let shoff = b.len();
        for (name, off, size) in &hdrs {
            let mut h = [0u8; 64];
            h[..4].copy_from_slice(&name.to_le_bytes());
            h[0x18..0x20].copy_from_slice(&(*off as u64).to_le_bytes());
            h[0x20..0x28].copy_from_slice(&(*size as u64).to_le_bytes());
            b.extend_from_slice(&h);
        }
        b[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        b[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        b[0x3c..0x3e].copy_from_slice(&(hdrs.len() as u16).to_le_bytes());
        b[0x3e..0x40].copy_from_slice(&((hdrs.len() - 1) as u16).to_le_bytes());
        b
    }

    pub fn ctx(d: &Path) -> Context {
        Context::new(crate::context::Config {
            home: Some(d.to_path_buf()),
            cache_dir: Some(d.join("cache")),
            state_dir: Some(d.join("state")),
            data_dir: Some(d.join("data")),
            allow_loopback_http: true,
            ..Default::default()
        })
        .unwrap()
    }

    /// QA-STORE-26: a URL's last part is never `.`/`..`, a hidden name, or a path.
    #[test]
    fn url_file_name_is_sanitised() {
        assert_eq!(
            url_file_name("https://x/a/Foo-1.AppImage"),
            "Foo-1.AppImage"
        );
        assert_eq!(
            url_file_name("https://x/a/Foo.AppImage?x=1#y"),
            "Foo.AppImage"
        );
        for bad in [
            "https://x/..",
            "https://x/.",
            "https://x/",
            "https://x/a/..",
        ] {
            assert_eq!(url_file_name(bad), "app.AppImage", "{bad}");
        }
        assert_eq!(url_file_name("https://x/.hidden"), "hidden");
        assert_eq!(url_file_name("https://x/a\\b"), "ab");
    }

    #[test]
    fn update_info_forms() {
        let cases = [
            (
                "zsync|https://x/a.zsync",
                Some(UpdateInfo::Zsync {
                    url: "https://x/a.zsync".into(),
                }),
            ),
            (
                "gh-releases-zsync|o|r|latest|App-*x86_64.AppImage.zsync",
                Some(UpdateInfo::GhReleasesZsync {
                    owner: "o".into(),
                    repo: "r".into(),
                    tag: "latest".into(),
                    file: "App-*x86_64.AppImage.zsync".into(),
                }),
            ),
            ("gh-releases-zsync|o/../x|r|latest|f", None),
            ("bintray-zsync|a|b|c|d", None),
            ("", None),
        ];
        for (s, want) in cases {
            assert_eq!(UpdateInfo::parse(s), want, "{s}");
        }
        assert!(glob(
            "App-*x86_64.AppImage.zsync",
            "App-1.2-x86_64.AppImage.zsync"
        ));
        assert!(!glob(
            "App-*x86_64.AppImage.zsync",
            "App-1.2-aarch64.AppImage.zsync"
        ));
        assert!(glob("*", "x") && glob("a*b*c", "aXbYc") && !glob("a*b*c", "aXcYb"));
    }

    #[test]
    fn names_from_files() {
        for (f, n) in [
            ("Krita-5.2.6-x86_64.AppImage", "Krita"),
            ("Obsidian-1.6.7.AppImage", "Obsidian"),
            ("nvim.appimage", "nvim"),
            ("LM-Studio-0.3.5.AppImage", "LM-Studio"),
            ("app-v2.0.AppImage", "app"),
        ] {
            assert_eq!(name_from_file(f), n, "{f}");
        }
    }

    #[test]
    fn inspect_reads_sections_without_running() {
        let b = fake_appimage(
            b"payload",
            &[
                (".upd_info", b"zsync|https://e/x.zsync\0\0\0"),
                (".sha256_sig", b"\0\0"),
                (".sig_key", b""),
            ],
        );
        let d = temp("ai-inspect");
        std::fs::write(d.join("a"), &b).unwrap();
        let i = inspect(&d.join("a")).unwrap();
        assert!(i.appimage && !i.signed);
        assert_eq!(
            i.update,
            Some(UpdateInfo::Zsync {
                url: "https://e/x.zsync".into()
            })
        );
        assert!(!verify_signature(&d.join("a"), &i, None).unwrap());
        assert!(
            verify_signature(&d.join("a"), &i, Some("pin")).is_err(),
            "unsigned after signed is refused"
        );
    }

    #[test]
    fn install_adopt_update_rollback() {
        let d = temp("ai-flow");
        let c = ctx(&d);
        let payload_v1 = crate::zsync::tests::noise(150_000, 2);
        let mut payload_v2 = payload_v1.clone();
        payload_v2[70_000..70_016].copy_from_slice(b"version-two-here");
        // The update information inside both files names the loopback server, so learn its port first.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let u = format!("zsync|http://127.0.0.1:{port}/App.AppImage.zsync");
        let new = fake_appimage(&payload_v2, &[(".upd_info", u.as_bytes())]);
        let old = fake_appimage(&payload_v1, &[(".upd_info", u.as_bytes())]);
        let z = crate::zsync::make(&new, "App.AppImage", "App.AppImage", 2048);
        let _srv = crate::testutil::serve_on(
            port,
            vec![
                ("/App.AppImage", Served::bytes(new.clone())),
                ("/App.AppImage.zsync", Served::bytes(z)),
            ],
        );
        let srv_holder = (new, old);
        std::fs::create_dir_all(&c.apps).unwrap();
        std::fs::write(c.apps.join("App-1.0-x86_64.AppImage"), &srv_holder.1).unwrap();
        assert_eq!(adopt(&c), ["App"]);
        let r = update(
            &c,
            &UpdateRequest {
                check_only: true,
                ..Default::default()
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            r[0].state,
            UpdateState::Available {
                size: srv_holder.0.len() as u64
            }
        );
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        let UpdateState::Updated {
            fetched_bytes,
            journal,
            ..
        } = &r[0].state
        else {
            panic!("{:?}", r[0].state)
        };
        assert!(
            *fetched_bytes < 10_000,
            "delta, not a full download: {fetched_bytes}"
        );
        let path = c.apps.join("App-1.0-x86_64.AppImage");
        assert_eq!(std::fs::read(&path).unwrap(), srv_holder.0);
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        assert_eq!(r[0].state, UpdateState::UpToDate);
        crate::journal::rollback(&c, journal).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            srv_holder.1,
            "rollback restores the previous file"
        );
        let reg = Registry::load(&c);
        assert_eq!(
            reg.apps[0].superseded.len(),
            1,
            "v1 is remembered as replaced"
        );
    }

    /// Audit A14: an update may not bring back a version that was already replaced, nor one whose
    /// control file is older than the last one applied.
    #[test]
    fn downgrades_are_refused() {
        let d = temp("ai-downgrade");
        let c = ctx(&d);
        let v1 = fake_appimage(&crate::zsync::tests::noise(20_000, 5), &[]);
        let v2 = fake_appimage(&crate::zsync::tests::noise(20_000, 6), &[]);
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        // A control file for v1 with an MTime (zsyncmake writes one; `make` does not).
        let raw = crate::zsync::make(&v1, "A", "A.AppImage", 2048);
        let cut = raw
            .windows(2)
            .position(|w| {
                w == b"

"
            })
            .unwrap();
        let head = String::from_utf8_lossy(&raw[..cut]).replacen(
            "SHA-1:",
            "MTime: Mon, 02 Nov 2026 01:00:00 +0000
SHA-1:",
            1,
        );
        let z = [head.as_bytes(), &raw[cut..]].concat();
        let _srv = crate::testutil::serve_on(
            port,
            vec![
                ("/A.AppImage", Served::bytes(v1.clone())),
                ("/A.AppImage.zsync", Served::bytes(z)),
                ("/plain.zsync", Served::bytes(raw.clone())),
            ],
        );
        std::fs::create_dir_all(&c.apps).unwrap();
        let path = c.apps.join("A.AppImage");
        std::fs::write(&path, &v2).unwrap();
        let sha = |b: &[u8]| crate::fetch::hex(&Sha256::digest(b));
        let mut reg = Registry::default();
        let base = App {
            name: "A".into(),
            path: path.clone(),
            sha256: sha(&v2),
            update: Some(format!("zsync|http://127.0.0.1:{port}/A.AppImage.zsync")),
            key_pin: None,
            signed: false,
            url: None,
            mtime: None,
            superseded: vec![sha(&v1)],
        };
        reg.upsert(base.clone());
        reg.save(&c).unwrap();
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        let UpdateState::Failed { message } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert!(message.contains("already updated from"), "{message}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            v2,
            "the installed file stays"
        );
        // Same server, but the last update had a newer control file.
        reg.upsert(App {
            superseded: Vec::new(),
            mtime: Some(1_900_000_000),
            ..base.clone()
        });
        reg.save(&c).unwrap();
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        let UpdateState::Failed { message } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert!(message.contains("older version"), "{message}");
        // Audit S12: with a floor set, a control file that leaves MTime out is refused too.
        reg.upsert(App {
            superseded: Vec::new(),
            mtime: Some(1_700_000_000),
            update: Some(format!("zsync|http://127.0.0.1:{port}/plain.zsync")),
            ..base
        });
        reg.save(&c).unwrap();
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        let UpdateState::Failed { message } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert!(message.contains("cannot be told"), "{message}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            v2,
            "the installed file stays"
        );
    }

    #[test]
    fn the_plain_zsync_wins_when_a_release_has_two_that_match() {
        // Real release: `App-2.8-1-x86_64.AppImage.zsync` points at `App-2.8-x86_64.AppImage`.
        let body = r#"{"assets":[
          {"name":"App-2.8-1-x86_64.AppImage.zsync","browser_download_url":"https://e/App-2.8-1-x86_64.AppImage.zsync"},
          {"name":"App-2.8-x86_64.AppImage.zsync","browser_download_url":"https://e/App-2.8-x86_64.AppImage.zsync"}]}"#;
        let srv = crate::testutil::serve(vec![(
            "/repos/o/r/releases/latest",
            Served::bytes(body.as_bytes().to_vec()),
        )]);
        let d = temp("ai-glob");
        let c = ctx(&d);
        let u =
            UpdateInfo::parse("gh-releases-zsync|o|r|latest|App-*x86_64.AppImage.zsync").unwrap();
        assert_eq!(
            zsync_url(&c, &u, &srv.url("")).unwrap(),
            "https://e/App-2.8-x86_64.AppImage.zsync"
        );
    }

    /// Audit8 stability2 #9: the copy fallback of `move_file` lands under a temporary name and is
    /// renamed in; nothing is left half way, and a failed copy leaves the target as it was.
    #[test]
    fn the_copy_fallback_renames_a_whole_file_into_place() {
        let d = temp("ai-copy");
        let (from, to) = (d.join("a.AppImage"), d.join("b.AppImage"));
        std::fs::write(&from, vec![7u8; 300_000]).unwrap();
        std::fs::write(&to, b"old").unwrap();
        // A leftover from a crash is replaced, not appended to.
        std::fs::write(d.join("b.AppImage.tmp"), b"half").unwrap();
        copy_then_rename(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), vec![7u8; 300_000]);
        assert!(!d.join("b.AppImage.tmp").exists());
        assert!(
            from.exists(),
            "the caller removes the source after the rename"
        );
        let gone = d.join("missing");
        assert!(copy_then_rename(&gone, &to).is_err());
        assert_eq!(
            std::fs::read(&to).unwrap().len(),
            300_000,
            "target untouched"
        );
        assert!(!d.join("b.AppImage.tmp").exists());
    }
}
