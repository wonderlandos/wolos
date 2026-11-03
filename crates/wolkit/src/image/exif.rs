// SPDX-License-Identifier: BSD-2-Clause
//! The EXIF orientation tag (0x0112) of a picture: which of the eight turns and mirrors makes it
//! upright. Only the first directory is read; every offset is bounded by the block, so a broken
//! block reads as "upright" (1).

/// The orientation (1 to 8) in a TIFF-structured EXIF block, with or without the `Exif\0\0`
/// prefix a JPEG APP1 segment carries. 1 when absent or unreadable.
pub fn orientation(block: &[u8]) -> u8 {
    let b = block.strip_prefix(b"Exif\0\0").unwrap_or(block);
    read(b).filter(|o| (1..=8).contains(o)).unwrap_or(1)
}

/// Orientations 5 to 8 turn the picture a quarter, so its width and height swap.
pub fn swaps_axes(o: u8) -> bool {
    (5..=8).contains(&o)
}

fn read(b: &[u8]) -> Option<u8> {
    let le = match b.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    // A directory entry is 12 bytes: tag, type, count, value (inline when it fits in 4 bytes).
    for k in 0..count.min(512) {
        let e = ifd.checked_add(2 + 12 * k)?;
        if u16_at(e)? == 0x0112 {
            // Type 3 is SHORT; its value sits in the first two bytes of the value field.
            if u16_at(e + 2)? != 3 {
                return None;
            }
            return u8::try_from(u16_at(e + 8)?).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(le: bool, o: u16) -> Vec<u8> {
        let mut v: Vec<u8> = if le { b"II".to_vec() } else { b"MM".to_vec() };
        let p16 = |v: &mut Vec<u8>, x: u16| {
            v.extend_from_slice(&if le { x.to_le_bytes() } else { x.to_be_bytes() })
        };
        let p32 = |v: &mut Vec<u8>, x: u32| {
            v.extend_from_slice(&if le { x.to_le_bytes() } else { x.to_be_bytes() })
        };
        p16(&mut v, 42);
        p32(&mut v, 8);
        p16(&mut v, 2);
        // An unrelated tag first, then the orientation.
        p16(&mut v, 0x010f);
        p16(&mut v, 2);
        p32(&mut v, 4);
        p32(&mut v, 0);
        p16(&mut v, 0x0112);
        p16(&mut v, 3);
        p32(&mut v, 1);
        p16(&mut v, o);
        p16(&mut v, 0);
        p32(&mut v, 0);
        v
    }

    #[test]
    fn orientation_is_read_in_both_byte_orders() {
        assert_eq!(orientation(&block(true, 6)), 6);
        assert_eq!(orientation(&block(false, 8)), 8);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(block(false, 3));
        assert_eq!(orientation(&app1), 3);
    }

    #[test]
    fn broken_blocks_read_as_upright() {
        assert_eq!(orientation(b""), 1);
        assert_eq!(orientation(b"II*\0\xff\xff\xff\xff"), 1);
        assert_eq!(orientation(&block(true, 9)), 1);
        let b = block(true, 6);
        for n in 0..b.len() {
            let o = orientation(&b[..n]);
            assert!((1..=8).contains(&o));
        }
        assert!(swaps_axes(6) && !swaps_axes(3));
    }
}
