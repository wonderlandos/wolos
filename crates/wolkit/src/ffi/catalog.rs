// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_catalog_*` and `wolkit_lang_*`: the locale system (ADR 0045) for C and Crystal apps. A
//! catalog is one unit's compiled text tables (`.wtx`, made by `wol-i18n compile`); a lookup is a
//! hash-slot read and a slice, with no parsing and no allocation, and the text it returns lives as
//! long as the catalog. The tables and the fallback chain (language, then English) are
//! `shared::i18n`, the same code the shell and the tools use.

use super::{
    borrow, bytes_in, guard, guard_status, handle, into_raw, need, put, quiet, release, retain,
    text_in, wolkit_error, wolkit_status,
};
use crate::error::Error;
use shared::i18n::{Catalog, Key, Lang, Table};
use std::ffi::c_char;
use std::sync::Arc;

/// Most tables one catalog takes (one per language, the pseudo-locale included, with room).
const MAX_TABLES: usize = 32;
/// Largest table accepted from memory.
const MAX_TABLE: usize = 16 << 20;
/// `WOLKIT_LANG_CURRENT`: the language `wolkit_lang_set` chose.
const CURRENT: i32 = -1;
/// What a lookup of an unknown key hands back, so an unchecked caller still reads text.
static NOTHING: [u8; 1] = [0];

#[repr(C)]
#[allow(non_camel_case_types)] // the C name
pub struct wolkit_catalog {
    tag: handle::Tag<wolkit_catalog>,
    catalog: Catalog,
}

handle::tagged!(wolkit_catalog, 0x57_4f_4c_4b_43_41_54_01);

/// The language `lang` names: a `WOLKIT_LANG_*` id or `WOLKIT_LANG_CURRENT`.
fn lang_of(lang: i32) -> Result<Lang, Error> {
    if lang == CURRENT {
        return Ok(shared::i18n::lang());
    }
    usize::try_from(lang)
        .ok()
        .and_then(|i| Lang::ALL.get(i).copied())
        .ok_or_else(|| Error::invalid("That is not a WolOS language id; use a WOLKIT_LANG_ value."))
}

fn tables_err(e: shared::i18n::WtxError) -> Error {
    Error::invalid(format!("A text table could not be read: {e}."))
}

/// Builds a catalog from `count` tables in memory: the English one and any translations.
///
/// # Safety
/// `tables` and `lens` point at `count` readable entries, each table at `lens[i]` readable bytes;
/// `out` is writable; `error` is writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_load(
    tables: *const *const u8,
    lens: *const usize,
    count: usize,
    out: *mut *mut wolkit_catalog,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        if count == 0 || count > MAX_TABLES || tables.is_null() || lens.is_null() {
            return Err(Error::invalid(
                "A catalog needs between 1 and 32 text tables.",
            ));
        }
        let mut loaded = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: `count` readable entries (contract).
            let (p, n) = unsafe { (tables.add(i).read(), lens.add(i).read()) };
            // SAFETY: `n` readable bytes at `p` (contract).
            let b = unsafe { bytes_in(p, n, MAX_TABLE, "A text table") }?;
            loaded.push(Table::from_vec(b.to_vec()).map_err(tables_err)?);
        }
        let h = Arc::new(wolkit_catalog {
            tag: handle::Tag::new(),
            catalog: assemble(loaded)?,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
    })
}

/// English first, then the rest: a catalog's keys are those of its English table.
fn assemble(tables: Vec<Table>) -> Result<Catalog, Error> {
    let (en, rest): (Vec<Table>, Vec<Table>) =
        tables.into_iter().partition(|t| t.lang() == Lang::En);
    let mut en = en.into_iter();
    let first = en.next().ok_or_else(|| {
        Error::invalid("A catalog needs the English text table; add it and try again.")
    })?;
    if en.next().is_some() {
        return Err(Error::invalid("A catalog takes one English text table."));
    }
    let mut c = Catalog::new(first).map_err(tables_err)?;
    for t in rest {
        c.add(t).map_err(tables_err)?;
    }
    Ok(c)
}

