// SPDX-License-Identifier: BSD-2-Clause
//! Every `#define` of `include/wolkit.h`, as data. `tests/abi.rs` checks the header against this
//! list both ways, so a constant cannot drift from the header, and the tables the ids come from
//! (`scene::prop::TABLE`, `scene::Kind`, `scene::patch::op`, `event::kind`) are what the header
//! must list. Ids are append-only: a shipped value never changes.

use crate::error::Status;
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
const FIXED: &[(&str, i64)] = &[
    ("WOLKIT_ENGINE_HEADLESS", ENGINE_HEADLESS as i64),
    ("WOLKIT_ENGINE_FORCE_CPU", ENGINE_FORCE_CPU as i64),
    ("WOLKIT_ENGINE_FORCE_KIT", ENGINE_FORCE_KIT as i64),
    ("WOLKIT_DISPATCH_EVENTS", DISPATCH_EVENTS as i64),
    ("WOLKIT_DISPATCH_QUIT", DISPATCH_QUIT as i64),
    ("WOLKIT_INFO_BACKEND", INFO_BACKEND as i64),
    ("WOLKIT_INFO_PATCH_FORMAT", INFO_PATCH_FORMAT as i64),
    ("WOLKIT_INFO_MAX_NODES", INFO_MAX_NODES as i64),
    ("WOLKIT_INFO_A11Y_ACTIVE", INFO_A11Y_ACTIVE as i64),
    ("WOLKIT_INFO_MAX_PATCH", INFO_MAX_PATCH as i64),
    ("WOLKIT_INFO_MAX_WINDOWS", INFO_MAX_WINDOWS as i64),
    ("WOLKIT_INFO_MAX_EVENT_RECORD", INFO_MAX_EVENT_RECORD as i64),
    ("WOLKIT_INFO_GPU", INFO_GPU as i64),
    ("WOLKIT_BACKEND_HEADLESS", window::BACKEND_HEADLESS as i64),
    ("WOLKIT_BACKEND_KIT", window::BACKEND_KIT as i64),
    ("WOLKIT_BACKEND_NATIVE", window::BACKEND_NATIVE as i64),
    ("WOLKIT_TRIM_LIGHT", TRIM_LIGHT as i64),
    ("WOLKIT_TRIM_HIDDEN", TRIM_HIDDEN as i64),
    ("WOLKIT_TRIM_CRITICAL", TRIM_CRITICAL as i64),
    ("WOLKIT_WINDOW_FIXED_SIZE", window::WINDOW_FIXED_SIZE as i64),
    ("WOLKIT_ENV_REDUCE_MOTION", crate::env::FLAG_REDUCE_MOTION as i64),
    ("WOLKIT_ENV_REDUCE_TRANSPARENCY", crate::env::FLAG_REDUCE_TRANSPARENCY as i64),
    ("WOLKIT_ENV_CONTRAST", crate::env::FLAG_CONTRAST as i64),
    ("WOLKIT_DUMP_SCENE", window::dump::SCENE as i64),
    ("WOLKIT_DUMP_LAYOUT", window::dump::LAYOUT as i64),
    ("WOLKIT_DUMP_A11Y", window::dump::A11Y as i64),
    ("WOLKIT_POINTER_MOVE", window::POINTER_MOVE as i64),
    ("WOLKIT_POINTER_DOWN", window::POINTER_DOWN as i64),
    ("WOLKIT_POINTER_UP", window::POINTER_UP as i64),
    ("WOLKIT_POINTER_SCROLL", window::POINTER_SCROLL as i64),
    ("WOLKIT_POINTER_LEAVE", window::POINTER_LEAVE as i64),
    ("WOLKIT_MOD_SHIFT", window::MOD_SHIFT as i64),
    ("WOLKIT_MOD_CTRL", window::MOD_CTRL as i64),
    ("WOLKIT_MOD_ALT", window::MOD_ALT as i64),
    ("WOLKIT_MOD_SUPER", window::MOD_SUPER as i64),
    ("WOLKIT_PATCH_MAGIC", patch::MAGIC as i64),
    ("WOLKIT_PATCH_FORMAT", patch::FORMAT as i64),
    ("WOLKIT_PATCH_HEADER", patch::HEADER as i64),
    ("WOLKIT_PATCH_MUST_UNDERSTAND", patch::MUST_UNDERSTAND as i64),
    ("WOLKIT_APPEND", patch::APPEND as i64),
    ("WOLKIT_NO_STR", crate::scene::strings::NO_STR as i64),
    ("WOLKIT_ANIM_TWEEN", patch::ANIM_TWEEN as i64),
    ("WOLKIT_ANIM_SPRING", patch::ANIM_SPRING as i64),
    ("WOLKIT_DIM_AUTO", prop::DIM_AUTO as i64),
    ("WOLKIT_DIM_PX", prop::DIM_PX as i64),
    ("WOLKIT_DIM_PERCENT", prop::DIM_PERCENT as i64),
    ("WOLKIT_EVENT_HEADER", event::HEADER as i64),
    ("WOLKIT_EVENT_MAX_RECORD", event::MAX_RECORD as i64),
    ("WOLKIT_EVENT_FLAG_OVERFLOW", event::FLAG_OVERFLOW as i64),
];

/// Every `#define` in the header, name and value.
pub fn constants() -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = vec![
        ("WOLKIT_VERSION_MAJOR".into(), i64::from(crate::VERSION.0)),
        ("WOLKIT_VERSION_MINOR".into(), i64::from(crate::VERSION.1)),
        ("WOLKIT_VERSION_PATCH".into(), i64::from(crate::VERSION.2)),
    ];
    v.extend(Status::ALL.iter().map(|(n, s)| (format!("WOLKIT_{n}"), *s as i64)));
    v.extend(FIXED.iter().map(|(n, x)| ((*n).to_owned(), *x)));
    v.extend(Kind::ALL.iter().map(|k| (k.define(), *k as i64)));
    v.extend(prop::TABLE.iter().map(|d| (format!("WOLKIT_PROP_{}", d.ident), i64::from(d.id))));
    v.extend(op::NAMES.iter().map(|(n, x)| (format!("WOLKIT_OP_{n}"), i64::from(*x))));
    v.extend(event::kind::NAMES.iter().map(|(n, x)| (format!("WOLKIT_EVENT_{n}"), i64::from(*x))));
    v.extend(event::kind::NAMES.iter().map(|(n, x)| (format!("WOLKIT_BIND_{n}"), i64::from(bit(*x)))));
    v.extend(state::NAMES.iter().map(|(n, x)| (format!("WOLKIT_STATE_{n}"), i64::from(*x))));
    v.extend(ROLES.iter().map(|(x, n)| (format!("WOLKIT_ROLE_{n}"), i64::from(*x))));
    v
}
