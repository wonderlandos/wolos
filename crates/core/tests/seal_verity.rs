// SPDX-License-Identifier: BSD-2-Clause
//! `seal::verity`: the in-process fs-verity of `wol seal apply` (IMG013-3-SEAL-SLOW,
//! SEAL-FLAKE). The digest the kernel measures is compared with one computed here from the
//! fs-verity specification (Merkle tree of 4096-byte SHA-256 blocks, no salt, then the SHA-256
//! of the 256-byte descriptor), so neither side vouches for itself. The tests on a real
//! verity file system run on a loop-mounted btrfs (`sudo -n`; skipped, and said so, without).
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use wol_core::seal::verity::{Stage, seal_all, seal_file};

#[path = "support/loopfs.rs"]
mod loopfs;
use loopfs::LoopFs;

fn sha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// The fs-verity file digest of `data` as `fsverity measure` prints it (hex, no `sha256:`).
fn reference(data: &[u8]) -> String {
    let root: [u8; 32] = if data.is_empty() {
        [0; 32] // an empty file is the special case: an all-zero root hash
    } else {
        let mut level = data.to_vec();
        loop {
            let hashes: Vec<[u8; 32]> = level
                .chunks(4096)
                .map(|c| {
                    let mut b = c.to_vec();
                    b.resize(4096, 0);
                    sha(&b)
                })
                .collect();
            if hashes.len() == 1 {
                break hashes[0];
            }
            level = hashes.concat();
        }
    };
    let mut desc = vec![1u8, 1, 12, 0, 0, 0, 0, 0]; // version, sha256, 4096, no salt, reserved
    desc.extend((data.len() as u64).to_le_bytes());
    desc.extend(root);
    desc.resize(256, 0); // root hash padded to 64, salt 32, reserved 144
    sha(&desc).iter().map(|b| format!("{b:02x}")).collect()
}

/// Deterministic, not periodic with the block size.
fn bytes(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 11) as u8
        })
        .collect()
}

fn put(dir: &std::path::Path, name: &str, data: &[u8]) -> PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(data).unwrap();
    f.sync_all().unwrap();
    p
}

#[test]
fn the_reference_digest_is_the_specification_s() {
    // The one published vector this can be checked against without a kernel: the empty file's
    // descriptor has an all-zero root hash and data size 0.
    let mut desc = vec![1u8, 1, 12, 0, 0, 0, 0, 0];
    desc.extend(0u64.to_le_bytes());
    desc.resize(256, 0);
    let want: String = sha(&desc).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(reference(b""), want);
    assert_ne!(reference(b"a"), reference(b"b"));
    assert_ne!(reference(&bytes(4096, 1)), reference(&bytes(4097, 1)));
}

#[test]
fn the_kernel_digest_equals_the_reference_for_every_block_shape() {
    let Some(fs) = LoopFs::new("digest") else {
        return;
    };
    // 0, under a block, exactly a block, over, one full tree block of digests (128 x 4096), one
    // byte past it (a second level), and a few blocks of arbitrary content.
    for (i, n) in [
        0usize,
        1,
        4095,
        4096,
        4097,
        128 * 4096,
        128 * 4096 + 1,
        1_000_003,
    ]
    .into_iter()
    .enumerate()
    {
        let data = bytes(n, i as u32 + 1);
        let p = put(fs.path(), &format!("f{n}"), &data);
        assert_eq!(seal_file(&p).unwrap(), reference(&data), "{n} bytes");
        // Sealed already: the same answer, no error (the EEXIST / re-run case).
        assert_eq!(seal_file(&p).unwrap(), reference(&data), "{n} bytes again");
    }
}

