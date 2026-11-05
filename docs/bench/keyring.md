# Keyring benchmark (ADR 0054 section 5)

Measured 2026-11-04 on the forge box: Intel Xeon Platinum 8488C, 32 cpus, kernel 7.0.0-1014-aws, ext4 on
NVMe (EBS), release build of `wol-keyring-bench` (`crates/wolkeyring/src/bin/wol-keyring-bench.rs`).
One run per row, not repeated; the disk's `fsync` tail varies, so read p99 of any row that syncs as an
order of magnitude, not a promise. Items are 64-byte secrets with three attributes (`application`, `site`,
`user`). Times are microseconds as seen by the caller (a request and its reply, or the call).

Commands (from the repo root, binaries in the target folder):

    wol-keyring-bench inproc --items 10000 --ops 5000     # the library in this process
    wol-keyring-bench socket --items 10000 --ops 5000     # the fast path of a daemon
    dbus-run-session -- ours.sh --items 10000 --ops 2000  # wol-keyringd on a private session bus, bench `bus`
    dbus-run-session -- gk.sh   --items N     --ops 2000  # gnome-keyring-daemon 46.1 (--unlock --components=secrets), bench `bus`

(`ours.sh` and `gk.sh` start the daemon on the private bus, unlock it, and run `wol-keyring-bench bus`;
the same bench code calls `CreateItem`, `SearchItems` and `Item.GetSecret` on whoever owns
`org.freedesktop.secrets`.)

## 10 000 items

| Door | Operation | Calls | p50 | p99 |
|---|---|---:|---:|---:|
| in process | get | 5000 | 18.2 | 37.4 |
| in process | search, one attribute (unique) | 5000 | 1.4 | 2.0 |
| in process | search, two attributes (~100 hits) | 5000 | 91.5 | 196.0 |
| in process | set, new item (durable) | 10000 | 5845 | 23634 |
| in process | set, replace (durable) | 5000 | 5830 | 11230 |
| fast path (Unix socket) | get | 5000 | 28.6 | 66.6 |
| fast path | search, one attribute | 5000 | 12.7 | 31.5 |
| fast path | set, new item | 10000 | 7446 | 14054 |
| Secret Service (D-Bus), wol-keyringd | get | 2000 | 205.0 | 3420 |
| Secret Service, wol-keyringd | search, one attribute | 2000 | 94.7 | 967 |
| Secret Service, wol-keyringd | set, new item | 10000 | 8632 | 15007 |

A read p50 of 18 microseconds includes the audit line. A write is two `fsync`s (the record, then its
audit line): the cost of a durable change on this disk (about 6 ms), and the same in any store that
syncs before it answers.

## Against gnome-keyring 46.1 over D-Bus (the same bench code, the same box)

gnome-keyring rewrites its whole file on every `CreateItem`, so its write time grows with the number
of items; its 10 000-item populate did not finish in the 25 minutes the run was given, so the
10 000-item comparison is **not measured**. Measured sizes:

| Items | | gnome-keyring p50 / p99 | wol-keyringd p50 / p99 |
|---:|---|---:|---:|
| 1000 | get | 385.7 / 1623 | 209.1 / 2486 |
| 1000 | search, one attribute | 533.1 / 3702 | 95.5 / 136.5 |
| 1000 | set, new item | 28080 / 210638 | 9015 / 14655 |
| 3000 | get | 1014.5 / 2445 | (10 000: 205.0 / 3420) |
| 3000 | search, one attribute | 1236.8 / 3302 | (10 000: 94.7 / 967) |
| 3000 | set, new item | 56560 / 434498 | (10 000: 8632 / 15007) |

gnome-keyring's reads, searches and writes all grew with the item count between 1000 and 3000 items
(read p50 x2.6, search x2.3, write x2.0); wol-keyringd's did not between 1000 and 10 000 (an attribute
index, one appended record). The reason on gnome-keyring's side was not investigated.

Read the comparison with its limits in mind: gnome-keyring does no access-list check, no audit line
and no per-call caller identification, wol-keyringd does all three on every D-Bus call (reading
`/proc/<pid>` and a cached digest of the executable); the D-Bus rows are dominated by the bus round
trip for both. The p99 of wol-keyringd's D-Bus get (3.4 ms) is about 16 times its p50; the cause was not
isolated (candidates: the daemon's 5-second timer signing a checkpoint under the same lock, a disk
`fsync` of another request, scheduling).

## What changed because of the first measurement

The first run of `get` in process gave p50 3.6 ms and p99 90 ms: every read appended an audit line and
synced it, and every 64th signed a checkpoint (ML-DSA-65 and Ed25519, then another sync) inside the read.
Now a read's audit line is appended unsynced (the next durable line or checkpoint syncs it) and the
daemon's timer signs the due checkpoint: p50 15.7 us, p99 34.6 us on a 1000-item store (same box).

## Not measured

- The 10 000-item gnome-keyring numbers (above).
- Unlock time was timed once, by hand: `wol-keyring get` (one process: start, Argon2id 64 MiB x 3
  passes, one read) takes 0.12 to 0.13 s wall and 68 MB peak on this box (release build, three runs).
  No fingerprint, TPM or FIDO2 path was timed (no such hardware here).
- Memory of the daemon at 10 000 items; no regression guard in CI yet.

## After the security review fixes (same box, 1000-item stores, one run each, noisy)

The review added identity checks on every call: the program is re-compared with the pinned one before
and after the work, and on D-Bus the bus is asked for the caller's credentials twice. Measured cost:

| Door | Operation | before p50 / p99 | after p50 / p99 |
|---|---|---:|---:|
| fast path | get | 27.7 / 4537 (10k run: 28.6 / 66.6) | 45.4 / 281 |
| fast path | search | 14.8 / 39.6 | 35.3 / 48.9 |
| D-Bus | get | 209 / 2486 | 360 / 2303 and 474 / 1531 (two runs; a third, run while the disk was busy with a build, gave 1074 and 1672) |
| D-Bus | search | 95.5 / 136.5 | 95.0 / 1115 and 110.2 / 314 |

Timing inside the daemon (a debug print, removed): the caller's identification takes 110 to 185 us per
D-Bus call, of which about 45 us is the second credentials query. The D-Bus get is now about the same
as gnome-keyring's 386 us at 1000 items, instead of half of it, and it does the access check and the
audit that gnome-keyring does not.
