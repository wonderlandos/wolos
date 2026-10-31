# 0027 - ClamAV in its own process, with data the person fetches
Status: accepted. Changes the libclamav and signature-data points of ADR 0024.

Context: the first-use check should reach the full ClamAV signature set, and the parsers and unpackers only libclamav has. libclamav is GPL-2.0-only. libwol stays BSD-2-Clause, and the rule engine's tree holds Apache-2.0-only code, which cannot share a program with GPL-2.0-only code. Redistribution of the signature data has no sign-off, so it cannot be in the image. The library costs 1 GB of memory and 6.5 s to load.

Decision:
- `wol-xguard-clam` (`crates/xguard-clam`) is the only program that links libclamav. Its dependencies are libc and that library. Everything else passes it an open file over a Unix socket and reads one line: `clean`, `found NAME` or `error WHY`. No library of ours links it, and neither does anything libwol reaches.
- It starts on demand (`wolos-clam.socket`) and exits after 120 idle seconds, so at rest it costs nothing. It has no network and no write access.
- The image carries the `clamav` package and no signature data. Settings > Privacy & Security has a switch, off by default; it enables `wolos-clam-update.timer`, and freshclam fetches straight from the ClamAV project to this computer. Off deletes the data. Settings shows the state from `clam-state.json`.
- After each fetch `clam-update` builds the lean signature file here (`local/full.xgdb`), so everyday first-use checks use the same data with no ClamAV process running.
- ClamAV is asked only for deep candidates: tagged downloads, removable disks, Windows programs, AppImages, `.wol`, and installers through `wol-xguard scan --json`. wopkg calls that command, so it links nothing.
- The verdict cache is unchanged: a clean file is asked once, and never again until it or the signatures change.
- No answer from ClamAV is `incomplete`, never clean. The guard still lets the file run, and it is looked at again.
- The guard's unit must not have a mount namespace. Kernel marks hold on mounts, so in a namespace of its own it watches copies nobody uses and stops nothing, silently. `PrivateNetwork` makes one too. `tests/units.rs` guards this.

Measured (c7i.4xlarge, ext4, ClamAV 1.5.4, 3,628,245 signatures, ClamAV loaded during the guard-on rounds):

| What | Result |
|---|---|
| ClamAV load | 6.5 s warm, 7.0 s cold |
| ClamAV resident | 1.0 GB (peak 1.1 GB), while loaded; 0 once it exits |
| First use, 857 KB `.exe`, ClamAV loaded | 58 ms (15 ms with ClamAV off) |
| First use, ClamAV not loaded | 5.7 s: the program waits for the load |
| Second run of that file | 1.4 ms |
| Scan rate, random data | about 30 MB/s |
| Cached exec, off / on | 205.6 / 203.0 us (rounds vary by 5%: no measurable cost) |
| `open` in scope, off / on | 804 / 814 ns |
| `stat`, off / on | 415 / 415 ns |
| App launch, cached, off / on | 5.04 / 5.01 ms |
| Game asset load, cold, off / on | 14.02 / 14.03 s |
| Kernel compile, source cold, signature file in memory, off / on | 79.19 / 79.40 s (+0.26%, rounds vary by 0.9%) |
| Guard at rest | 10.0 MB resident, no wakeups in 60 s |

False positives: 0 across all 113,213 files under `/usr` (141,767 objects, 5.1 GB) and 559 Arch packages (228,381 objects) from 17 popular applications, through both engines. Four large packages were not fully unpacked by the lean engine and four test archives inside Go are damaged on purpose; both are reported as incomplete. ClamAV's own limits apply too: by default it skips files over 100 MB and stops at 400 MB per scan.

Consequences:
- The 30 MB budget holds for the guard, not for the machine while ClamAV is loaded.
- A deep candidate opened after two idle minutes waits about 6 s. The guard's wait is capped at 30 s; a slower disk may hit that and get `incomplete`.
- Files over ClamAV's limits are clean as far as it can say. The lean engine reads them.
- The image grows by the `clamav` package (185 MiB installed).
- The data's terms are the ClamAV project's, and its users accept them by turning this on.