/// Reads `<dir>/<unit>.<lang>.wtx` for every language that has a file (a language pack adds one).
///
/// # Safety
/// `dir` and `unit` point at their lengths of readable bytes; `out` is writable; `error` is
/// writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_load_dir(
    dir: *const c_char,
    dir_len: usize,
    unit: *const c_char,
    unit_len: usize,
    out: *mut *mut wolkit_catalog,
    error: *mut *mut wolkit_error,
) -> wolkit_status {
    guard(error, || {
        need(out, "out")?;
        // SAFETY: pointers and lengths are the caller's (contract).
        let dir = unsafe { text_in(dir, dir_len, 4096, "The text directory") }?;
        // SAFETY: as above.
        let unit = unsafe { text_in(unit, unit_len, 255, "The unit name") }?;
        let catalog = Catalog::load_dir(std::path::Path::new(dir), unit).map_err(|_| {
            Error::unavailable(format!(
                "The English text of “{unit}” is missing in {dir}; reinstall the app."
            ))
        })?;
        let h = Arc::new(wolkit_catalog {
            tag: handle::Tag::new(),
            catalog,
        });
        // SAFETY: checked non-NULL above.
        unsafe { *out = into_raw(h) };
        Ok(())
    })
}

/// # Safety
/// `catalog` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_retain(
    catalog: *mut wolkit_catalog,
) -> *mut wolkit_catalog {
    // SAFETY: NULL or live (contract).
    unsafe { retain(catalog) }
}

/// # Safety
/// `catalog` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_release(catalog: *mut wolkit_catalog) {
    // SAFETY: NULL or a reference the caller owns (contract).
    unsafe { release(catalog) }
}

/// Number of keys (0 for a handle that is not a live catalog).
///
/// # Safety
/// `catalog` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_count(catalog: *const wolkit_catalog) -> u32 {
    quiet(0, || {
        // SAFETY: NULL or live (contract).
        unsafe { borrow(catalog) }.map_or(0, |c| u32::try_from(c.catalog.len()).unwrap_or(u32::MAX))
    })
}

/// The key named `name`, or `WOLKIT_NO_KEY`. Constant time; do it once per string and keep the key.
///
/// # Safety
/// `catalog` is NULL or live; `name` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_find(
    catalog: *const wolkit_catalog,
    name: *const c_char,
    len: usize,
) -> u32 {
    quiet(u32::MAX, || {
        // SAFETY: NULL or live (contract).
        let Ok(c) = (unsafe { borrow(catalog) }) else {
            return u32::MAX;
        };
        // SAFETY: `len` readable bytes (contract).
        let Ok(name) = (unsafe { text_in(name, len, 256, "A key name") }) else {
            return u32::MAX;
        };
        c.catalog.find(name).map_or(u32::MAX, |k| k.0)
    })
}

/// Writes the text and its length.
fn give_text(text: &str, out: *mut *const c_char, len: *mut usize) {
    put(out, text.as_ptr().cast());
    put(len, text.len());
}

/// The text of `key` in `lang` (or `WOLKIT_LANG_CURRENT`), falling back to English. `*text` points
/// into the catalog (not NUL-terminated: use `*len`) and stays valid until the catalog is released.
///
/// # Safety
/// `catalog` is NULL or live; `text` and `len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_text(
    catalog: *const wolkit_catalog,
    key: u32,
    lang: i32,
    text: *mut *const c_char,
    len: *mut usize,
) -> wolkit_status {
    // SAFETY: forwarded contract.
    unsafe { lookup(catalog, key, None, lang, text, len) }
}

