//! libwopkg (ADR 0021, archicture/modules/wopkg.md): the fast and security-critical core of wopkg
//! behind a stable C ABI (`include/wopkg.h`, `libwopkg.so.1`, symbols versioned by
//! `abi/wopkg.map`). Requests and results are JSON documents; new fields are only ever added.
//! Rules at the boundary (ADR 0020): opaque handles, UTF-8 with lengths, a status and an optional
//! error object from every call that can fail, panics stopped here, progress callbacks on the
//! calling thread. The library holds no privilege: system changes are wold calls.
#![allow(non_camel_case_types)]

pub mod appimage;
pub mod context;
pub mod fetch;
pub mod ffi;
pub mod index;
pub mod journal;
pub mod release;
pub mod sync;
pub mod system;
#[cfg(test)]
pub mod testutil;
pub mod zsync;

use ffi::{
    Error, Result, give, give_json, guard, input, need, request, wopkg_error, wopkg_progress_fn,
    wopkg_status, wopkg_string,
};
use std::ffi::{c_char, c_void};

pub const VERSION: (u32, u32, u32) = (1, 0, 0);

/// Every `#define` in `wopkg.h` (test-checked against the header).
pub mod abi {
    use crate::ffi::Status;
    pub const CONSTANTS: &[(&str, i64)] = &[
        ("WOPKG_VERSION_MAJOR", crate::VERSION.0 as i64),
        ("WOPKG_VERSION_MINOR", crate::VERSION.1 as i64),
        ("WOPKG_VERSION_PATCH", crate::VERSION.2 as i64),
        ("WOPKG_OK", Status::Ok as i64),
        ("WOPKG_ERR_INVALID_ARGUMENT", Status::InvalidArgument as i64),
        (
            "WOPKG_ERR_PERMISSION_DENIED",
            Status::PermissionDenied as i64,
        ),
        ("WOPKG_ERR_UNAVAILABLE", Status::Unavailable as i64),
        ("WOPKG_ERR_INTERNAL", Status::Internal as i64),
        ("WOPKG_ERR_UNTRUSTED", Status::Untrusted as i64),
        ("WOPKG_ERR_NOT_FOUND", Status::NotFound as i64),
    ];
}

/// Opaque context: paths and network policy (`context::Context`).
pub struct wopkg_context(context::Context);

const MAX_QUERY: usize = 1024;

/// Runtime version, packed `major << 16 | minor << 8 | patch`.
#[unsafe(no_mangle)]
pub extern "C" fn wopkg_version() -> u32 {
    (VERSION.0 << 16) | (VERSION.1 << 8) | VERSION.2
}

/// "1.0.0", static.
#[unsafe(no_mangle)]
pub extern "C" fn wopkg_version_string() -> *const c_char {
    c"1.0.0".as_ptr()
}

unsafe fn ctx<'a>(c: *const wopkg_context) -> Result<&'a context::Context> {
    // SAFETY: NULL or a live context (contract).
    unsafe { c.as_ref() }
        .map(|c| &c.0)
        .ok_or_else(|| Error::invalid("The context is NULL."))
}

/// # Safety
/// `config` points at `len` bytes (or NULL with 0); `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_context_create(
    config: *const c_char,
    len: usize,
    out: *mut *mut wopkg_context,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c: context::Config = request(config, len)?;
        let c = context::Context::new(c)?;
        // SAFETY: checked non-NULL.
        unsafe { *out = Box::into_raw(Box::new(wopkg_context(c))) };
        Ok(())
    })
}

/// # Safety
/// `context` is NULL or a context the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_context_release(context: *mut wopkg_context) {
    if !context.is_null() {
        // SAFETY: from Box::into_raw in create (contract).
        drop(unsafe { Box::from_raw(context) });
    }
}

/// Fetch changed source inputs and rebuild their index segments (`sync::Request` -> reports).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_index_update(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    progress: wopkg_progress_fn,
    user_data: *mut c_void,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let r: sync::Request = request(req, len)?;
        let mut p = ffi::progress_of(progress, user_data);
        let reports = sync::run(c, &r, &mut p)?;
        give_json(out, &serde_json::json!({ "sources": reports }))
    })
}

