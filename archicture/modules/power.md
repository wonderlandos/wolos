# Power

The power source picks the profile; Low Power and High Power Mode override it (ADR 0014).
Code: `wol_core::power::policy` (pure: facts from read-only probes in, fixed argv out).
`wol power apply` runs the plan; udev runs it on AC changes, systemd at boot and resume, wol-admind `power-mode` after a mode change.

| Mechanism | AC (Auto) | Battery (Auto) | Low | High | Where |
|---|---|---|---|---|---|
| Energy profile | balanced | balanced (CPU prefers efficiency on battery) | power-saver | performance | `powerprofilesctl set`; switched only when it differs |
| Wi-Fi power save | off | on | on (battery) | off | `iw dev <if> set power_save`, interfaces that answer `get power_save` |
| Display off when idle | lock time + 1 min | 2 min | | | hypridle (`prefs::hypridle_conf`, system `hypridle.conf`) |
| Sleep when idle | never | 10 min | | | hypridle -> `wol power idle sleep` (needs `mem` in /sys/power/state) |
| Lid closed | suspend | suspend | | | logind; wol-admind `lid` writes `/etc/systemd/logind.conf.d/50-wolos-lid.conf` |

power-profiles-daemon drives the CPU energy preference (intel_pstate, amd-pstate-epp) and the ACPI platform profile. Without it nothing steers the CPU and the switch hides. The mode is saved in `/etc/wolos/power.json`.

Probes: `nproc`, `powerprofilesctl get`, `ls`/`cat` under `/sys/class/power_supply`, `/proc/acpi/button/lid`, `/sys/power/state`, `/sys/class/net/*/wireless`, `iw`. Entry names reach argv only when plain (`[A-Za-z0-9_.:-]`, no leading `-`).

Not offered yet: a battery charge limit (`charge_control_end_threshold`).

## Sleep, alerts, session power
- Every sleep path locks first. The systemd-sleep hook calls `wol power suspend`, which locks each session that asks for the password after sleep.
- Battery notifications at 20, 10 and 5 %. At 3 % the computer sleeps (`shell::alerts`).
- When the startup disk is under 5 % free, the shell shows one notification until space is back.
- Sleep, Restart and Shut Down go through wol-admind `session-power` (`systemctl suspend|reboot|poweroff`).
  - The account at the computer needs no password. Restart and Shut Down ask for one while another account has a session.
  - Restart, Shut Down and Log Out ask first, with a 60 s countdown.

## Commands
`wol power status|apply|mode [auto|low|high]|sleep|restart|shutdown|idle display-off|sleep [--json]`.

## Needs hardware
- Suspend and the lock before it.
- power-profiles-daemon profiles on a real laptop.
- Wi-Fi power save on iwlwifi/rtw88.
- udev power_supply change events.

Tests cover the plans with fixtures, not these effects.
