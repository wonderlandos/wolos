# System API (`wol_core::system`)

What the shell and `wol sys <topic> [--json]` read about the machine.
Shape per topic: pure parser (fixture/table-tested) + read via `Host` trait (real host or fake). Linux only (ADR 0016): everything is a file under /proc and /sys, a symlink (`Host::read_link`), or a short command. `Host::is_freebsd` and `sysctl_*` remain as transitional defaults (false/None) until the last caller is gone.

| Topic | Source | TTL |
|---|---|---|
| about | `/etc/os-release`, `/proc/cpuinfo`, `/proc/meminfo`, `/proc/sys/kernel/{ostype,osrelease}` (`-wolos` = WolKernel) | once |
| stats (cpu, mem, load, uptime) | `/proc/stat`, `/proc/meminfo`, `/proc/uptime` | 0.9 s, no fork |
| processes | `/proc/[pid]/stat` into a struct-of-arrays table | 2 s, no fork |
| disks | `df -kP` | 30 s |
| temperatures | `/sys/class/thermal`, `/sys/class/hwmon` | 5 s |
| gpu | `lspci -mm`; render nodes from `/sys/class/drm` (`realtime::gpu_probe`) | once |
| hw (`modules/hardware.md`) | `/sys/bus/{pci,usb,acpi}`, `/proc/modules`, `pacman -Qq` | 30 s |
| power | `/sys/class/power_supply` (scope Device = a peripheral) | 5 s |
| network | `ip -j addr`, `ip route`, `/proc/net/route`, `nmcli` | 3 s |
| audio | `wpctl`, `pactl` (PipeWire) | 2 s |
| brightness | `/sys/class/backlight`, set through logind `SetBrightness` | 1 s |
| bluetooth | `bluetoothctl` | 5 s |
| time zone | `/etc/localtime` link, `timedatectl` | once |

Invariants (tested):
- snapshots never fail: stale value or "unknown" defaults (cached snapshot fallback)
- commands time out after 2 s
- stats sample: no fork, < 2 ms, bounded allocation
- process sampler: allocation-free after warm-up, CPU% from deltas
- root actions go through `wol-admind` (`shared::admin`), never direct
- secrets reach tools on stdin, never argv
- real-host tests read this machine's /proc and /sys (info, stats, processes, power, GPUs, zone)
