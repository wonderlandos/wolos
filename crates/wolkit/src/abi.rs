// SPDX-License-Identifier: BSD-2-Clause
//! Every `#define` of `include/wolkit.h`, as data. `tests/abi.rs` checks the header against this
//! list both ways, so a constant cannot drift from the header, and the tables the ids come from
//! (`scene::prop::TABLE`, `scene::Kind`, `scene::patch::op`, `event::kind`) are what the header
//! must list. Ids are append-only: a shipped value never changes.

use crate::error::Status;

/// A constant as the `i64` the table keeps (they all fit).
fn n(v: impl TryInto<i64>) -> i64 {
    v.try_into().unwrap_or(i64::MAX)
}

use crate::event::{self, bit};
use crate::scene::kind::{ROLES, state};
use crate::scene::patch::{self, op};
use crate::scene::{Kind, prop};
use crate::window;

pub const ENGINE_HEADLESS: u32 = 1;
pub const ENGINE_FORCE_CPU: u32 = 2;
pub const ENGINE_FORCE_KIT: u32 = 4;

pub const DISPATCH_EVENTS: u32 = 1;
pub const DISPATCH_QUIT: u32 = 2;

pub const INFO_BACKEND: u32 = 1;
pub const INFO_PATCH_FORMAT: u32 = 2;
pub const INFO_MAX_NODES: u32 = 3;
pub const INFO_A11Y_ACTIVE: u32 = 4;
pub const INFO_MAX_PATCH: u32 = 5;
pub const INFO_MAX_WINDOWS: u32 = 6;
pub const INFO_MAX_EVENT_RECORD: u32 = 7;
pub const INFO_GPU: u32 = 8;

pub const TRIM_LIGHT: u32 = 0;
pub const TRIM_HIDDEN: u32 = 1;
pub const TRIM_CRITICAL: u32 = 2;

