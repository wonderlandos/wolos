// SPDX-License-Identifier: BSD-2-Clause
//! libWolkit (ADR 0039, docs/modules/libwolkit.md): the WolOS UI engine. A retained scene that an
//! app changes with binary patch streams, a style/layout/paint pipeline and a window backend, over
//! one stable C ABI (`include/wolkit.h`, `libwolkit.so.1`, symbols versioned by `abi/wolkit.map`).
//! `tests/abi.rs` fails on any removed or changed symbol, declaration, property or kind id.
//!
//! Rules at the boundary: opaque handles only; `_create` returns a reference the caller releases,
//! `_get_` borrows from its argument; UTF-8 strings with lengths; a status code and an optional
//! error object from every call that can fail; no callbacks (everything the engine tells the app
//! is a record in the event stream); one thread at a time per engine; a panic never crosses the
//! boundary (it becomes `WOLKIT_ERR_INTERNAL`).
//!
//! Layout of the crate (the owner of each part is in the file header):
//! `scene` (arena, patch, I10), `event` (I12), `error`, `loop_`, `window` (backends), `ffi` (the
//! only place with `extern "C"`), and the pipeline stages `style`, `layout`, `text`, `paint`,
//! `cpu`, `gpu`, `input`, `a11y`.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod a11y;
pub mod abi;
pub mod anim;
pub mod copy;
pub mod cpu;
pub mod diag;
pub mod engine;
pub mod env;
pub mod error;
pub mod event;
pub mod ffi;
pub mod gpu;
pub mod input;
pub mod layout;
pub mod loop_;
pub mod paint;
pub mod scene;
pub mod style;
pub mod text;
pub mod window;

/// libWolkit's own version (not the WolOS version): major.minor.patch.
pub const VERSION: (u32, u32, u32) = (1, 0, 0);
