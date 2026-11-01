# Memory bandwidth (iGPU)

Ceiling on iGPUs: shared LPDDR, not FLOPS. Unit of cost: bytes moved per frame.
Baseline (derived): 1920x1200 RGBA8 = 9.2 MB per full read or write; at 60 Hz = 553 MB/s per full-screen pass.
Contract: ADR 0007. Hardware: `intel-graphics.md`. Current config: `hyprland-fork.md`.

Legend: Layer = C compositor (wol-hypr/Hyprland), D kernel driver, M Mesa, A apps. Agnostic = same code on every vendor; Backend = per-vendor code or knob.

| Paradigm | Mechanism | Saving (source) | Layer | Agnostic? | Measure |
|---|---|---|---|---|---|
| Lossless render compression (Intel CCS, AMD DCC, Arm AFBC) | per-block delta/constant encoding of render targets, fast-clear | AFBC: up to 50% system bandwidth ([Arm](https://www.arm.com/technologies/graphics-technologies/arm-frame-buffer-compression)); DCC: lossless block deltas, no % published ([GPUOpen](https://gpuopen.com/dcc-overview/)); CCS: "generally reduces bandwidth" ([Mesa MR 25003](https://gitlab.freedesktop.org/mesa/mesa/-/merge_requests/25003)) | M, D (modifier) | Agnostic via DRM format modifiers; compression itself is vendor | modifier in use (`drm_info`/plane state), A/B with `INTEL_DEBUG=noccs` |
| Display FBC | display engine scans out a compressed copy of the primary plane | qualitative, best on flat UI ([kernel FBC doc](https://kernel.org/doc./html/next/gpu/intel-display/fbc.html)) | D | Backend (i915 `enable_fbc`; AMD/NV have own) | `i915_fbc_status` (debugfs), package power idle |
| Transaction elimination | skip tile writeback when tile CRC unchanged | up to 99% framebuffer writes on UI ([Arm blog](https://developer.arm.com/community/arm-community-blogs/b/mobile-graphics-and-gaming-blog/posts/should-have-gone-to-bandwidth-savers)) | HW | Mali only; damage tracking is the portable equivalent | n/a on Intel |
| TBDR / memoryless targets | keep color/depth in on-chip tile memory, write once; drop unused attachments | removes full depth buffer traffic ([Apple TT 602](https://developer.apple.com/videos/play/tech-talks/602/)) | M, C | Agnostic API: `loadOp/storeOp = DONT_CARE`, `glInvalidateFramebuffer`; gain only on tilers | GPU memory read/write counters |
| Tile-local compute | process a tile in workgroup shared memory (SLM), one global read + one write | see reduction row | C, A | Agnostic (GLSL/SPIR-V compute) | bytes/frame = (reads+writes) x size |
| Damage tracking + buffer age | redraw only damaged rects | linear in damaged area: 1% damage = 1% of 553 MB/s (derived) | C | Agnostic | `debug:damage_blink`, frame-time log damage area |
| Direct scanout / planes | client buffer goes to a KMS plane, no composite pass | removes composite read+write; "very power efficient, 3D engine not used" ([Mutter MR 3177](https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/3177)); extra planes can cost power ([KWin MR 6170](https://invent.kde.org/plasma/kwin/-/merge_requests/6170)) | C, D | Agnostic (KMS atomic + modifiers) | `hyprctl` scanout state, package power |
| Dual-Kawase blur | downsample chain, small kernel per level | beats optimized Gaussian ([Intel, Strugar](https://intel.com/content/www/us/en/developer/articles/technical/an-investigation-of-fast-real-time-gpu-based-image-blur-algorithms.html)); bytes: Gaussian 2-pass full res ~4 full-screen passes vs dual-Kawase 2 levels ~0.8 (derived: 1/4+1/16 per level, down+up) | C | Agnostic | frame-time per blur pass |
| Half/quarter-res effects | shadows, blur, glow at reduced res, upsample in final pass | 1/4 or 1/16 of bytes (derived) | C | Agnostic | same |
| Cached blur behind static | reuse blurred backdrop until backdrop damages | blur cost -> 0 on static frames (`new_optimizations`, `xray`, already on) | C | Agnostic | count blur passes/s while idle |
| Pass/kernel fusion | fold shadow+blur+tint+round-corner into one shader; fewer FBOs | each removed pass = one RT write + read (derived, 2x9.2 MB at 1920x1200) | C | Agnostic | FBO count per frame |
| Format choice | RGBA8/sRGB for UI; RGBA16F only for HDR; RGB10A2 for 10-bit | RGBA16F = 2x bytes of RGBA8 (8 vs 4 B/px, derived) | C, A | Agnostic | allocated format per RT |
| Hierarchical reduction | subgroup op -> SLM workgroup reduce -> one global write per group | 30x over naive, 73% of peak BW ([Harris, NVIDIA](https://developer.download.nvidia.com/compute/cuda/1.1-Beta/x86_website/projects/reduction/doc/reduction.pdf)) | C (luma/histogram), A | Agnostic (`GL_KHR_shader_subgroup`) | GB/s vs STREAM peak |
| Interaction-net reduction (HVM2/Bend) | fork/join graph rewriting on GPU | bitonic sort 0.21 s RTX 4090 vs 0.96 s M3 Max CPU ([Bend](https://github.com/HigherOrderCO/Bend)); CUDA only | A (offline jobs) | No: NVIDIA/CUDA only | n/a |
| Cache-aware layout (shared LLC) | tiled (Y/Tile4) layouts, keep working set in LLC; CPU+GPU share LLC on Intel | qualitative; LLC is shared, use it for small RTs | M, D | Backend (cache policy/MOCS) | LLC miss via hwpmc (CPU side only) |
| Zero-copy (dmabuf + modifiers) | share buffers, never read back to CPU | removes one copy per frame per buffer (derived: 9.2 MB each way) | C, M, A | Agnostic (linux-dmabuf, already on) | absence of `glReadPixels`/shm copies |
| VA-API decode to scanout | decoded NV12 dmabuf to plane or one sample | NV12 1.5 B/px vs RGBA8 4 B/px (derived) | A, C, D | Agnostic API (VA-API/dmabuf), backend driver | `vainfo`, plane in use |
| Frequency vs bandwidth | race-to-idle; low GT freq when BW-bound; RC6/DC states | qualitative; ties to PSR/DC6 (`intel-graphics.md`) | D | Backend (i915 sysfs, amdgpu `power_dpm_force_performance_level`) | RAPL PP1/PKG energy per frame |

Interaction nets, honest verdict: compositor work is regular, data-parallel and bandwidth-bound; graph rewriting adds pointer traffic and no locality. Useful only for irregular offline CPU/GPU jobs (search, layout solve). Use plain hierarchical reduction for GPU work.

## Top 5 for WolOS

| Rank | Change | Expected gain | Effort | Status (0.11.1) |
|---|---|---|---|---|
| 1 | Damage-only frames everywhere: verify no full redraw from shell/OSD/menu bar; damage-area metric in frame log | idle/typing frames from 100% to <5% of screen bytes | S (measure + fix offenders) | metric done: wol-hypr ring, `wol-realtime hypr stats` (`damage_pct`, `full_frames`); offenders: measure on hardware |
| 2 | Direct scanout + planes for fullscreen video/games, VA-API NV12 straight to plane | composite pass removed; 4 -> 1.5 B/px for video | S-M (config on; verify on DRM) | `direct_scanout = 2`; Firefox VA-API decode on (policies.json); test `video_scanout.rs` |
| 3 | Render compression via modifiers on all compositor RTs and scanout (Tile4/Y + CCS on Xe) | up to ~50% RT bandwidth (AFBC class figure) | M (modifier negotiation in Aquamarine) | Aquamarine negotiates by default; session never sets `AQ_NO_MODIFIERS`/`INTEL_DEBUG` (test); A/B `noccs` in verify-gpu.sh; `GpuCaps.modifiers` empty until KMS IN_FORMATS is read |
| 4 | Effects at reduced res + fusion: shadow/glow at 1/4, blur stays dual-Kawase cached, fuse tint+round+shadow | 2-4 fewer full-screen passes while animating | M (wol-hypr shaders) | not started: needs pass counting in the ring first (passes = 1 lower bound today) |
| 5 | Telemetry + freq policy (ADR 0007): bytes/frame and energy/frame, race-to-idle when idle | makes 1-4 measurable; avoids high GT freq on BW-bound frames | M | telemetry done (`gpu caps|sample`: freq, FBC, RAPL PP1 µJ); freq policy: ADR 0008 boost gate + power limits |

## Measurement (ADL-P 8086:46a8, FreeBSD 15.1, native)

| Need | Tool on FreeBSD | Status | Fallback |
|---|---|---|---|
| GPU busy/freq (intel_gpu_top) | `intel_gpu_top` (`igt-gpu-tools`) | needs i915 PMU via `perf_event_open`: FreeBSD has no such syscall -> not usable | `sysctl hw.dri.0` and lindebugfs `i915_frequency_info`, `i915_fbc_status`, `i915_edp_psr_status` (`mount -t lindebugfs lindebugfs /sys/kernel/debug`) |
| i915 PMU counters | i915 PMU through LinuxKPI | not exposed (no perf subsystem in LinuxKPI); verify `strings i915kms.ko \| grep i915_pmu` | GL `GL_INTEL_performance_query` / `VK_INTEL_performance_query` in Mesa if `i915` OA works; else none |
| Memory bandwidth counters (IMC uncore) | `pmcstat` (hwpmc) | ADL uncore IMC events: check `pmccontrol -L`; likely absent | RAPL energy: `cpucontrol -m 0x611` (PKG), `0x639` (PP1 = GPU) on `/dev/cpuctl0` |
| Memory BW peak (CPU) | STREAM (`stream.c`, build with `cc -O3 -fopenmp`) | works | - |
| Memory BW peak (GPU) | `vkpeak` / `clpeak` if packaged; else a Vulkan compute copy probe | check `pkg search` | `vkcube` timing is not a BW test |
| Frame time | wol-hypr hook on `render` begin/end, log `CLOCK_MONOTONIC` + damage area + pass count to a ring, dump on `wol hypr stats` | to build (ADR 0007) | `HYPRLAND_TRACE`/`debug:overlay = true` |
| Power | RAPL above, `acpiconf -i 0` battery rate | works | - |

Protocol: fixed scene (static desktop, typing, blur window drag, 1080p VA-API video), 60 s each, report bytes/frame (derived from passes x size), ms/frame p50/p95, PP1 and PKG W.