/// Fixed-size constants that are not generated from a table.
fn fixed() -> Vec<(&'static str, i64)> {
    vec![
        ("WOLKIT_ENGINE_HEADLESS", n(ENGINE_HEADLESS)),
        ("WOLKIT_ENGINE_FORCE_CPU", n(ENGINE_FORCE_CPU)),
        ("WOLKIT_ENGINE_FORCE_KIT", n(ENGINE_FORCE_KIT)),
        ("WOLKIT_DISPATCH_EVENTS", n(DISPATCH_EVENTS)),
        ("WOLKIT_DISPATCH_QUIT", n(DISPATCH_QUIT)),
        ("WOLKIT_INFO_BACKEND", n(INFO_BACKEND)),
        ("WOLKIT_INFO_PATCH_FORMAT", n(INFO_PATCH_FORMAT)),
        ("WOLKIT_INFO_MAX_NODES", n(INFO_MAX_NODES)),
        ("WOLKIT_INFO_A11Y_ACTIVE", n(INFO_A11Y_ACTIVE)),
        ("WOLKIT_INFO_MAX_PATCH", n(INFO_MAX_PATCH)),
        ("WOLKIT_INFO_MAX_WINDOWS", n(INFO_MAX_WINDOWS)),
        ("WOLKIT_INFO_MAX_EVENT_RECORD", n(INFO_MAX_EVENT_RECORD)),
        ("WOLKIT_INFO_GPU", n(INFO_GPU)),
        ("WOLKIT_BACKEND_HEADLESS", n(window::BACKEND_HEADLESS)),
        ("WOLKIT_BACKEND_KIT", n(window::BACKEND_KIT)),
        ("WOLKIT_BACKEND_NATIVE", n(window::BACKEND_NATIVE)),
        ("WOLKIT_TRIM_LIGHT", n(TRIM_LIGHT)),
        ("WOLKIT_TRIM_HIDDEN", n(TRIM_HIDDEN)),
        ("WOLKIT_TRIM_CRITICAL", n(TRIM_CRITICAL)),
        ("WOLKIT_WINDOW_FIXED_SIZE", n(window::WINDOW_FIXED_SIZE)),
        (
            "WOLKIT_ENV_REDUCE_MOTION",
            n(crate::env::FLAG_REDUCE_MOTION),
        ),
        (
            "WOLKIT_ENV_REDUCE_TRANSPARENCY",
            n(crate::env::FLAG_REDUCE_TRANSPARENCY),
        ),
        ("WOLKIT_ENV_CONTRAST", n(crate::env::FLAG_CONTRAST)),
        ("WOLKIT_DUMP_SCENE", n(window::dump::SCENE)),
        ("WOLKIT_DUMP_LAYOUT", n(window::dump::LAYOUT)),
        ("WOLKIT_DUMP_A11Y", n(window::dump::A11Y)),
        ("WOLKIT_POINTER_MOVE", n(window::POINTER_MOVE)),
        ("WOLKIT_POINTER_DOWN", n(window::POINTER_DOWN)),
        ("WOLKIT_POINTER_UP", n(window::POINTER_UP)),
        ("WOLKIT_POINTER_SCROLL", n(window::POINTER_SCROLL)),
        ("WOLKIT_POINTER_LEAVE", n(window::POINTER_LEAVE)),
        ("WOLKIT_MOD_SHIFT", n(window::MOD_SHIFT)),
        ("WOLKIT_MOD_CTRL", n(window::MOD_CTRL)),
        ("WOLKIT_MOD_ALT", n(window::MOD_ALT)),
        ("WOLKIT_MOD_SUPER", n(window::MOD_SUPER)),
        ("WOLKIT_PATCH_MAGIC", n(patch::MAGIC)),
        ("WOLKIT_PATCH_FORMAT", n(patch::FORMAT)),
        ("WOLKIT_PATCH_HEADER", n(patch::HEADER)),
        ("WOLKIT_PATCH_MUST_UNDERSTAND", n(patch::MUST_UNDERSTAND)),
        ("WOLKIT_APPEND", n(patch::APPEND)),
        ("WOLKIT_NO_STR", n(crate::scene::strings::NO_STR)),
        ("WOLKIT_ANIM_TWEEN", n(patch::ANIM_TWEEN)),
        ("WOLKIT_ANIM_SPRING", n(patch::ANIM_SPRING)),
        ("WOLKIT_DIM_AUTO", n(prop::DIM_AUTO)),
        ("WOLKIT_DIM_PX", n(prop::DIM_PX)),
        ("WOLKIT_DIM_PERCENT", n(prop::DIM_PERCENT)),
        ("WOLKIT_EVENT_HEADER", n(event::HEADER)),
        ("WOLKIT_EVENT_MAX_RECORD", n(event::MAX_RECORD)),
        ("WOLKIT_EVENT_FLAG_OVERFLOW", n(event::FLAG_OVERFLOW)),
    ]
}

/// Every `#define` in the header, name and value.
pub fn constants() -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = vec![
        ("WOLKIT_VERSION_MAJOR".into(), i64::from(crate::VERSION.0)),
        ("WOLKIT_VERSION_MINOR".into(), i64::from(crate::VERSION.1)),
        ("WOLKIT_VERSION_PATCH".into(), i64::from(crate::VERSION.2)),
    ];
    v.extend(
        Status::ALL
            .iter()
            .map(|(n, s)| (format!("WOLKIT_{n}"), i64::from(s.code()))),
    );
    v.extend(fixed().into_iter().map(|(n, x)| (n.to_owned(), x)));
    v.extend(Kind::ALL.iter().map(|k| (k.define(), i64::from(k.id()))));
    v.extend(
        prop::TABLE
            .iter()
            .map(|d| (format!("WOLKIT_PROP_{}", d.ident), i64::from(d.id))),
    );
    v.extend(
        op::NAMES
            .iter()
            .map(|(n, x)| (format!("WOLKIT_OP_{n}"), i64::from(*x))),
    );
    v.extend(
        event::kind::NAMES
            .iter()
            .map(|(n, x)| (format!("WOLKIT_EVENT_{n}"), i64::from(*x))),
    );
    v.extend(
        event::kind::NAMES
            .iter()
            .map(|(n, x)| (format!("WOLKIT_BIND_{n}"), i64::from(bit(*x)))),
    );
    v.extend(
        state::NAMES
            .iter()
            .map(|(n, x)| (format!("WOLKIT_STATE_{n}"), i64::from(*x))),
    );
    v.extend(
        ROLES
            .iter()
            .map(|(x, n)| (format!("WOLKIT_ROLE_{n}"), i64::from(*x))),
    );
    v
}
