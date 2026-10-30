//! System facts an app may read without asking: OS name and version, kernel, appearance and accent.
//! All of it is the person's own settings or world-readable files, so no service call is needed.
use crate::context::wol_context;
use crate::error::{Result, guard, need, quiet, wol_error, wol_status};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::theme::{Color, Variant};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wol_core::theme::{ThemeCatalog, accent, active};

pub type wol_appearance = i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Appearance {
    Light = 0,
    Dark = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
    pub appearance: Appearance,
    pub accent_name: String,
    /// 0xRRGGBB.
    pub accent_rgb: u32,
}

/// `$XDG_CONFIG_HOME`, else `$HOME/.config`.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".config")
        })
}

/// Appearance, accent id and colour the session shows for a saved theme id at local `hour`
/// (the same resolution the shell and `wol theme` use).
pub fn appearance(saved: Option<&str>, hour: u8) -> (Appearance, &'static str, u32) {
    let auto_dark = accent::auto_is_dark(hour);
    let id = active::effective(accent::effective(saved, auto_dark).as_deref(), auto_dark);
    let catalog = accent::variants()
        .into_iter()
        .fold(ThemeCatalog::builtin(), ThemeCatalog::with);
    let theme = catalog.resolve(&id).ok();
    let variant = theme.as_ref().map_or_else(
        || accent::split(saved).0.unwrap_or(Variant::Light),
        |t| t.variant,
    );
    let name = accent::split(saved).1;
    let hex = theme
        .as_ref()
        .and_then(|t| t.tokens.get("accent").cloned())
        .or_else(|| accent::color(variant, name).map(str::to_owned))
        .unwrap_or_default();
    let rgb = Color::parse(&hex).map_or(0, |c| {
        (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
    });
    let a = if variant == Variant::Dark {
        Appearance::Dark
    } else {
        Appearance::Light
    };
    (a, name, rgb)
}

/// (NAME, VERSION_ID) of an os-release file.
pub fn os_release(text: &str) -> (String, String) {
    let get = |k: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| key.trim() == k)
            .map(|(_, v)| v.trim().trim_matches('"').to_owned())
    };
    (
        get("NAME").unwrap_or_else(|| "WolOS".into()),
        get("VERSION_ID")
            .or_else(|| get("VERSION"))
            .unwrap_or_default(),
    )
}

fn local_hour() -> u8 {
    // SAFETY: time(NULL) and localtime_r write only into the zeroed `tm`.
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 12;
    }
    tm.tm_hour.clamp(0, 23) as u8
}

/// This machine and session now.
pub fn read(config_home: &Path) -> Info {
    let r = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let os =
        std::fs::read_to_string("/etc/os-release").unwrap_or_else(|_| r("/usr/lib/os-release"));
    let (os_name, os_version) = os_release(&os);
    let (ostype, osrelease) = (
        r("/proc/sys/kernel/ostype"),
        r("/proc/sys/kernel/osrelease"),
    );
    let kernel = wol_core::system::info::SystemInfo::from(&wol_core::system::info::Facts {
        ostype: &ostype,
        osrelease: &osrelease,
        ..Default::default()
    })
    .kernel;
    let saved = active::get(&active::default_file(config_home));
    let (appearance, accent_name, accent_rgb) = appearance(saved.as_deref(), local_hour());
    Info {
        os_name,
        os_version,
        kernel,
        appearance,
        accent_name: accent_name.into(),
        accent_rgb,
    }
}

pub struct wol_system_info {
    os_name: Arc<wol_string>,
    os_version: Arc<wol_string>,
    kernel: Arc<wol_string>,
    accent_name: Arc<wol_string>,
    appearance: Appearance,
    accent_rgb: u32,
}

impl From<Info> for wol_system_info {
    fn from(i: Info) -> Self {
        Self {
            os_name: wol_string::new(&i.os_name),
            os_version: wol_string::new(&i.os_version),
            kernel: wol_string::new(&i.kernel),
            accent_name: wol_string::new(&i.accent_name),
            appearance: i.appearance,
            accent_rgb: i.accent_rgb,
        }
    }
}

