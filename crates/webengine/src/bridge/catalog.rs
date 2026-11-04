// SPDX-License-Identifier: BSD-2-Clause
//! The permission catalog of system web apps (ADR 0048). It lives in `wol_core::sysapp` because wold
//! reads it too (and checks every call again); this is the bridge's name for it.
pub use wol_core::sysapp::{PERMISSIONS, Permission, TOPICS, is_name, permission, permission_of};
