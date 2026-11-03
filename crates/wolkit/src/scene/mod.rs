// SPDX-License-Identifier: BSD-2-Clause
//! The retained scene (plan section 2.2): a struct-of-arrays arena of nodes addressed by slot, the
//! patch stream that changes it (I10) and the traits the pipeline stages code against.
//!
//! - [`ids`]: node ids and limits
//! - [`kind`]: node kinds, roles, state and dirty bits
//! - [`prop`]: the property table and value checks
//! - [`patch`]: the patch decoder, validator and writer
//! - [`apply`]: atomic application with a journal
//! - [`read`]: [`SceneRead`] and [`SceneWrite`], the stage interface
//! - [`arena`]: the [`Scene`]
//! - [`strings`]: interned text

pub mod apply;
pub mod arena;
pub mod ids;
pub mod kind;
pub mod patch;
pub mod prop;
pub mod read;
pub mod strings;

pub use apply::{Applied, Journal};
pub use arena::{AnimReq, Scene};
pub use ids::{Idx, MAX_DEPTH, MAX_NODES, NIL, NodeId};
pub use kind::{Kind, dirty, state};
pub use patch::{Op, Patch, PropValue, Writer};
pub use read::{A11y, Rect, SceneRead, SceneWrite, walk};
