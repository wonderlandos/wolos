// SPDX-License-Identifier: BSD-2-Clause
//! Paint (plan section 2.6): the display list and its chunks, the scene lowering that fills it,
//! the token palette it resolves colours against, damage tracking and the GPU instance batcher.

pub mod batch;
pub mod damage;
pub mod geom;
pub mod glyphs;
pub mod list;
pub mod palette;
pub mod scene;
pub mod text;
