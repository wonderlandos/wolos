# The sealed system volume, `wol seal` (ADR 0025)

After an install or an update the slot's subvolume is read-only and every file under the
verity directories carries fs-verity; the digests go to a manifest inside the slot and the
manifest's own digest goes to the ESP next to the slot's kernel (`crates/core/src/seal.rs`,
decisions; `crates/controller/src/seal.rs`, the verbs). At boot `wol seal check` compares the
three and, on a mismatch, boots the other slot and tells the person.

## `wol seal apply` turns fs-verity on in this process
`wol seal apply ROOT DIGESTFILE` walks the slot, writes the complete manifest and seals the files.
The sealing step is `wol_core::seal::verity` (`crates/core/src/system/verity.rs`): the kernel's
`FS_IOC_ENABLE_VERITY` and `FS_IOC_MEASURE_VERITY` on a descriptor, with what `fsverity enable`
uses by default (SHA-256, 4096-byte blocks, no salt, no signature), on 4 worker threads.

- One job per inode: a hard link (perl's `perlbug` and `perlthanks`) takes the digest of the first
  path of its inode in walk order (IMG013-SEAL-HARDLINK); a file that has fs-verity already (an
  enable that answers `EEXIST`) gives the digest it has.
- The answers come back in the order of the paths and the manifest is a sorted map, so its bytes
  do not depend on which thread was first. A test builds the same tree twice on a loop-mounted
  btrfs, seals one with the old one-file-at-a-time `fsverity` steps and one in process, and
  compares the manifest and root digest byte for byte
  (`the_in_process_seal_writes_the_manifest_the_fsverity_program_did`).
- A failure says the step and the system's reason: `usr/lib/os.py: fs-verity could not be
  enabled: Inappropriate ioctl for device (os error 25)`, `... fs-verity digest could not be
  read: ...`, `... could not be opened: ...`. Nothing waits on a child, so there is no time-out to
  be mistaken for an answer (SEAL-FLAKE: a kill of `fsverity measure` after a clock jump was
  reported as "no fs-verity digest" and one install failed).
- `WOL_SEAL_FSVERITY_BIN=1` in the environment of `wol seal apply` makes it run the `fsverity`
  program per file instead (measure, enable, measure: the pre-0.13 way). For diagnosis only; it is
  slow, and a program that is killed is reported as such ("the tool failed or was stopped").

### Measurement
Before (IMAGE-013-3, an install of 71,385 manifest entries, three `fsverity` processes per file,
one thread): about 14 minutes. After, `bench_files_per_second` in
`crates/core/tests/seal_verity.rs` (release build, 71,385 files of 0.5 to 12 KiB, a loop-mounted
btrfs on a 32-core AWS box under other builds' load, 2026-11-05):

| threads | time | files/s |
|---|---|---|
| 1 | 2.03 s | 35,000 |
| 4 | 2.07 s | 34,500 |
| 8 | 4.21 s | 17,000 |

IMAGE-013-3's C stand-in measured 19,000 files/s on 1 thread, 39,000 on 4, 6,300 on 16: past
about 8 threads the file system's own locks make it slower, which is why the count is 4
(`seal::verity::THREADS`). On this box one thread already saturates the disk's metadata path, so
4 threads did not add; the gain over the process-per-file seal is about 400 times either way
(14 minutes to about 2 seconds).

### Tests
`crates/core/tests/seal_verity.rs`: the kernel's digest equals one computed from the fs-verity
specification (Merkle tree, descriptor, no `fsverity` program involved) for 0, 1, 4095, 4096,
4097, 128 x 4096, 128 x 4096 + 1 and about 1 MB of content; 400 files on 4 and on 1 thread answer in
order, two links of one inode agree; failures name the step. They need a loop-mounted btrfs
(`crates/core/tests/support/loopfs.rs`: `sudo -n`, `mkfs.btrfs`, `mount -o loop`) and say
`SKIPPED` without one; `WOL_REQUIRE_LOOP=1` makes a missing loop file system a failure.
