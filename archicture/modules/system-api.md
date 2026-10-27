# System API (`wol_core::system`)

What the shell and `wol sys <topic> [--json]` read about the machine.
Shape per topic: pure parser (fixture/table-tested) + read via `Host` trait (real host or fake). FreeBSD syscalls behind `cfg(target_os = "freebsd")`.

| Topic | FreeBSD | Linux (dev host) | TTL |
|---|---|---|---|
| stats (cpu, mem, load, uptime) | sysctl `kern.cp_time`, `vm.stats.*`, `kern.boottime` | `/proc/stat`, `/proc/meminfo`, `/proc/uptime` | 0.9 s, no fork |
| processes | `kern.proc` sysctl into struct-of-arrays table | `/proc` | 2 s, no fork |
| disks | `df -kP` | same | 30 s |
| temperatures | `dev.cpu.N.temperature` | `/sys/class/thermal` | 5 s |
| gpu | `pciconf -lv`, `hw.dri.*` | `/sys/class/drm` | once |
| hw (devices, driver/firmware state; `modules/hardware.md`) | `pciconf -lv`, `devinfo -v`, `usbconfig`, `kldstat`, `pkg` | empty | 30 s |
| power | `hw.acpi.battery.*` | `/sys/class/power_supply` | 5 s |
| network | `ifconfig` | same | 3 s |
| audio | `mixer -o`, `/dev/sndstat` | `wpctl` | 2 s |
| brightness | `backlight -q` | `/sys/class/backlight` | 1 s |

Invariants (tested):
- snapshots never fail: stale value or "unknown" defaults (cached snapshot fallback)
- commands time out after 2 s
- stats sample: no fork, < 2 ms, bounded allocation
- process sampler: allocation-free after warm-up, CPU% from deltas
- root actions go through `wol-admind` (`shared::admin`), never direct
- secrets reach tools on stdin, never argv
