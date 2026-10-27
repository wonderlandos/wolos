# realtime

WolOS session glue for Hyprland. Hyprland draws everything; this crate only supplies policy and wiring.

| Part | Path |
|---|---|
| Hyprland IPC (`.socket.sock` requests, `.socket2.sock` events) | `src/hypr.rs` |
| Event bus client/publisher (wold `subscribe` / `publish`) | `src/bus.rs` |
| Frontmost app -> QoS UserInteractive (`foreground <pid>`, allocation-free) | `src/qos.rs` |
| GPU choice (display GPU, iGPU on hybrids, `WOL_GPU`), session env, Hyprland fragment | `src/gpu_select.rs`, `src/gpu_probe.rs`, `src/gpu_env.rs` |
| Pure policy: animation curves, title bar, placement, consent, session lock, glass params | `src/anim.rs` ... `src/glass.rs`, `src/shaders/` |
| C ABI for `plugins/wol-hypr` | `src/ffi.rs`, `include/wol_realtime.h` |

```sh
wol-realtime                       # event loop: Hyprland -> QoS + bus
wol-realtime gpu env               # export lines, sourced by wol-session
wol-realtime gpu hyprconf [--vrr]  # Hyprland config fragment
cargo test -p realtime
```

Config: `/usr/local/etc/wolos/{glass,consent,idle}.json`.
