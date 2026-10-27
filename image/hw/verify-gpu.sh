#!/bin/sh
# WolOS native-boot GPU proof. Run as root on real hardware after booting WolOS (USB or installed):
#   sh /usr/local/libexec/wolos/verify-gpu.sh      (or image/hw/verify-gpu.sh from a checkout)
# Writes /var/log/wolos-gpu-report.txt (send it back). Read-only except: loads lindebugfs/cpuctl,
# mounts debugfs on /sys/kernel/debug. Benchmarks need a running Wayland session (WAYLAND_DISPLAY).
# Env: WOL_REPORT (output path), WOL_BENCH=0 skips benchmarks.
set -u
OUT=${WOL_REPORT:-/var/log/wolos-gpu-report.txt}
export PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin
: > "$OUT" || { echo "verify-gpu: cannot write $OUT (run as root)" >&2; exit 1; }
sec() { printf '\n===== %s\n' "$*" >> "$OUT"; }
run() { printf '$ %s\n' "$*" >> "$OUT"; sh -c "$*" >> "$OUT" 2>&1; printf '[rc=%s]\n' "$?" >> "$OUT"; }
have() { command -v "$1" >/dev/null 2>&1; }

sec "system"
run "date -u; uname -a; cat /etc/wolos-release"
run "pciconf -lv | grep -A4 -E 'class=0x03'"
sec "driver + firmware"
run "kldstat | grep -E 'drm|i915|amdgpu|dmabuf|ttm|_bin'"
run "sysctl hw.dri"
run "dmesg | grep -iE 'drm|i915|guc|huc|dmc|firmware'"
run "grep -iE 'drm|i915|guc|huc|dmc|firmware' /var/log/messages | tail -60"
run "ls /boot/modules | grep -E '^i915_(adl|tgl|rkl|mtl)'"
run "sysctl compat.linuxkpi | grep -E 'i915_(enable_(guc|fbc|psr|dc)|modeset)'"
run "ls -l /dev/dri"
sec "GuC / HuC / DMC / FBC / PSR / frequency (debugfs)"
kldload -n lindebugfs 2>/dev/null
mkdir -p /sys/kernel/debug
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
# MSR 0x606 = RAPL units (bits 12:8 energy unit), 0x611 = package energy counter, 0x639 = PP1 (graphics).
kldload -n cpuctl 2>/dev/null
# cpucontrol prints "MSR 0x611: 0x<hi> 0x<lo>"; the counters are the low 32 bits.
msr() { cpucontrol -m "$1" /dev/cpuctl0 2>/dev/null | awk '{print $NF}'; }
u=$(msr 0x606)
if [ -n "$u" ]; then
  e0=$(msr 0x611); g0=$(msr 0x639); sleep 10; e1=$(msr 0x611); g1=$(msr 0x639)
  eu=$(( (u >> 8) & 31 ))            # energy unit = 1/2^eu J
  d=$(( (e1 - e0) & 0xffffffff )); gd=$(( (g1 - g0) & 0xffffffff ))
  echo "$d $gd $eu" | awk '{ printf "package %.2f W, graphics (PP1) %.2f W (raw %s %s, unit 2^-%s J)\n", $1/2^$3/10, $2/2^$3/10, $1, $2, $3 }' >> "$OUT"
else
  echo "no RAPL MSR access (cpuctl)" >> "$OUT"
fi
run "sysctl dev.cpu.0.freq hw.acpi.acline dev.cpu.0.cx_usage"

# A/B: one knob, same benchmark, score + RAPL energy per side. WOL_AB=0 skips.
# ab NAME "A setup" "B setup": setup is eval'd before the bench; i915_wol_* sysctls are restored after.
sec "A/B (WOL_AB=${WOL_AB:-1})"
energy() { [ -n "$u" ] && echo "$(msr 0x611) $(msr 0x639)"; }
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
  if [ -n "$e0" ] && [ -n "$e1" ]; then
    w=$(echo "$e0 $e1 $eu $((t1 - t0))" | awk '{ s=($5>0)?$5:1; printf "pkg %.2f W gpu %.2f W", (($3-$1)%4294967296)/2^$5/s, (($4-$2)%4294967296)/2^$5/s }')
  fi
  echo "  $1: $r ${w:+($w)}" >> "$OUT"
}
ab() {
  name=$1; a=$2; b=$3
  saved=$(sysctl -e compat.linuxkpi.i915_wol_waitboost compat.linuxkpi.i915_wol_deadline_boost 2>/dev/null)
  echo "$name" >> "$OUT"; side "A [$a]" "$a"; side "B [$b]" "$b"
  [ -n "$saved" ] && sysctl $saved >/dev/null 2>&1
  unset INTEL_DEBUG
}
if [ "${WOL_AB:-1}" = 1 ]; then
  if sysctl -n compat.linuxkpi.i915_wol_waitboost >/dev/null 2>&1; then
    ab "waitboost: upstream (all waiters) vs foreground only (ADR 0008)" \
      "sysctl compat.linuxkpi.i915_wol_waitboost=1" "sysctl compat.linuxkpi.i915_wol_waitboost=2"
    ab "frame-deadline boost off vs on (ADR 0008)" \
      "sysctl compat.linuxkpi.i915_wol_deadline_boost=0" "sysctl compat.linuxkpi.i915_wol_deadline_boost=1"
  else
    echo "WolOS i915kms not loaded (no compat.linuxkpi.i915_wol_*): fork A/B skipped" >> "$OUT"
  fi
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
for d in /sys/kernel/debug/dri/*; do [ -r "$d/i915_rps_boost_info" ] && run "cat $d/i915_rps_boost_info"; done
echo "verify-gpu: wrote $OUT"
