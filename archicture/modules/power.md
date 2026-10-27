# Power

The power source picks the profile; Low Power and High Power Mode override it (ADR 0014).
Code: `wol_core::power::policy` (pure: facts from read-only probes in, fixed argv out).
`wol power apply` runs the plan; devd runs it on AC changes, rc.d `wolos_power` at boot and resume, wol-admind `power-mode` after a mode change.

| Mechanism | AC (Auto) | Battery (Auto) | Low | High | Where |
|---|---|---|---|---|---|
| Speed Shift energy preference | 50 | 75 | 85 / 100 | 0 | `dev.hwpstate_intel.N.epp`, every CPU |
| powerd flags (only without Speed Shift) | `-a hiadaptive` | powerd's `adaptive` | `-a minimum -b minimum` | `-a maximum -b hiadaptive` | `powerd_flags`; restarted only when powerd runs and the flags change |
| Deepest idle state | Cmax | Cmax | Cmax | C1 on AC | `hw.acpi.cpu.cx_lowest` |
| Wi-Fi power save | off | on | on (battery) | off | `ifconfig wlanN powersave`, PMGT-capable interfaces only |
| Display off when idle | lock time + 1 min | 2 min | | | hypridle (`prefs::hypridle_conf`, system `hypridle.conf`) |
| Sleep when idle | never | 10 min | | | hypridle -> `wol power idle sleep` (needs S3) |
| Lid closed | sleep (S3) | sleep (S3) | | | wol-admind `lid` |

AMD CPUs get powerd (no Speed Shift driver for them in 15.1). The mode is saved in `/usr/local/etc/wolos/power.json`.

Left as FreeBSD sets them, no tunable:
- i915 RC6 and frame buffer compression are on by default where the GPU supports them.
- USB devices idle into suspend after `hw.usb.power_timeout` (30 s).
- PCIe ASPM stays whatever the firmware set: FreeBSD has no policy switch.

At boot, PCI functions that no driver claims go to D3 (`hw.pci.do_power_nodriver=1`). Otherwise an unclaimed audio DSP keeps the package out of deep idle states.

Not offered: a battery charge limit (FreeBSD 15.1 has no interface for it) and codec power gating (snd_hda has no switch).

## Sleep, alerts, session power
- Every sleep path locks first. rc.suspend runs `wolos_power suspend`, which calls `wol power suspend` and locks each session that asks for the password after sleep.
- Battery notifications at 20, 10 and 5 %. At 3 % the computer sleeps (`shell::alerts`).
- When the startup disk is under 5 % free, the shell shows one notification until space is back.
- Sleep, Restart and Shut Down go through wol-admind `session-power`.
  - The account at the computer needs no password. Restart and Shut Down ask for one while another account has a session.
  - Restart, Shut Down and Log Out ask first, with a 60 s countdown.

## Commands
`wol power status|apply|mode [auto|low|high]|sleep|restart|shutdown|idle display-off|sleep [--json]`.

## Needs hardware
- S3 sleep and the lock before it.
- The EPP values on a real Speed Shift laptop.
- Wi-Fi power save on iwlwifi/rtw88.
- devd ACAD events.

Tests cover the plans with fixtures, not these effects.
