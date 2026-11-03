// SPDX-License-Identifier: BSD-2-Clause
//! Number conversions without the `as` casts `RR-017` forbids. Integer index and size conversions
//! are `crate::scene::ids::{Ix, to_u8, to_u16, to_u32}`; this module is for the float to integer
//! step the painters and the layout take, which saturate instead of wrapping and send `NaN` to 0.

/// Float to `i128` (saturating, `NaN` is 0), the one wide step the narrower helpers go through.
fn wide(f: f32) -> i128 {
    f as i128
}

pub fn trunc_i32(f: f32) -> i32 {
    i32::try_from(wide(f)).unwrap_or(if f < 0.0 { i32::MIN } else { i32::MAX })
}

pub fn round_i32(f: f32) -> i32 {
    trunc_i32(f.round())
}

pub fn floor_i32(f: f32) -> i32 {
    trunc_i32(f.floor())
}

pub fn ceil_i32(f: f32) -> i32 {
    trunc_i32(f.ceil())
}

/// A non-negative float as a `u32` (negative is 0).
pub fn trunc_u32(f: f32) -> u32 {
    u32::try_from(wide(f).max(0)).unwrap_or(u32::MAX)
}

pub fn round_u32(f: f32) -> u32 {
    trunc_u32(f.round())
}

/// A float as a byte: clamped to 0..=255.
pub fn sat_u8(f: f32) -> u8 {
    u8::try_from(wide(f).clamp(0, 255)).unwrap_or(u8::MAX)
}

/// A non-negative float as a `usize` (negative is 0).
pub fn trunc_usize(f: f32) -> usize {
    usize::try_from(wide(f).max(0)).unwrap_or(usize::MAX)
}

pub fn to_i32(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

/// Narrow to `i16`, saturating at both ends.
pub fn to_i16(n: i32) -> i16 {
    i16::try_from(n.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

pub fn to_i64(n: impl TryInto<i64>) -> i64 {
    n.try_into().unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_saturate_and_nan_is_zero() {
        assert_eq!(trunc_i32(f32::NAN), 0);
        assert_eq!(trunc_i32(3.9), 3);
        assert_eq!(trunc_i32(-3.9), -3);
        assert_eq!(trunc_i32(1e30), i32::MAX);
        assert_eq!(trunc_i32(-1e30), i32::MIN);
        assert_eq!(round_i32(2.5), 3);
        assert_eq!(floor_i32(-0.1), -1);
        assert_eq!(ceil_i32(0.1), 1);
        assert_eq!(trunc_u32(-4.0), 0);
        assert_eq!(sat_u8(300.0), 255);
        assert_eq!(sat_u8(-1.0), 0);
        assert_eq!(trunc_usize(-1.0), 0);
        assert_eq!(to_i32(u32::MAX), i32::MAX);
    }
}