/// The form of a plural key for `count` (CLDR rules of the language), as [`wolkit_catalog_text`].
///
/// # Safety
/// As [`wolkit_catalog_text`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_catalog_plural(
    catalog: *const wolkit_catalog,
    key: u32,
    count: u64,
    lang: i32,
    text: *mut *const c_char,
    len: *mut usize,
) -> wolkit_status {
    // SAFETY: forwarded contract.
    unsafe { lookup(catalog, key, Some(count), lang, text, len) }
}

/// # Safety
/// As [`wolkit_catalog_text`].
unsafe fn lookup(
    catalog: *const wolkit_catalog,
    key: u32,
    count: Option<u64>,
    lang: i32,
    text: *mut *const c_char,
    len: *mut usize,
) -> wolkit_status {
    // A failed lookup still leaves "" behind, so a caller that ignores the status reads nothing.
    put(text, NOTHING.as_ptr().cast());
    put(len, 0);
    guard_status(|| {
        need(text, "text")?;
        need(len, "len")?;
        // SAFETY: NULL or live (contract).
        let c = unsafe { borrow(catalog) }?;
        let l = lang_of(lang)?;
        if usize::try_from(key).map_or(true, |k| k >= c.catalog.len()) {
            return Err(Error::invalid("That key is not in this catalog."));
        }
        let s = match count {
            Some(n) => c.catalog.plural(Key(key), Some(n), l),
            None => c.catalog.text(Key(key), l),
        };
        give_text(s, text, len);
        Ok(())
    })
}

/// The language named by a tag (`pt-BR`, `zh-Hans`) or a POSIX locale (`pt_BR.UTF-8`), as a
/// `WOLKIT_LANG_*` id; -1 for a language WolOS has no translation of (show English).
///
/// # Safety
/// `name` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wolkit_lang_parse(name: *const c_char, len: usize) -> i32 {
    quiet(-1, || {
        // SAFETY: `len` readable bytes (contract).
        let Ok(s) = (unsafe { text_in(name, len, 64, "A language name") }) else {
            return -1;
        };
        let found = Lang::from_tag(s).or_else(|| Lang::from_posix(&s.replace('-', "_")));
        found.map_or(-1, |l| i32::try_from(l.index()).unwrap_or(-1))
    })
}

/// The BCP 47 tag of a language id as static NUL-terminated text, or NULL for an unknown id.
#[unsafe(no_mangle)]
pub extern "C" fn wolkit_lang_tag(lang: i32) -> *const c_char {
    quiet(std::ptr::null(), || match lang_of(lang) {
        Ok(l) => tag_c(l).as_ptr().cast(),
        Err(_) => std::ptr::null(),
    })
}

/// NUL-terminated tags, in `Lang::ALL` order.
const TAGS: [&[u8]; 11] = [
    b"en\0",
    b"pt-BR\0",
    b"es\0",
    b"fr\0",
    b"de\0",
    b"ja\0",
    b"ko\0",
    b"zh-Hans\0",
    b"id\0",
    b"tl\0",
    b"xx\0",
];

fn tag_c(l: Lang) -> &'static [u8] {
    TAGS.get(l.index()).copied().unwrap_or(b"en\0")
}

/// The language this process shows text in (`WOLKIT_LANG_EN` until `wolkit_lang_set`).
#[unsafe(no_mangle)]
pub extern "C" fn wolkit_lang_get() -> i32 {
    quiet(0, || {
        i32::try_from(shared::i18n::lang().index()).unwrap_or(0)
    })
}

/// Shows text in `lang` from now on, for every catalog lookup that asks for the current language.
#[unsafe(no_mangle)]
pub extern "C" fn wolkit_lang_set(lang: i32) -> wolkit_status {
    guard_status(|| {
        if lang == CURRENT {
            return Err(Error::invalid("Choose a language, not “current”."));
        }
        shared::i18n::set_lang(lang_of(lang)?);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_match_the_language_list() {
        for l in Lang::ALL {
            let c = tag_c(l);
            assert_eq!(&c[..c.len() - 1], l.tag().as_bytes());
            assert_eq!(c.last(), Some(&0));
        }
    }
}
