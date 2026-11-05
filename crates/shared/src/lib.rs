// SPDX-License-Identifier: BSD-2-Clause
//! Repository-wide contracts and primitives. Transport- and storage-agnostic where practical.
pub mod admin;
pub mod atlas;
#[cfg(unix)]
pub mod atomic;
pub mod bounded;
pub mod bundle;
pub mod errors;
pub mod events;
pub mod hardware;
pub mod i18n;
#[cfg(unix)]
pub mod jobs;
pub mod layout;
pub mod login;
pub mod memsaver;
pub mod package;
pub mod packages;
pub mod par;
pub mod patience;
pub mod plugins;
pub mod portal;
pub mod qos;
pub mod safe_path;
pub mod shell;
#[cfg(unix)]
pub mod sys;
pub mod theme;
pub mod update;
pub mod view;
pub mod widgets;