pub fn search(c: &context::Context, query: &str, limit: usize) -> serde_json::Value {
    let t = std::time::Instant::now();
    let segs = c.segments();
    let inst = c.installed();
    let hits = index::search::search(&segs, query, limit, &|r| inst.has(r));
    serde_json::json!({ "query": query, "results": hits, "took_us": t.elapsed().as_micros() as u64 })
}

/// Unified search: one ranked result per app with every source that has it.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_search(
    context: *const wopkg_context,
    query: *const c_char,
    len: usize,
    limit: u32,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let q = input(query, len, MAX_QUERY, "The search")?;
        give_json(out, &search(c, q, limit.clamp(1, 1000) as usize))
    })
}

pub fn info(c: &context::Context, name: &str) -> Result<serde_json::Value> {
    let segs = c.segments();
    let inst = c.installed();
    let n = name.trim().to_lowercase();
    let hits = index::search::search(&segs, &n, 50, &|r| inst.has(r));
    let hit = hits
        .into_iter()
        .find(|h| {
            h.key == n
                || h.sources
                    .iter()
                    .any(|s| s.record.name.to_lowercase() == n || s.record.id.to_lowercase() == n)
        })
        .ok_or_else(|| {
            Error::new(
                ffi::Status::NotFound,
                format!("No app or package is called “{name}”."),
            )
        })?;
    Ok(serde_json::to_value(hit).unwrap_or_default())
}

/// One app by name, package name or app id, with every source.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_info(
    context: *const wopkg_context,
    name: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give_json(out, &info(c, input(name, len, MAX_QUERY, "The name")?)?)
    })
}

/// Package names starting with `prefix`, one per line (shell completion).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_complete(
    context: *const wopkg_context,
    prefix: *const c_char,
    len: usize,
    limit: u32,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let p = input(prefix, len, MAX_QUERY, "The prefix")?;
        let names = index::search::complete(&c.segments(), p, limit.clamp(1, 100_000) as usize);
        give(out, names.join("\n"));
        Ok(())
    })
}

/// What is installed, per source.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_installed(
    context: *const wopkg_context,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let i = c.installed();
        let sorted = |s: &std::collections::HashSet<String>| {
            let mut v: Vec<&String> = s.iter().collect();
            v.sort();
            v.into_iter().cloned().collect::<Vec<_>>()
        };
        give_json(
            out,
            &serde_json::json!({
                "pacman": sorted(&i.pacman), "flatpak": sorted(&i.flatpak), "wol": sorted(&i.wol),
                "recipe": sorted(&i.recipes), "appimage": appimage::Registry::load(c).apps,
            }),
        )
    })
}

/// Parallel, resumed, checked downloads (`fetch::Request` -> per-item outcomes).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_fetch(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    progress: wopkg_progress_fn,
    user_data: *mut c_void,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let mut r: fetch::Request = request(req, len)?;
        r.jobs = r.jobs.or(Some(c.jobs));
        let mut p = ffi::progress_of(progress, user_data);
        give_json(
            out,
            &serde_json::json!({ "items": fetch::many(&c.net, &r, &mut p) }),
        )
    })
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum VerifyReq {
    Sha256 {
        path: std::path::PathBuf,
        sha256: String,
    },
    /// A `.wol` file (ADR 0011) against the trusted keys.
    Wol { path: std::path::PathBuf },
    /// Raw Ed25519 signature file of `path` (indexes, recipe repos).
    Ed25519 {
        path: std::path::PathBuf,
        signature: std::path::PathBuf,
    },
}

