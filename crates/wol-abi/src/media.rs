// SPDX-License-Identifier: BSD-2-Clause
//! A player of the app's own (ADR 0020, 1.16): the desktop gives it the name
//! `org.mpris.MediaPlayer2.wol.<app>` on the session bus, so the media keys, Control Center and
//! `playerctl` see and control it. The player lasts as long as its connection to wold (like a
//! search provider); the remote controls come back as `wol_media_fn` calls inside
//! `wol_context_dispatch`.
use crate::client;
use crate::context::{lock, wol_context};
use crate::error::{Error, UserData, guard, input, need, wol_error, wol_status};
use crate::kit::arc_of;
use crate::string::{into_raw, release, retain};
use shared::portal::{
    CAN_NEXT, CAN_PREVIOUS, CAN_SEEK, Call, MAX_MEDIA_US, MAX_PATH, MAX_TITLE, MediaKey,
    MediaState, MediaStatus,
};
use std::ffi::{c_char, c_void};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

pub type wol_media_status = i32;
pub type wol_media_key = i32;

/// A remote control reached the player: the key, and for seeking the seconds.
// SAFETY: the app's own function, called only inside `wol_context_dispatch` on its thread.
pub type wol_media_fn = Option<unsafe extern "C" fn(*mut wol_media, wol_media_key, f64, UserData)>;

/// `wol_media_status` and `wol_media_key` values, in the order of `wol.h`.
pub const STATUS_STOPPED: i64 = 0;
pub const STATUS_PLAYING: i64 = 1;
pub const STATUS_PAUSED: i64 = 2;
pub const KEY_PLAY: i64 = 0;
pub const KEY_PAUSE: i64 = 1;
pub const KEY_TOGGLE: i64 = 2;
pub const KEY_STOP: i64 = 3;
pub const KEY_NEXT: i64 = 4;
pub const KEY_PREVIOUS: i64 = 5;
pub const KEY_SEEK: i64 = 6;
pub const KEY_SET_POSITION: i64 = 7;

#[repr(C)]
pub struct wol_media {
    tag: crate::handle::Tag<wol_media>,
    context: Arc<wol_context>,
    handler: Mutex<(wol_media_fn, usize)>,
    state: Mutex<MediaState>,
    _held: UnixStream,
}

fn key_number(k: MediaKey) -> i64 {
    match k {
        MediaKey::Play => KEY_PLAY,
        MediaKey::Pause => KEY_PAUSE,
        MediaKey::Toggle => KEY_TOGGLE,
        MediaKey::Stop => KEY_STOP,
        MediaKey::Next => KEY_NEXT,
        MediaKey::Previous => KEY_PREVIOUS,
        MediaKey::Seek => KEY_SEEK,
        MediaKey::SetPosition => KEY_SET_POSITION,
    }
}

/// wold passed on a remote control: run the app's handler, if it set one.
pub(crate) fn pressed(m: &Arc<wol_media>, key: MediaKey, value_us: i64) {
    let (f, user_data) = *lock(&m.handler);
    let Some(f) = f else { return };
    // SAFETY: the app's callback on its thread (inside dispatch); the handle outlives the call.
    unsafe {
        f(
            Arc::as_ptr(m).cast_mut(),
            i32::try_from(key_number(key)).unwrap_or(-1),
            value_us as f64 / 1e6,
            user_data as UserData,
        )
    };
}

/// Seconds the app gave -> microseconds, 0 to 100 days.
fn micros(seconds: f64, what: &str) -> crate::error::Result<i64> {
    // Refuses NaN, infinity and a negative number.
    let span = std::time::Duration::try_from_secs_f64(seconds)
        .ok()
        .and_then(|d| i64::try_from(d.as_micros()).ok())
        .filter(|us| *us <= MAX_MEDIA_US);
    span.ok_or_else(|| Error::invalid(format!("{what} is 0 to 8,640,000 seconds (100 days).")))
}

fn status_of(n: i32) -> crate::error::Result<MediaStatus> {
    match i64::from(n) {
        STATUS_STOPPED => Ok(MediaStatus::Stopped),
        STATUS_PLAYING => Ok(MediaStatus::Playing),
        STATUS_PAUSED => Ok(MediaStatus::Paused),
        _ => Err(Error::invalid(
            "The status is WOL_MEDIA_STOPPED, _PLAYING or _PAUSED.",
        )),
    }
}

/// Change the state with `f`, check it whole, keep it and tell the desktop.
fn change(
    media: *mut wol_media,
    f: impl FnOnce(&mut MediaState) -> crate::error::Result<()>,
) -> crate::error::Result<()> {
    // SAFETY: NULL or a libwol handle (header contract); the tag is checked first.
    let m = unsafe { crate::string::borrow(media.cast_const()) }?;
    let mut next = lock(&m.state).clone();
    f(&mut next)?;
    next.check().map_err(Error::invalid)?;
    *lock(&m.state) = next.clone();
    client::run(m.context.wold(), &Call::MediaState { state: next })
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_publish(
    context: *mut wol_context,
    out: *mut *mut wol_media,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        if context.media().is_some() {
            return Err(Error::invalid("This context already publishes a player."));
        }
        // Listening first, so no key is lost.
        context.ensure_bus()?;
        let held = client::hold(context.wold(), &Call::MediaPublish {})?;
        let first = MediaState {
            status: MediaStatus::Stopped,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            art: String::new(),
            length_us: 0,
            position_us: 0,
            can: 0,
        };
        let m = Arc::new(wol_media {
            tag: crate::handle::Tag::new(),
            context: context.clone(),
            handler: Mutex::new((None, 0)),
            state: Mutex::new(first.clone()),
            _held: held,
        });
        client::run(context.wold(), &Call::MediaState { state: first })?;
        context.set_media(&m);
        // SAFETY: `out` was checked non-NULL above and is writable (header contract).
        unsafe { *out = into_raw(m) };
        Ok(())
    })
}

