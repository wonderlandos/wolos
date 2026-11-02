// SPDX-License-Identifier: GPL-2.0-or-later
//! wopkg's parsers of downloaded bytes (audit B10): .zsync control files, AppImage ELF sections and
//! update information, index segments. Never panic, whatever the network or the disk hands over.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wopkg::{appimage, index::segment::Segment, zsync};

fuzz_target!(|data: &[u8]| {
    if let Ok(c) = zsync::parse(data) {
        let known = zsync::known_blocks(&c, data);
        let _ = zsync::missing_ranges(&c, &known);
    }
    let _ = appimage::sections(data);
    let _ = appimage::is_appimage(data);
    let _ = appimage::UpdateInfo::parse(&String::from_utf8_lossy(data));
    // Segment::open maps a file: hand it the bytes as one.
    let path = std::env::temp_dir().join(format!("wopkg-fuzz-segment-{}", std::process::id()));
    std::fs::write(&path, data).unwrap();
    if let Some(s) = Segment::open(&path) {
        let _ = (s.len(), s.is_empty(), s.fingerprint().len());
    }
});