pub fn verify(c: &context::Context, body: &str) -> Result<serde_json::Value> {
    let r: VerifyReq = serde_json::from_str(body)
        .map_err(|e| Error::invalid(format!("The request is not valid: {e}.")))?;
    let keys = || wol_core::package::read_keys(&c.keys);
    match r {
        VerifyReq::Sha256 { path, sha256 } => {
            let got = fetch::sha256_file(&path)
                .ok_or_else(|| Error::new(ffi::Status::NotFound, "That file cannot be read."))?;
            if got != sha256.to_ascii_lowercase() {
                return Err(Error::untrusted("The file does not match its checksum."));
            }
            Ok(serde_json::json!({ "trust": "checksum" }))
        }
        VerifyReq::Wol { path } => {
            let v = wol_core::package::verify(&path, &keys()).map_err(Error::untrusted)?;
            let signed = matches!(v.trust, wol_core::package::Trust::Signed(_));
            Ok(
                serde_json::json!({ "trust": if signed { "signed" } else { "unsigned" }, "info": v.info }),
            )
        }
        VerifyReq::Ed25519 { path, signature } => {
            let text = std::fs::read(&path)
                .map_err(|_| Error::new(ffi::Status::NotFound, "That file cannot be read."))?;
            let sig = std::fs::read(&signature)
                .map_err(|_| Error::untrusted("The signature is missing."))?;
            match wol_core::package::verify_index(&text, Some(&sig), &keys()) {
                Ok(wol_core::package::Trust::Signed(_)) => {
                    Ok(serde_json::json!({ "trust": "signed" }))
                }
                _ => Err(Error::untrusted(
                    "The signature does not match a WolOS key.",
                )),
            }
        }
    }
}

/// Check a file: SHA-256, `.wol` package, or Ed25519 signature (`{"kind":...}`).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_verify(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give_json(
            out,
            &verify(c, input(req, len, ffi::MAX_REQUEST, "The request")?)?,
        )
    })
}

/// Update information, signature and hashes of an AppImage, without running it.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_appimage_inspect(
    context: *const wopkg_context,
    path: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        unsafe { ctx(context) }?;
        let p = input(path, len, 4096, "The path")?;
        give_json(out, &appimage::inspect(std::path::Path::new(p))?)
    })
}

/// Install an AppImage from a URL, or adopt a file, into `~/Applications`.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_appimage_install(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    progress: wopkg_progress_fn,
    user_data: *mut c_void,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let r: appimage::InstallRequest = request(req, len)?;
        let mut p = ffi::progress_of(progress, user_data);
        give_json(out, &appimage::install(c, &r, &mut p)?)
    })
}

/// Check or update AppImages by zsync delta (`appimage::UpdateRequest`).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_appimage_update(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    progress: wopkg_progress_fn,
    user_data: *mut c_void,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let r: appimage::UpdateRequest = request(req, len)?;
        let mut p = ffi::progress_of(progress, user_data);
        give_json(
            out,
            &serde_json::json!({ "apps": appimage::update(c, &r, &mut p)? }),
        )
    })
}

/// Remove an installed AppImage by name (undo with `wopkg_rollback`); `out` = the history id.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_appimage_remove(
    context: *const wopkg_context,
    name: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give(
            out,
            appimage::remove(c, input(name, len, 256, "The name")?)?,
        );
        Ok(())
    })
}

/// Every recorded change, oldest first.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_history(
    context: *const wopkg_context,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give_json(out, &serde_json::json!({ "entries": journal::list(c) }))
    })
}

/// Record a change the caller made (Flatpak, AUR, recipe); `out` = its id.
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_history_record(
    context: *const wopkg_context,
    entry: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        let e: journal::Entry = request(entry, len)?;
        give(out, journal::record(c, e)?);
        Ok(())
    })
}

/// Undo change `id`: file swaps happen here; `out` is the entry, whose `undo` says what is left
/// for the caller (`run`: a command; `system`: a wold rollback).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_rollback(
    context: *const wopkg_context,
    id: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give_json(out, &journal::rollback(c, input(id, len, 64, "The id")?)?)
    })
}

/// One system change through wold (`shared::packages::PackageCall` JSON).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_system_call(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        let c = unsafe { ctx(context) }?;
        let body = input(req, len, shared::portal::MAX_LINE, "The request")?;
        let call =
            shared::packages::PackageCall::decode(body.as_bytes()).map_err(Error::invalid)?;
        system::call(c, &call)
    })
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PlanReq {
    /// Feed base URL; `<base>/<channel>.json` and `.json.sig`.
    base_url: Option<String>,
    channel: Option<String>,
    to: Option<String>,
    current: Option<String>,
    machine_id: Option<String>,
    paused: Option<bool>,
}

pub const FEEDS: &str = "https://wonderlandos.github.io/releases";

