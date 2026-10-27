//! Window thumbnails for Mission Control via `hyprland_toplevel_export_v1` (works for windows
//! that are covered or on other desktops, unlike screen-region copies).
//! `wol-window-shot <address>` writes `WOLSHOT <w> <h> <stride> <format>\n` + raw rows to stdout.

/// The protocol's toplevel handle: the low 32 bits of Hyprland's window address.
pub fn handle(address: &str) -> Option<u32> {
    let hex = address.trim_start_matches("0x");
    u64::from_str_radix(hex, 16).ok().map(|a| a as u32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// wl_shm format code.
    pub format: u32,
}

/// Byte order of a 32-bit wl_shm pixel in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Bgra,
    Bgrx,
    Rgba,
    Rgbx,
}

pub fn layout(format: u32) -> Option<Layout> {
    match format {
        0 => Some(Layout::Bgra),
        1 => Some(Layout::Bgrx),
        0x3432_4241 => Some(Layout::Rgba),
        0x3432_4258 => Some(Layout::Rgbx),
        _ => None,
    }
}

const MAGIC: &str = "WOLSHOT";

pub fn encode(f: Frame, pixels: &[u8]) -> Vec<u8> {
    let mut v = format!(
        "{MAGIC} {} {} {} {}\n",
        f.width, f.height, f.stride, f.format
    )
    .into_bytes();
    v.extend_from_slice(pixels);
    v
}

pub fn decode(bytes: &[u8]) -> Option<(Frame, &[u8])> {
    let nl = bytes.iter().position(|&b| b == b'\n')?;
    let head = std::str::from_utf8(&bytes[..nl]).ok()?;
    let mut w = head.split(' ');
    if w.next()? != MAGIC {
        return None;
    }
    let mut n = || w.next()?.parse::<u32>().ok();
    let f = Frame {
        width: n()?,
        height: n()?,
        stride: n()?,
        format: n()?,
    };
    let px = &bytes[nl + 1..];
    (px.len() as u64 >= f.stride as u64 * f.height as u64).then_some((f, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_from_addresses() {
        for (a, want) in [
            ("0x55d6976ed990", Some(0x976ed990)),
            ("55d6976ed990", Some(0x976ed990)),
            ("0x1", Some(1)),
            ("zz", None),
            ("", None),
        ] {
            assert_eq!(handle(a), want, "{a}");
        }
    }

    #[test]
    fn shm_layouts() {
        for (f, want) in [
            (0, Some(Layout::Bgra)),
            (1, Some(Layout::Bgrx)),
            (0x3432_4241, Some(Layout::Rgba)),
            (0x3432_4258, Some(Layout::Rgbx)),
            (0x3033_5241, None),
        ] {
            assert_eq!(layout(f), want, "{f:#x}");
        }
    }

    #[test]
    fn frame_round_trip() {
        let f = Frame {
            width: 2,
            height: 1,
            stride: 8,
            format: 1,
        };
        let px = [1, 2, 3, 4, 5, 6, 7, 8];
        let bytes = encode(f, &px);
        assert_eq!(decode(&bytes), Some((f, &px[..])));
        assert_eq!(decode(b"WOLSHOT 2 1 8 1\n123"), None, "short pixel data");
        assert_eq!(decode(b"garbage"), None);
    }
}