/// # Safety
/// `context` is live; `out` and `error` are writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_copy(
    context: *mut wol_context,
    out: *mut *mut wol_system_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let info: wol_system_info = read(ctx.config_home()).into();
        unsafe { *out = into_raw(Arc::new(info)) };
        Ok(())
    })
}

fn get<'a>(info: *const wol_system_info) -> Result<&'a wol_system_info> {
    unsafe { borrow(info) }
}

fn empty() -> *const wol_string {
    static EMPTY: std::sync::OnceLock<Arc<wol_string>> = std::sync::OnceLock::new();
    Arc::as_ptr(EMPTY.get_or_init(|| wol_string::new("")))
}

fn field(
    info: *const wol_system_info,
    f: fn(&wol_system_info) -> &Arc<wol_string>,
) -> *const wol_string {
    quiet(empty(), || get(info).map_or(empty(), |i| Arc::as_ptr(f(i))))
}

/// # Safety
/// `info` is NULL or live. The result is borrowed from `info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_os_name(
    info: *const wol_system_info,
) -> *const wol_string {
    field(info, |i| &i.os_name)
}

/// # Safety
/// `info` is NULL or live. The result is borrowed from `info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_os_version(
    info: *const wol_system_info,
) -> *const wol_string {
    field(info, |i| &i.os_version)
}

/// # Safety
/// `info` is NULL or live. The result is borrowed from `info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_kernel(
    info: *const wol_system_info,
) -> *const wol_string {
    field(info, |i| &i.kernel)
}

/// # Safety
/// `info` is NULL or live. The result is borrowed from `info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_accent_name(
    info: *const wol_system_info,
) -> *const wol_string {
    field(info, |i| &i.accent_name)
}

/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_appearance(
    info: *const wol_system_info,
) -> wol_appearance {
    get(info).map_or(Appearance::Light as i32, |i| i.appearance as i32)
}

/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_get_accent_rgb(info: *const wol_system_info) -> u32 {
    get(info).map_or(0, |i| i.accent_rgb)
}

/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_retain(
    info: *mut wol_system_info,
) -> *mut wol_system_info {
    unsafe { retain(info) }
}

/// # Safety
/// `info` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_system_info_release(info: *mut wol_system_info) {
    unsafe { release(info) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_table() {
        let cases: [(Option<&str>, u8, Appearance, &str, u32); 6] = [
            (
                Some("wol-dark-blue"),
                12,
                Appearance::Dark,
                "blue",
                0x0a84ff,
            ),
            (
                Some("wol-light-blue"),
                23,
                Appearance::Light,
                "blue",
                0x007aff,
            ),
            (Some("wol-light"), 12, Appearance::Light, "violet", 0x7c5cff),
            (None, 12, Appearance::Light, "violet", 0x7c5cff),
            (None, 22, Appearance::Dark, "violet", 0x8b6cff),
            (
                Some("wol-auto-green"),
                3,
                Appearance::Dark,
                "green",
                0x30d158,
            ),
        ];
        for (saved, hour, a, name, rgb) in cases {
            assert_eq!(appearance(saved, hour), (a, name, rgb), "{saved:?} {hour}");
        }
    }

    #[test]
    fn os_release_table() {
        assert_eq!(
            os_release("NAME=\"WolOS\"\nVERSION_ID=0.11\nPRETTY_NAME=\"WolOS 0.11\"\n"),
            ("WolOS".into(), "0.11".into())
        );
        assert_eq!(os_release(""), ("WolOS".into(), String::new()));
        assert_eq!(
            os_release("NAME=Arch Linux\nVERSION=rolling"),
            ("Arch Linux".into(), "rolling".into())
        );
    }

    #[test]
    fn real_host_reads() {
        let i = read(Path::new("/nonexistent"));
        assert!(!i.os_name.is_empty() && !i.kernel.is_empty());
    }
}