pub fn release_plan(c: &context::Context, body: &str) -> Result<serde_json::Value> {
    let r: PlanReq = if body.trim().is_empty() {
        PlanReq::default()
    } else {
        serde_json::from_str(body)
            .map_err(|e| Error::invalid(format!("The request is not valid: {e}.")))?
    };
    let channel = r
        .channel
        .or_else(|| {
            std::fs::read_to_string("/etc/wolos/channel")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("stable".into());
    if !release::CHANNELS.contains(&channel.as_str()) {
        return Err(Error::invalid("The channel is stable, beta or nightly."));
    }
    let current = r
        .current
        .or_else(|| {
            let t = std::fs::read_to_string(&c.release_file).ok()?;
            serde_json::from_str::<serde_json::Value>(&t).ok()?["version"]
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(|| Error::unavailable("This system's release is unknown."))?;
    let machine = r
        .machine_id
        .or_else(|| {
            std::fs::read_to_string("/etc/machine-id")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_default();
    let paused = r.paused.unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        std::fs::read_to_string(c.state.join("pause"))
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
            .is_some_and(|until| until > now)
    });
    let base = r.base_url.unwrap_or(FEEDS.into());
    let dir = c.cache.join("releases");
    let (feed, sig) = (
        dir.join(format!("{channel}.json")),
        dir.join(format!("{channel}.json.sig")),
    );
    for (url, dest) in [
        (format!("{base}/{channel}.json"), &feed),
        (format!("{base}/{channel}.json.sig"), &sig),
    ] {
        if let Err(e) = fetch::conditional(&c.net, &url, dest)
            && !dest.exists()
        {
            return Err(e);
        }
    }
    let text =
        std::fs::read(&feed).map_err(|_| Error::unavailable("The release list cannot be read."))?;
    let s =
        std::fs::read(&sig).map_err(|_| Error::untrusted("The release list has no signature."))?;
    let f = release::Feed::verify(&text, &s, &wol_core::package::read_keys(&c.keys))?;
    let p = release::plan(&release::Inputs {
        current: &current,
        feed: &f,
        to: r.to.as_deref(),
        machine_id: &machine,
        paused,
    })?;
    Ok(serde_json::to_value(p).unwrap_or_default())
}

/// What `wopkg update` should do with the system release (signed channel feed, rollout, pause).
/// # Safety
/// Pointers as documented in `wopkg.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wopkg_release_plan(
    context: *const wopkg_context,
    req: *const c_char,
    len: usize,
    out: *mut *mut wopkg_string,
    error: *mut *mut wopkg_error,
) -> wopkg_status {
    guard(error, || {
        need(out, "out")?;
        let c = unsafe { ctx(context) }?;
        give_json(
            out,
            &release_plan(c, input(req, len, ffi::MAX_REQUEST, "The request")?)?,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_agrees() {
        let s = unsafe { std::ffi::CStr::from_ptr(wopkg_version_string()) };
        assert_eq!(
            s.to_str().unwrap(),
            format!("{}.{}.{}", VERSION.0, VERSION.1, VERSION.2)
        );
        assert_eq!(wopkg_version(), 0x01_00_00);
    }

    #[test]
    fn release_plan_from_a_signed_feed() {
        let d = testutil::temp("plan");
        let seed = [5u8; 32];
        std::fs::create_dir_all(d.join("keys")).unwrap();
        std::fs::write(
            d.join("keys/wolos.pub"),
            wol_core::package::public_pem(&wol_core::package::public_of(&seed)),
        )
        .unwrap();
        let feed = br#"{"format":1,"channel":"beta","releases":[{"version":"1.1.0-beta.2","manifest":"{}","signature":"00"}]}"#.to_vec();
        let sig = wol_core::package::sign(&seed, &feed).to_vec();
        let srv = testutil::serve(vec![
            ("/beta.json", testutil::Served::bytes(feed)),
            ("/beta.json.sig", testutil::Served::bytes(sig)),
        ]);
        let mut c = appimage::tests::ctx(&d);
        c.keys = d.join("keys");
        let body = format!(
            r#"{{"base_url":"{}","channel":"beta","current":"1.0.0","machine_id":"m","paused":false}}"#,
            srv.url("")
        );
        let p = release_plan(&c, &body).unwrap();
        assert_eq!(p["reason"], "available");
        assert_eq!(p["target"]["version"], "1.1.0-beta.2");
        let bad = body.replace("beta\",\"current", "canary\",\"current");
        assert!(release_plan(&c, &bad).is_err());
    }
}
