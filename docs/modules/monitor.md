# System Monitor

System Monitor (`wol-monitor`) shows what the computer is doing right now: how busy the processor, memory, disks, network, graphics card and sensors are over the last minute, which apps are open and what each one costs, every process, the disks' space, and the services that start with the session.

Code: data in `crates/core/src/monitor/` (BSD-2-Clause, no GTK), the app in `crates/shell/src/ui/monitor/` and `crates/shell/src/bin/wol-monitor.rs`, the graph in `crates/shell/src/kit/graph.rs`, words in `crates/shell/src/copy_monitor.rs`. Desktop entry: `desktop/overlay/usr/share/applications/org.wolos.monitor.desktop` (icon `wolos-system-monitor`; `design/icons/aliases.tsv` also maps `org.wolos.monitor`, `utilities-system-monitor`, `org.gnome.SystemMonitor`, `gnome-system-monitor`, `htop` and `btop` onto it).

## Why

Memory Saver (ADR 0026) rests and pauses apps in the background, and the per-app scopes (`app-*.scope`) already hold the true cost of each app. Without a monitor a person cannot see either: which app is taking the memory, whether an app is paused or only resting, what is keeping the fan on. A terminal `top` lists processes, not apps, and knows nothing of Memory Saver.

## Layers

"Everything separated": data, model and views are separate modules, and only one of them touches the system.

| Layer | What it is | Code |
|---|---|---|
| Data | Pure parsers, one per file, plus one `Sampler` that reads them under a `Root` (`/` on a running system, a fixture tree in tests) and keeps the previous counters to turn them into rates. No GTK, no threads. | `wol_core::monitor::{cpu, net, disk, gpu, temps, apps, procs, startup, sampler}` |
| Feed | The only app code that reads or changes the system: the sampler thread, the installed apps, the user manager over D-Bus, the signals of Quit and End Process. Every call runs off the GTK thread (`feed::later`). | `ui/monitor/feed.rs` |
| Model | The last sample, a minute of history per graph (`Ring`, 60 values), the rows each page shows (apps named and sorted, the process tree), numbers as people read them. Pure. | `ui/monitor/model.rs` |
| Views | Kit widgets that draw the model and hand choices to callbacks (`Actions`); no reads, no timers. | `ui/monitor/{mod,overview,apps,processes,disks,network,startup,tile,confirm}.rs`, `kit/graph.rs` |

## Sampling

One thread (`wol-monitor`) samples once a second, on the second (`feed::TICK`, no drift), and sends the `Sample` over a bounded channel; it stops when the window closes and the channel does. Every second it reads `/proc/stat` (total and per core), `/proc/meminfo`, `/proc/loadavg`, `/proc/net/dev` (loopback skipped; kind and link from `/sys/class/net/<name>`), `/proc/diskstats` (whole disks in `/sys/block`, not loop, RAM, zram, optical or network devices; sectors are 512 bytes), `/sys/class/drm/cardN/device` (`gpu_busy_percent`, `mem_info_vram_*`: amdgpu has them, i915 and nouveau do not, and the page says so), hwmon and thermal zones (the system API's reader, `system::stats::Temperature`), and the app scopes. What no open page shows is not read (`sampler::Want`): the process table only for Processes (the system API's `ProcessSampler` over the same root), PSS only for Apps and at most every 5 s per app (`PSS_EVERY`), disk space (`df -kP`, the only fork) only for Disks and at most every 10 s (`DISKS_EVERY`).

