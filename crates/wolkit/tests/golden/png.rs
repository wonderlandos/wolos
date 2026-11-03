// SPDX-License-Identifier: BSD-2-Clause
//! A minimal PNG writer and reader for the golden images (RGBA8, filter 0 only).

use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::decompress_to_vec_zlib;

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, t) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = table[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &compress_to_vec_zlib(&raw, 9));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Decodes a PNG written by [`encode`]: `(width, height, rgba)`.
pub fn decode(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if png.get(..8) != Some(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Err("not a PNG".into());
    }
    let mut at = 8;
    let (mut w, mut h) = (0u32, 0u32);
    let mut idat = Vec::new();
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = &png[at + 4..at + 8];
        let data = png.get(at + 8..at + 8 + len).ok_or("truncated chunk")?;
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(data[0..4].try_into().unwrap());
                h = u32::from_be_bytes(data[4..8].try_into().unwrap());
                if data[8..13] != [8, 6, 0, 0, 0] {
                    return Err("only RGBA8 non-interlaced PNGs".into());
                }
            }
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        at += 12 + len;
    }
    let raw = decompress_to_vec_zlib(&idat).map_err(|e| format!("inflate: {e:?}"))?;
    let stride = w as usize * 4;
    if raw.len() != (stride + 1) * h as usize {
        return Err("size mismatch".into());
    }
    let mut out = Vec::with_capacity(stride * h as usize);
    for row in raw.chunks_exact(stride + 1) {
        if row[0] != 0 {
            return Err("unsupported PNG filter".into());
        }
        out.extend_from_slice(&row[1..]);
    }
    Ok((w, h, out))
}

#[test]
fn png_roundtrip() {
    let px: Vec<u8> = (0..4 * 5 * 4).map(|i| (i * 7) as u8).collect();
    let (w, h, back) = decode(&encode(4, 5, &px)).unwrap();
    assert_eq!((w, h), (4, 5));
    assert_eq!(back, px);
}
