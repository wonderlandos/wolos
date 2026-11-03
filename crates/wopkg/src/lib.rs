// SPDX-License-Identifier: GPL-2.0-or-later
//! libwopkg (ADR 0021, docs/modules/wopkg.md): the fast and security-critical core of wopkg
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

/// Record a change the caller made (Flatpak, community package, recipe); `out` = its id.
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

/// Release feed base: `<channel>.json` and `<channel>.json.sig` of every channel in the CDN's
/// `latest/` folder, which the release tooling (wolos-build) writes after each publication is
/// uploaded and verified, so it always holds every channel.
pub const FEEDS: &str = "https://wolos.squareweb.app/latest";
/// Where the feed is read when [`FEEDS`] answers 404 or cannot be reached: the same files as
/// assets of the latest wolos-packages release on GitHub (it answers with a redirect to the
/// asset; `fetch` follows it). Both are checked against the same keys.
pub const FEEDS_FALLBACK: &str =
    "https://github.com/wonderlandos/wolos-packages/releases/latest/download";

/// Feed bases in the order tried: the request's own base alone, else the CDN then GitHub.
fn feed_bases(base_url: Option<String>) -> Vec<String> {
    base_url.map_or_else(
        || vec![FEEDS.to_owned(), FEEDS_FALLBACK.to_owned()],
        |b| vec![b],
    )
}

/// A fetch error after which the next feed base is tried: the file is not there (404) or the
/// server could not be reached or answered with an error. A body that is too large, or any
/// check of the feed itself, stops there.
fn try_next_base(e: &Error) -> bool {
    matches!(
        e.status,
        crate::ffi::Status::NotFound | crate::ffi::Status::Unavailable
    )
}

/// Largest `<channel>.json` release feed read.
pub const MAX_FEED: u64 = 4 << 20;
/// An Ed25519 signature file.
const MAX_SIG: u64 = 64;

/// Checks a feed pair: signature, format and channel, then freshness against the highest serial
/// accepted for this channel (audit C6-2: never older than seen, never past its `expires`).
fn checked_feed(
    text: &[u8],
    sig: &[u8],
    keys: &[[u8; 32]],
    channel: &str,
    last: u64,
) -> Result<release::Feed> {
    let f = release::Feed::verify(text, sig, keys)?;
    if f.channel != channel {
        return Err(Error::untrusted("The release list is for another channel."));
    }
    release::check_fresh(
        "release list",
        f.serial,
        f.expires,
        last,
        release::unix_now(),
    )?;
    Ok(f)
}