Apps are the cgroup v2 `app-*.scope` units under `/sys/fs/cgroup/user.slice/user-<uid>.slice` (the session's user only): systemd launcher scopes in `user@<uid>.service/app.slice` and the `app-wol-<id>-<pid>.scope` units Memory Saver moves apps into. Per scope: `cgroup.procs`, `memory.current`, `memory.swap.current`, `cpu.stat usage_usec` (CPU percent of one core since the last sample), `cgroup.events frozen`, and PSS summed from each process's `smaps_rollup`. The app id comes from the unit name (`apps::app_of`: launcher prefix and launch tag stripped, systemd escapes decoded); name and icon from the installed desktop entries (`apps::frontmost_name`, `apps::icon_for`, the menu bar's lookup). Memory Saver's view comes from `/var/run/wol-memsaver.json` (`shared::memsaver::Status`): Napping or a frozen cgroup is Paused, Saving is Resting. Energy is Low, Moderate or High from the mean CPU of the last minute (below 5 %, below 30 %, above): there is no per-app power counter an unprivileged reader can use, and CPU time is what costs battery.

## Pages

| Page | Shows | Does |
|---|---|---|
| Overview | Seven graph cards, two a row from 1024 px, one in a narrow window: Processor (total, cores, load), Memory, Compressed memory (swap), Disk activity (read and written), Network activity (received and sent), Graphics (load, or a sentence when the card reports none), Temperature (hottest sensor). | |
| Apps | One row per app scope: icon, name, memory (PSS, else `memory.current`), CPU, energy; a badge when Resting or Paused. A footnote says what energy means and whether Memory Saver is on. | Quit: asks in a sheet, then SIGTERM to every process of the scope. A paused app cannot answer, so its button is Force Quit (SIGKILL, which a frozen cgroup still delivers). |
| Processes | Every process as a tree (children under their parent, busiest first, foldable), or the toolbar filter's matches (name or number) as a flat list. A list view: only the rows on screen are widgets. Columns: name, number, CPU, memory. | End Process (toolbar, destructive) asks in a sheet, then SIGTERM. A process of another account is refused by the kernel and a toast says so. |
| Disks | Space per mounted disk (one row per device: a btrfs `/` and `/home` are one disk; tmpfs and the like are not listed) with a meter; read and write activity per physical disk. | |
| Network | One graph per interface (received and sent), its kind (Wi-Fi, Ethernet, Virtual), connected or not, and what it moved since it came up. | |
| Startup | The user manager's services (`ListUnitFiles` on the session bus), enabled or disabled; static, masked, generated, alias and template units are not listed. The person's own (`~/.config/systemd/user`) switch; the ones WolOS ships show On or Off only. | A switch calls `EnableUnitFiles`/`DisableUnitFiles` and `Reload`; it waits for the answer and flips back on a refusal. Open Login Items opens Settings > General (apps that open at login). |

The status bar holds counts only ("6 apps, 284 processes"). The window is `AppWindow` (HIG §1): the sidebar collapses below 640 px. Colours are theme tokens only: each graph series is a drawing layer with a tone class (`tone-accent`, `tone-success`, `tone-warning`) and the grid takes `--line` (`.graph-grid`), so light, dark and the accent apply live.

## Tests

- `crates/core/tests/monitor.rs` on `crates/core/tests/fixtures/monitor/t0` and `t1` (the same machine two seconds apart, laid out like `/`): per-core CPU, memory and swap, network rates without loopback, whole disks and their rates, mounts one per device, GPU cards and not connectors, temperatures, app scopes of one user with memory, PSS, CPU, frozen and Memory Saver state, energy, the process table with parents and CPU, what was not asked for is not read, an empty root, the startup rows. Written failing first (10 of 12 failed on the stubs), then made to pass.
- Unit tests in `wol_core::monitor` (escaped and tagged scope names, malformed lines), `ui::monitor::model` (history length, tree and filter, samples moving the graphs, number formats), `kit::graph` (x placement, scale), `copy_monitor` (every sentence through `copy::lint`), `ui::monitor::feed` (the page's wants reach the sampler).
- Goldens (`crates/shell/tests/golden_cases/monitor.rs`, light and dark): every page over the demo minute (`ui::monitor::demo`) at 1280x800 and 480x360, the empty Apps page, the Overview before the first sample, Startup loading and not read, a filter with no match, the Force Quit question.
- `crates/shell/tests/monitor_entry.rs`: the desktop entry starts `wol-monitor` under `org.wolos.monitor` with the set's icon, and the icon aliases name it.

`WOL_MONITOR_DEMO=1 wol-monitor` shows the demo minute instead of the machine (screenshots); `wol-monitor <Page>` opens on a page.
