// SPDX-License-Identifier: BSD-2-Clause
//! Number conversions that name what happens at the edge: a float goes to the nearest integer and
//! saturates, a wire word is read as the signed number it carries. `as` casts to the narrow
//! integer types are forbidden (RR-017); an enum or an index goes through `scene::ids` (`Ix`,
//! `to_u8`, `to_u16`, `to_u32`).

/// The nearest integer of `v` (NaN is 0), wide enough that the narrowing below only saturates.
fn nearest(v: f32) -> i128 {
    if v.is_nan() { 0 } else { v.round() as i128 }
}

pub fn f32_to_i16(v: f32) -> i16 {
    i16::try_from(nearest(v).clamp(i128::from(i16::MIN), i128::from(i16::MAX))).unwrap_or(0)
}

pub fn f32_to_u16(v: f32) -> u16 {
    u16::try_from(nearest(v).clamp(0, i128::from(u16::MAX))).unwrap_or(0)
}

pub fn f32_to_u8(v: f32) -> u8 {
    u8::try_from(nearest(v).clamp(0, i128::from(u8::MAX))).unwrap_or(0)
}

pub fn f32_to_u32(v: f32) -> u32 {
    u32::try_from(nearest(v).clamp(0, i128::from(u32::MAX))).unwrap_or(0)
}

/// A wire word holding an `i32` (two's complement), as that number.
pub fn wire_i32(w: u32) -> i32 {
    i32::from_ne_bytes(w.to_ne_bytes())
}

/// An `i32` clamped into `i16`.
pub fn i32_to_i16(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

/// An `i32` clamped into `u16` (negative is 0).
pub fn i32_to_u16(v: i32) -> u16 {
    u16::try_from(v.clamp(0, i32::from(u16::MAX))).unwrap_or(0)
}