/// The channel feed: `<base>/<channel>.json` and `.json.sig` fetched into memory (bounded,
/// conditional on the stored validators), checked as a pair, and only then written over the cache
/// with their validators (audit C10-2). A download that fails or does not check leaves the cached
/// pair as it was, and the cached pair is used when it still checks (it expires too); otherwise
/// the error is returned. The highest serial accepted per channel is kept in the state dir, which
/// outlives the cache.
fn release_feed(c: &context::Context, bases: &[String], channel: &str) -> Result<release::Feed> {
    let keys = wol_core::package::read_keys(&c.keys);
    let dir = c.cache.join("releases");
    let feed = dir.join(format!("{channel}.json"));
    let sig = dir.join(format!("{channel}.json.sig"));
    let seen = c.state.join(format!("release-{channel}.serial"));
    let last = std::fs::read_to_string(&seen)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let read = shared::bounded::read_file;
    let old = (read(&feed, MAX_FEED), read(&sig, MAX_SIG));
    let have_pair = old.0.is_some() && old.1.is_some();
    // Validators count only while the whole pair is cached.
    let tag = |p: &std::path::Path| {
        have_pair
            .then(|| std::fs::read_to_string(fetch::tag_path(p)).ok())
            .flatten()
    };
    // The pair comes from one base: the first that has it ([`try_next_base`] moves on).
    let pair = |base: &str| {
        fetch::get_conditional(
            &c.net,
            &format!("{base}/{channel}.json"),
            tag(&feed).as_deref(),
            MAX_FEED,
        )
        .and_then(|f| {
            let s = fetch::get_conditional(
                &c.net,
                &format!("{base}/{channel}.json.sig"),
                tag(&sig).as_deref(),
                MAX_SIG,
            )?;
            Ok((f, s))
        })
    };
    let mut got = Err(Error::unavailable("No release list address is set."));
    for base in bases {
        got = pair(base);
        if !got.as_ref().is_err_and(try_next_base) {
            break;
        }
    }
    let use_cached = |e: Error| match &old {
        (Some(t), Some(s)) => checked_feed(t, s, &keys, channel, last).map_err(|_| e),
        _ => Err(e),
    };
    let fresh = got.and_then(|(gf, gs)| {
        let pick = |g: &fetch::Got, o: &Option<Vec<u8>>| match g {
            fetch::Got::Body { bytes, .. } => Some(bytes.clone()),
            fetch::Got::NotModified => o.clone(),
        };
        let (Some(t), Some(s)) = (pick(&gf, &old.0), pick(&gs, &old.1)) else {
            return Err(Error::unavailable("The release list cannot be read."));
        };
        let f = checked_feed(&t, &s, &keys, channel, last)?;
        Ok((f, [(sig.clone(), s, gs), (feed.clone(), t, gf)]))
    });
    let f = match fresh {
        Ok((f, files)) => {
            let cant = || Error::unavailable("The release list cannot be saved.");
            std::fs::create_dir_all(&dir).map_err(|_| cant())?;
            for (path, bytes, g) in files {
                if let fetch::Got::Body { validator, .. } = g {
                    let tp = fetch::tag_path(&path);
                    let _ = std::fs::remove_file(&tp);
                    shared::atomic::atomic_write(&path, 0o644, &bytes).map_err(|_| cant())?;
                    if let Some(v) = validator {
                        let _ = shared::atomic::atomic_write(&tp, 0o644, v.as_bytes());
                    }
                }
            }
            f
        }
        Err(e) => use_cached(e)?,
    };
    if f.serial > last && std::fs::create_dir_all(&c.state).is_ok() {
        let _ = shared::atomic::atomic_write(&seen, 0o644, f.serial.to_string().as_bytes());
    }
    Ok(f)
}

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
    let f = release_feed(c, &feed_bases(r.base_url), &channel)?;
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

    /// The default feed is the CDN's `latest/` folder, then the latest wolos-packages release's
    /// assets on GitHub (`<channel>.json` and `.json.sig` appended), over TLS, with no trailing
    /// slash to double. A request's own base is the only one tried.
    #[test]
    fn default_feed_is_the_cdn_then_the_latest_release_assets() {
        assert_eq!(FEEDS, "https://wolos.squareweb.app/latest");
        assert_eq!(
            FEEDS_FALLBACK,
            "https://github.com/wonderlandos/wolos-packages/releases/latest/download"
        );
        for b in [FEEDS, FEEDS_FALLBACK] {
            assert!(b.starts_with("https://") && !b.ends_with('/'), "{b}");
        }
        assert_eq!(feed_bases(None), [FEEDS, FEEDS_FALLBACK]);
        assert_eq!(
            feed_bases(Some("https://m.example/x".into())),
            ["https://m.example/x"]
        );
    }

    /// The CDN answers 404 or cannot be reached: the feed comes from the next base, checked with
    /// the same keys. A feed that does not check stops there: no other base is tried.
    #[test]
    fn the_feed_falls_back_when_the_cdn_has_none_or_is_down() {
        let d = testutil::temp("fallback");
        let seed = [5u8; 32];
        std::fs::create_dir_all(d.join("keys")).unwrap();
        std::fs::write(
            d.join("keys/wolos.pub"),
            wol_core::package::public_pem(&wol_core::package::public_of(&seed)),
        )
        .unwrap();
        let feed = br#"{"format":1,"channel":"beta","releases":[{"version":"1.1.0-beta.2","manifest":"{}","signature":"00"}]}"#.to_vec();
        let sig = wol_core::package::sign(&seed, &feed).to_vec();
        let good = testutil::serve(vec![
            ("/beta.json", testutil::Served::bytes(feed.clone())),
            ("/beta.json.sig", testutil::Served::bytes(sig)),
        ]);
        let empty = testutil::serve(vec![]);
        // A port nothing listens on: bind one, note it, close it.
        let down = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
        };
        let mut c = appimage::tests::ctx(&d);
        c.keys = d.join("keys");
        for first in [empty.url(""), down] {
            let _ = std::fs::remove_dir_all(c.cache.join("releases"));
            let _ = std::fs::remove_dir_all(&c.state);
            let f = release_feed(&c, &[first.clone(), good.url("")], "beta").unwrap();
            assert_eq!(f.releases[0].version, "1.1.0-beta.2", "{first}");
        }
        // Signed with another key on the first base: refused, never read from the second.
        let forged = testutil::serve(vec![
            ("/beta.json", testutil::Served::bytes(feed.clone())),
            (
                "/beta.json.sig",
                testutil::Served::bytes(wol_core::package::sign(&[6u8; 32], &feed).to_vec()),
            ),
        ]);
        let _ = std::fs::remove_dir_all(c.cache.join("releases"));
        assert!(release_feed(&c, &[forged.url(""), good.url("")], "beta").is_err());
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
    /// C6-2 / C10-2: a feed is checked as a pair before it replaces the cache. A bad, oversized,
    /// mismatched, other-channel, older or expired copy leaves the good cached pair and its
    /// validators as they were; with no good cache the error is returned.
    #[test]
    fn a_bad_feed_never_replaces_the_good_cached_one() {
        use testutil::Served;
        let d = testutil::temp("plan-guard");
        let seed = [5u8; 32];
        std::fs::create_dir_all(d.join("keys")).unwrap();
        std::fs::write(
            d.join("keys/wolos.pub"),
            wol_core::package::public_pem(&wol_core::package::public_of(&seed)),
        )
        .unwrap();
        let feed = |channel: &str, serial: u64, expires: u64| {
            format!(
                concat!(
                    r#"{{"format":1,"channel":"{channel}","serial":{serial},"expires":{expires},"#,
                    r#""releases":[{{"version":"1.1.0","manifest":"{{}}","signature":"00"}}]}}"#,
                ),
                channel = channel,
                serial = serial,
                expires = expires,
            )
            .into_bytes()
        };
        let sign = |t: &[u8]| Served::bytes(wol_core::package::sign(&seed, t).to_vec());
        let good = feed("stable", 5, 0);
        let older = feed("stable", 4, 0);
        let expired = feed("stable", 9, 1);
        let other = feed("beta", 9, 0);
        let newer = feed("stable", 6, 0);
        let mut tampered = good.clone();
        tampered[3] ^= 1;
        let srv = testutil::serve(vec![
            ("/g/stable.json", Served::bytes(good.clone()).etag("\"g\"")),
            ("/g/stable.json.sig", sign(&good).etag("\"gs\"")),
            ("/tampered/stable.json", Served::bytes(tampered)),
            ("/tampered/stable.json.sig", sign(&good)),
            (
                "/big/stable.json",
                Served::bytes(vec![b' '; MAX_FEED as usize + 1]),
            ),
            ("/big/stable.json.sig", sign(&good)),
            ("/pair/stable.json", Served::bytes(newer.clone())),
            ("/pair/stable.json.sig", sign(&good)),
            ("/other/stable.json", Served::bytes(other.clone())),
            ("/other/stable.json.sig", sign(&other)),
            ("/old/stable.json", Served::bytes(older.clone())),
            ("/old/stable.json.sig", sign(&older)),
            ("/exp/stable.json", Served::bytes(expired.clone())),
            ("/exp/stable.json.sig", sign(&expired)),
            ("/new/stable.json", Served::bytes(newer.clone())),
            ("/new/stable.json.sig", sign(&newer)),
        ]);
        let mut c = appimage::tests::ctx(&d);
        c.keys = d.join("keys");
        let cache = c.cache.join("releases");
        let at = |base: &str| release_feed(&c, &[srv.url(base)], "stable");
        assert_eq!(at("/g").unwrap().serial, 5);
        let snapshot = || {
            [
                "stable.json",
                "stable.json.sig",
                "stable.json.etag",
                "stable.json.sig.etag",
            ]
            .map(|n| std::fs::read(cache.join(n)).unwrap())
        };
        let before = snapshot();
        assert_eq!(before[0], good);
        assert_eq!(before[2], b"etag \"g\"");
        for base in ["/tampered", "/big", "/pair", "/other", "/old", "/exp"] {
            // The good cached pair is still served, and nothing on disk changed.
            assert_eq!(at(base).unwrap().serial, 5, "{base}");
            assert_eq!(snapshot(), before, "{base}");
        }
        let left: Vec<String> = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 4, "no temp or part files: {left:?}");
        // A newer pair replaces it and raises the floor kept in the state dir.
        assert_eq!(at("/new").unwrap().serial, 6);
        assert_eq!(std::fs::read(cache.join("stable.json")).unwrap(), newer);
        assert!(
            !cache.join("stable.json.etag").exists(),
            "no validator served"
        );
        // Without a cache the error comes through, and the floor outlives the cache.
        std::fs::remove_dir_all(&cache).unwrap();
        assert!(at("/old").is_err());
        assert!(at("/g").is_err(), "serial 5 is below the 6 already seen");
        assert!(at("/exp").unwrap_err().message.contains("expired"));
        assert!(at("/other").is_err());
        assert!(!cache.join("stable.json").exists());
        assert_eq!(at("/new").unwrap().serial, 6);
    }
}