/// # Safety
/// `media` is live; `user_data` is passed back untouched. A NULL `on_key` takes the handler away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_set_handler(
    media: *mut wol_media,
    on_key: wol_media_fn,
    user_data: *mut c_void,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        // SAFETY: as in `change`.
        let m = unsafe { crate::string::borrow(media.cast_const()) }?;
        *lock(&m.handler) = (on_key, user_data.addr());
        Ok(())
    })
}

/// # Safety
/// `media` is live; each string points at its length; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_set_track(
    media: *mut wol_media,
    title: *const c_char,
    title_len: usize,
    artist: *const c_char,
    artist_len: usize,
    album: *const c_char,
    album_len: usize,
    length_seconds: f64,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let title = input(title, title_len, MAX_TITLE, "The title")?.to_owned();
        let artist = input(artist, artist_len, MAX_TITLE, "The artist")?.to_owned();
        let album = input(album, album_len, MAX_TITLE, "The album")?.to_owned();
        let length_us = micros(length_seconds, "The length")?;
        change(media, |s| {
            s.title = title;
            s.artist = artist;
            s.album = album;
            s.length_us = length_us;
            Ok(())
        })
    })
}

/// # Safety
/// `media` is live; `path` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_set_art(
    media: *mut wol_media,
    path: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let art = input(path, len, MAX_PATH, "The path")?.to_owned();
        change(media, |s| {
            s.art = art;
            Ok(())
        })
    })
}

/// # Safety
/// `media` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_set_state(
    media: *mut wol_media,
    status: wol_media_status,
    position_seconds: f64,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let status = status_of(status)?;
        let position_us = micros(position_seconds, "The position")?;
        change(media, |s| {
            s.status = status;
            s.position_us = position_us;
            Ok(())
        })
    })
}

/// # Safety
/// `media` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_set_capabilities(
    media: *mut wol_media,
    flags: u32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let can = u8::try_from(flags)
            .ok()
            .filter(|f| f & !(CAN_NEXT | CAN_PREVIOUS | CAN_SEEK) == 0)
            .ok_or_else(|| Error::invalid("Unknown WOL_MEDIA_CAN_ flags."))?;
        change(media, |s| {
            s.can = can;
            Ok(())
        })
    })
}

/// # Safety
/// `media` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_retain(media: *mut wol_media) -> *mut wol_media {
    unsafe { retain(media) }
}

/// # Safety
/// `media` is NULL or a reference the caller owns. The last release takes the player away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_media_release(media: *mut wol_media) {
    unsafe { release(media) }
}

crate::handle::tagged!(wol_media, 0x6a1f_7c42_93be_08d5);

#[cfg(test)]
mod tests {
    use super::*;

    /// `abi::CONSTANTS` spells the ability flags as numbers (it is a const table): they are the
    /// wire's.
    #[test]
    fn the_ability_flags_are_the_wires() {
        let flag = |name: &str| {
            crate::abi::CONSTANTS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| *v)
        };
        assert_eq!(flag("WOL_MEDIA_CAN_NEXT"), Some(i64::from(CAN_NEXT)));
        assert_eq!(
            flag("WOL_MEDIA_CAN_PREVIOUS"),
            Some(i64::from(CAN_PREVIOUS))
        );
        assert_eq!(flag("WOL_MEDIA_CAN_SEEK"), Some(i64::from(CAN_SEEK)));
    }

    #[test]
    fn seconds_become_microseconds_or_are_refused() {
        assert_eq!(micros(215.5, "t").unwrap(), 215_500_000);
        assert_eq!(micros(0.0, "t").unwrap(), 0);
        for bad in [-1.0, f64::NAN, f64::INFINITY, 8_640_001.0, 1e300] {
            assert!(micros(bad, "t").is_err(), "{bad}");
        }
    }

    #[test]
    fn every_key_has_its_number() {
        for (k, n) in [
            (MediaKey::Play, KEY_PLAY),
            (MediaKey::Pause, KEY_PAUSE),
            (MediaKey::Toggle, KEY_TOGGLE),
            (MediaKey::Stop, KEY_STOP),
            (MediaKey::Next, KEY_NEXT),
            (MediaKey::Previous, KEY_PREVIOUS),
            (MediaKey::Seek, KEY_SEEK),
            (MediaKey::SetPosition, KEY_SET_POSITION),
        ] {
            assert_eq!(key_number(k), n);
        }
        assert!(status_of(3).is_err() && status_of(-1).is_err());
    }
}
