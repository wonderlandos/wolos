# 0014 - the power source picks the profile
Status: accepted
Context: Low Power Mode used to set powerd flags, which did nothing on Speed Shift CPUs (powerd is off there), and nothing changed when the charger was unplugged. A laptop should follow its power source without being asked; the user only overrides.
Decision:
- One pure policy (`wol_core::power::policy`): read-only facts (power source, CPU driver, idle states, Wi-Fi interfaces, S3) and the saved mode (auto, low, high) produce one plan of fixed argv. `wol power apply` runs it from devd (AC changes, priority above power_profile), rc.d at boot and resume, and wol-admind after a mode change. Nothing is set that the machine lacks.
- Speed Shift energy preference first; powerd only where cpufreq has no Speed Shift; deepest idle state except High Power on AC; Wi-Fi power save on battery. FreeBSD defaults that already save power (i915 RC6/FBC, USB suspend) are kept, not re-set.
- Battery-only idle steps live in hypridle (display off at 2 min, sleep at 10 min) and ask `wol power idle`, which does nothing on AC.
- Sleep always locks first (rc.suspend hook), whichever path asked for it. Sleep, Restart and Shut Down go through wol-admind `session-power`: the account at the computer needs no password, unless Restart or Shut Down would end another account's session.
Consequences: every power change is one code path with table tests; effects on real hardware (S3, EPP values, Wi-Fi power save) are checked on laptops, not in tests. No battery charge limit until FreeBSD has an interface for it.
