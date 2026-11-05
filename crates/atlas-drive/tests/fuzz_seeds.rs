// SPDX-License-Identifier: GPL-2.0-or-later
//! Replays the seeds of fuzz/regressions/atlas_drive_wire through the body of the fuzz target, on stable.
#[test]
fn the_seeds_replay_without_a_panic() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/regressions/atlas_drive_wire");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        atlas_drive::fuzz_one(&std::fs::read(e.path()).unwrap());
        n += 1;
    }
    assert!(n >= 5, "{n} seeds");
    // and a few hostile shapes
    for data in [&b""[..], b"\0\0\0", b"{\"nodes\":[{\"nodeId\":\"1\",\"childIds\":[\"1\"]}]}", b"{\"nodes\":[{\"nodeId\":\"a\",\"parentId\":\"b\"},{\"nodeId\":\"b\",\"parentId\":\"a\",\"childIds\":[\"a\"]}]}", &[0xff, 0xfe, 0x00]] {
        atlas_drive::fuzz_one(data);
    }
}
