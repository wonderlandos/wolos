// SPDX-License-Identifier: BSD-2-Clause
//! Window size negotiation with the compositor (xdg-shell): the size a configure asks for, held
//! inside the app's limits, and the buffer size at the current scale. The limits (`set_min_size`,
//! `set_max_size`) are sent before the first commit so the compositor's placement sees them
//! (plan section 2.7); a fixed-size window sends the same size as both.

/// A `configure` as received: 0 means "you choose".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configure {
    pub width: i32,
    pub height: i32,
    pub activated: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub resizing: bool,
}

impl Configure {
    /// Parse the toplevel configure's state array (native-endian u32 values).
    pub fn with_states(width: i32, height: i32, states: &[u8]) -> Configure {
        let mut c = Configure {
            width,
            height,
            ..Default::default()
        };
        for s in states.as_chunks::<4>().0 {
            let v = u32::from_ne_bytes([s[0], s[1], s[2], s[3]]);
            match v {
                1 => c.maximized = true,
                2 => c.fullscreen = true,
                3 => c.resizing = true,
                4 => c.activated = true,
                _ => {}
            }
        }
        c
    }
}

/// The limits to send: `(min, max)` with 0 meaning none.
pub fn limits(
    min: (u32, u32),
    max: (u32, u32),
    fixed: Option<(u32, u32)>,
) -> ((i32, i32), (i32, i32)) {
    let i = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    match fixed {
        Some((w, h)) => ((i(w), i(h)), (i(w), i(h))),
        None => ((i(min.0), i(min.1)), (i(max.0), i(max.1))),
    }
}

/// The logical size to use after `c`, starting from `current`, held inside the limits. A
/// maximized or fullscreen window takes the compositor's size as is.
pub fn size_after(
    c: &Configure,
    current: (u32, u32),
    min: (u32, u32),
    max: (u32, u32),
) -> (u32, u32) {
    let pick = |asked: i32, cur: u32, lo: u32, hi: u32, tiled: bool| -> u32 {
        let v = u32::try_from(asked).ok().filter(|v| *v > 0).unwrap_or(cur);
        if tiled {
            return v.clamp(1, 16_384);
        }
        let v = v.max(lo);
        let v = if hi > 0 { v.min(hi) } else { v };
        v.clamp(1, 16_384)
    };
    let tiled = c.maximized || c.fullscreen;
    (
        pick(c.width, current.0, min.0, max.0, tiled),
        pick(c.height, current.1, min.1, max.1, tiled),
    )
}

/// Device pixels for a logical length at `scale` (rounded half up, as the viewport scales).
pub fn device(px: u32, scale: f32) -> u32 {
    let s = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let f = (f64::from(px) * f64::from(s)).round().clamp(1.0, 65_536.0);
    // The clamp makes the conversion exact.
    u32::try_from(f as u64).unwrap_or(1)
}
