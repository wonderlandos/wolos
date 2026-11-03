#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# WolOS native-boot GPU proof. Run as root on real hardware after booting WolOS (USB or installed):
#   sh /usr/lib/wolos/verify-gpu.sh      (or files/usr/lib/wolos/verify-gpu.sh from a checkout)
# Writes /var/log/wolos-gpu-report.txt (send it back). Read-only except: mounts debugfs on
# /sys/kernel/debug, loads intel_rapl_msr when no RAPL zone exists, and the A/B section flips the
# WolKernel boost knobs it finds (restored after). Benchmarks need a running Wayland session.
# Env: WOL_REPORT (output path), WOL_BENCH=0 skips benchmarks, WOL_AB=0 skips the A/B runs.
set -u
OUT=${WOL_REPORT:-/var/log/wolos-gpu-report.txt}
export PATH=/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/sbin:/bin:/sbin
: > "$OUT" || { echo "verify-gpu: cannot write $OUT (run as root)" >&2; exit 1; }
sec() { printf '\n===== %s\n' "$*" >> "$OUT"; }
run() { printf '$ %s\n' "$*" >> "$OUT"; sh -c "$*" >> "$OUT" 2>&1; printf '[rc=%s]\n' "$?" >> "$OUT"; }
have() { command -v "$1" >/dev/null 2>&1; }
I915=/sys/module/i915/parameters
HINTS=/sys/kernel/wolos

sec "system"
run "date -u; uname -a; cat /etc/os-release"
run "lspci -nnk | grep -A3 -E 'VGA|3D|Display'"
sec "driver + firmware"
run "lsmod | grep -E '^(i915|xe|amdgpu|nouveau|drm|ttm)'"
run "ls /sys/module | grep -xE 'i915|xe|amdgpu|nouveau'"
for p in enable_guc enable_fbc enable_psr enable_dc modeset; do
  [ -r "$I915/$p" ] && run "grep -H . $I915/$p"