#[test]
fn many_files_on_four_threads_answer_in_input_order_and_hard_links_agree() {
    let Some(fs) = LoopFs::new("many") else {
        return;
    };
    let mut paths = Vec::new();
    let mut want = Vec::new();
    for i in 0..400u32 {
        let data = bytes((i as usize * 37) % 9000, i + 7);
        paths.push(put(fs.path(), &format!("f{i:03}"), &data));
        want.push(reference(&data));
    }
    // Two links of one inode in the list: whichever enables first, the other must still answer.
    let link = fs.path().join("link");
    std::fs::hard_link(&paths[5], &link).unwrap();
    paths.push(link);
    want.push(want[5].clone());
    for threads in [4, 1] {
        let got = seal_all(&paths, threads);
        assert_eq!(got.len(), paths.len());
        for (i, r) in got.into_iter().enumerate() {
            assert_eq!(
                r.unwrap().unwrap(),
                want[i],
                "{} ({threads} threads)",
                paths[i].display()
            );
        }
    }
}

#[test]
fn a_failure_is_named_by_its_step_and_stops_the_rest() {
    let Some(fs) = LoopFs::new("fail") else {
        return;
    };
    let good = put(fs.path(), "good", b"x");
    let gone = fs.path().join("missing");
    let target = put(fs.path(), "target", b"y");
    let link = fs.path().join("symlink");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let open = seal_file(&gone).unwrap_err();
    assert_eq!(open.stage, Stage::Open);
    assert!(open.to_string().contains("could not be opened"), "{open}");
    assert_eq!(
        seal_file(&link).unwrap_err().stage,
        Stage::Open,
        "never through a link"
    );
    // A file open for writing cannot be sealed (ETXTBSY): the enable step, with the system's words.
    let busy = put(fs.path(), "busy", b"zz");
    let _w = std::fs::OpenOptions::new()
        .append(true)
        .open(&busy)
        .unwrap();
    let e = seal_file(&busy).unwrap_err();
    assert_eq!(e.stage, Stage::Enable);
    let text = e.to_string();
    assert!(
        text.contains("could not be enabled") && !text.contains("no fs-verity digest"),
        "{text}"
    );
    // In a list: entries before the failure are answers, the failure is an Err, and the first
    // entry that is not an answer is an Err in order.
    let got = seal_all(&[good, busy, gone], 4);
    assert!(matches!(got[0], Some(Ok(_))));
    assert!(matches!(got[1], Some(Err(_))));
}

#[test]
fn a_file_system_without_fs_verity_says_so_from_the_kernel() {
    // /dev/shm is tmpfs, which has no fs-verity; the answer is the system's, at the enable step.
    let dir = std::path::Path::new("/dev/shm");
    if !dir.is_dir() {
        return;
    }
    let p = put(
        dir,
        &format!("wol-verity-none-{}", std::process::id()),
        b"data",
    );
    let e = seal_file(&p).unwrap_err();
    let _ = std::fs::remove_file(&p);
    assert_eq!(e.stage, Stage::Enable, "{e}");
    assert!(e.err.raw_os_error().is_some(), "{e}");
}

/// The number in docs/modules/seal.md: `cargo test -p core --test seal_verity --release --
/// --ignored --nocapture bench` (WOL_BENCH_FILES, default 71385: the files of an install).
#[test]
#[ignore = "a measurement, not a check"]
fn bench_files_per_second() {
    let n: usize = std::env::var("WOL_BENCH_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(71_385);
    for threads in [1usize, 4, 8] {
        let Some(fs) = LoopFs::sized("bench", 8192) else {
            return;
        };
        let paths: Vec<PathBuf> = (0..n)
            .map(|i| {
                let dir = fs.path().join(format!("d{}", i % 200));
                std::fs::create_dir_all(&dir).unwrap();
                put_nosync(
                    &dir,
                    &format!("f{i}"),
                    &bytes(500 + (i * 97) % 12_000, i as u32 + 1),
                )
            })
            .collect();
        let t = std::time::Instant::now();
        let got = seal_all(&paths, threads);
        let secs = t.elapsed().as_secs_f64();
        assert!(got.iter().all(|r| matches!(r, Some(Ok(_)))));
        println!(
            "BENCH {n} files, {threads} thread(s): {secs:.2} s, {:.0} files/s",
            n as f64 / secs
        );
    }
}

fn put_nosync(dir: &std::path::Path, name: &str, data: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}
