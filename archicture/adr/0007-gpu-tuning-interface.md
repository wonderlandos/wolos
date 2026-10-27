# 0007 - GPU tuning interface
Status: accepted (v1 implemented: `gpu_tune.rs`, `frame_stats.rs`, `wol-realtime gpu caps|sample`, `hypr stats`)
Context: iGPU throughput is bound by shared memory bandwidth (`modules/memory-bandwidth.md`). Tuning must apply to i915/xe now and amdgpu/nvidia later without forking compositor logic. `crates/realtime` already probes GPUs (`gpu_probe.rs`, `gpu_select.rs`) and exposes a C ABI to wol-hypr (`wol_realtime.h`).
Decision:
- Contract lives in `crates/realtime/src/gpu_tune.rs` (domain-local; promote to `crates/shared` only when a second domain reads it). Plain data, no I/O in the types:

```rust
pub struct GpuCaps {
    pub driver: String,                 // gpu_select::GpuInfo::driver
    pub render_compression: bool,       // CCS / DCC / AFBC
    pub display_compression: Option<bool>, // FBC state, None = unknown
    pub modifiers: Vec<u64>,            // DRM format modifiers for scanout ARGB8888
    pub freq_mhz: Option<FreqRange>,    // min/max/current
    pub freq_control: bool,             // writable min/max
}
pub struct GpuSample { pub t_ns: u64, pub busy_pct: Option<f32>, pub freq_mhz: Option<u32>,
    pub gpu_energy_uj: Option<u64>, pub mem_bytes: Option<u64> } // None = not exposed
pub trait GpuBackend {
    fn caps(&self) -> GpuCaps;
    fn sample(&self) -> GpuSample;
    fn set_freq(&self, min_mhz: u32, max_mhz: u32) -> Result<(), Error>; // Err if !freq_control
}
```
- Backends: `i915` first (lindebugfs/sysctl freq, `i915_fbc_status`, RAPL PP1 via `cpuctl`), then `amdgpu` (sysfs pp_dpm, DCC from modifiers). Unknown vendor: `caps()` from KMS modifiers only, `sample()` all `None`.
- Consumers: wol-hypr reads `caps` once via the C ABI (prefer compressed modifiers, pick effect resolution) and writes per-frame timing (ms, damage px, pass count) to a ring in `realtime`. wold (root) owns `set_freq` and the race-to-idle policy; the compositor never writes sysfs.
- v1: `GpuCaps`, `GpuSample` (freq, energy, busy if present), i915 backend, frame-time ring, `wol hypr stats`. Deferred: `set_freq` policy, amdgpu/nvidia backends, memory-byte counters (no FreeBSD source today), xe.
Tests: table tests per backend over fixture files (sysctl/debugfs text -> `GpuCaps`); unknown vendor yields all `None`.
Consequences: one small trait, one backend per vendor. Missing counters are `None`, never faked. Bytes/frame stays a derived metric until a counter exists.