done
run "dmesg | grep -iE 'drm|i915|xe |guc|huc|dmc|firmware'"
run "journalctl -k -b --no-pager | grep -iE 'drm|i915|guc|huc|dmc|firmware' | tail -60"
run "ls /usr/lib/firmware/i915 | grep -E '^(adl|tgl|rkl|mtl|dg2)'"
run "ls -l /dev/dri"
sec "WolKernel hints (${HINTS})"
if [ -d "$HINTS" ]; then run "grep -rH . $HINTS"; else echo "no $HINTS (not the WolKernel)" >> "$OUT"; fi
sec "GuC / HuC / DMC / FBC / PSR / frequency (debugfs)"
# debugfs is built into the kernel: nothing to load, only to mount.
mount | grep -q ' /sys/kernel/debug ' || mount -t debugfs debugfs /sys/kernel/debug 2>>"$OUT"
for d in /sys/kernel/debug/dri/*; do
  [ -d "$d" ] || continue
  for f in name gt/uc/guc_info gt/uc/huc_info gt0/uc/guc_info gt0/uc/huc_info i915_dmc_info i915_fbc_status \
    i915_edp_psr_status gt/frequency gt0/frequency i915_frequency_info i915_runtime_pm_status; do
    [ -r "$d/$f" ] && run "cat $d/$f"
  done
done
sec "GL / EGL"
have eglinfo && run "eglinfo -B"
have glxinfo && run "glxinfo -B"
sec "Vulkan"
have vulkaninfo && run "vulkaninfo --summary"
sec "VA-API"
run "echo LIBVA_DRIVER_NAME=\${LIBVA_DRIVER_NAME:-unset}"
for n in /dev/dri/renderD*; do [ -e "$n" ] && have vainfo && run "LIBVA_DRIVER_NAME=iHD vainfo --display drm --device $n"; done
sec "benchmarks"
if [ "${WOL_BENCH:-1}" = 1 ]; then
  have vkmark && run "timeout 120 vkmark -b vertex:duration=10 -b texture:duration=10 -b shading:duration=10"
  have glmark2-wayland && [ -n "${WAYLAND_DISPLAY:-}" ] && run "timeout 120 glmark2-wayland -b build:duration=10 -b texture:duration=10 -b shading:duration=10"
  have glmark2-es2-drm && [ -z "${WAYLAND_DISPLAY:-}" ] && run "timeout 120 glmark2-es2-drm -b build:duration=10 -b shading:duration=10"
  have glmark2 && [ -n "${DISPLAY:-}" ] && run "timeout 120 glmark2 -b build:duration=10"
else
  echo "skipped (WOL_BENCH=0)" >> "$OUT"
fi
sec "compute (Vulkan, anv: FLOPS + memory bandwidth)"
have vkpeak && [ "${WOL_BENCH:-1}" = 1 ] && run "timeout 300 vkpeak 0"
sec "idle power (RAPL package energy, 10 s)"
# The kernel's powercap RAPL zones: package-0 and its uncore (graphics, PP1) subzone, in microjoules.
[ -d /sys/class/powercap/intel-rapl:0 ] || modprobe -q intel_rapl_msr 2>>"$OUT"
zone() {
  for z in /sys/class/powercap/intel-rapl:0 /sys/class/powercap/intel-rapl:0:*; do
    [ "$(cat "$z/name" 2>/dev/null)" = "$1" ] && [ -r "$z/energy_uj" ] && { echo "$z"; return 0; }
  done
  return 1
}
pkg=$(zone package-0) || pkg=""
gfx=$(zone uncore) || gfx=""
uj() { if [ -n "$1" ]; then cat "$1/energy_uj" 2>/dev/null; else echo 0; fi; }
# "pkg gfx range": the counters wrap at the package zone's max_energy_range_uj.
energy() { [ -n "$pkg" ] && echo "$(uj "$pkg") $(uj "$gfx") $(cat "$pkg/max_energy_range_uj" 2>/dev/null || echo 0)"; }
# shellcheck disable=SC2016 # an awk program: $n are awk fields
watts='{ r=($3>0)?$3:4294967296; s=($7>0)?$7:1; p=$4-$1; if (p<0) p+=r; g=$5-$2; if (g<0) g+=r; printf "pkg %.2f W gpu %.2f W", p/1e6/s, g/1e6/s }'
if [ -n "$pkg" ]; then
  e0=$(energy); sleep 10; e1=$(energy)
  echo "$e0 $e1 10" | awk "$watts" >> "$OUT"
  echo " (package ${pkg##*/}${gfx:+, graphics ${gfx##*/}})" >> "$OUT"
else
  echo "no readable RAPL zone (/sys/class/powercap/intel-rapl:0, root only)" >> "$OUT"
fi
run "grep -H . /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor /sys/class/power_supply/*/online"
run "grep -H . /sys/devices/system/cpu/cpu0/cpuidle/state*/name /sys/devices/system/cpu/cpu0/cpuidle/state*/usage"

# A/B: one knob, same benchmark, score + RAPL watts per side. WOL_AB=0 skips.
# ab NAME "A setup" "B setup": setup is eval'd before the bench; a knob is restored after.
sec "A/B (WOL_AB=${WOL_AB:-1})"
bench() {
  if have vkmark; then timeout 60 vkmark -b vertex:duration=8 -b shading:duration=8 2>&1 | grep -i 'score'
  elif have glmark2-es2-drm; then timeout 60 glmark2-es2-drm -b shading:duration=8 2>&1 | grep -i 'score'
  else echo "no benchmark (vkmark/glmark2)"; fi
}
side() {   # $1 label, $2 setup (eval'd here so exports reach the bench)
  eval "$2" >/dev/null 2>&1
  e0=$(energy); t0=$(date +%s)
  r=$(bench)
  t1=$(date +%s); e1=$(energy)
  w=""
  if [ -n "$e0" ] && [ -n "$e1" ]; then w=$(echo "$e0 $e1 $((t1 - t0))" | awk "$watts"); fi
  echo "  $1: $r ${w:+($w)}" >> "$OUT"
}
ab() {
  echo "$1" >> "$OUT"; side "A [$2]" "$2"; side "B [$3]" "$3"
  unset INTEL_DEBUG
}
if [ "${WOL_AB:-1}" = 1 ]; then
  # The WolKernel's GPU boost switches (foreground-only i915 waitboost, frame-deadline boost; ADR
  # 0008): each patch has a runtime off switch, a 0/1 file under /sys/kernel/wolos or an i915 param.
  knobs=0
  for k in "$HINTS"/*boost* "$I915"/*wol*; do
    [ -w "$k" ] || continue
    v=$(cat "$k" 2>/dev/null)
    case $v in 0 | 1 | Y | N) ;; *) continue ;; esac
    knobs=$((knobs + 1))
    ab "${k}: off vs on (ADR 0008)" "echo 0 > $k" "echo 1 > $k"
    echo "$v" > "$k"
  done
  [ "$knobs" -gt 0 ] || echo "no WolKernel boost knob ($HINTS/*boost*, $I915/*wol*): fork A/B skipped" >> "$OUT"
  ab "render compression: INTEL_DEBUG=noccs vs default (memory-bandwidth #3)" \
    "export INTEL_DEBUG=noccs" "unset INTEL_DEBUG"
  run "wol-realtime power show"
  pl1=$(wol-realtime power show 2>/dev/null | sed -n 's/.*"pl1":{"watts":\([0-9.]*\).*/\1/p')
  if [ -n "$pl1" ] && ! wol-realtime power show | grep -q '"locked":true'; then
    ab "package PL1: firmware ${pl1} W vs 15 W (power limit)" \
      "wol-realtime power set --pl1 $pl1" "wol-realtime power set --pl1 15"
    wol-realtime power set --pl1 "$pl1" >/dev/null 2>&1
  else
    echo "PL1 A/B skipped (no RAPL access or MSR locked by firmware)" >> "$OUT"
  fi
else
  echo "skipped" >> "$OUT"
fi
for d in /sys/kernel/debug/dri/*; do
  for f in i915_rps_boost_info gt0/rps_boost; do [ -r "$d/$f" ] && run "cat $d/$f"; done
done
echo "verify-gpu: wrote $OUT"
