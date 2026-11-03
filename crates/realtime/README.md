# realtime

WolOS session glue for the compositor. The compositor draws everything; this crate only supplies policy and wiring.

| Part | Path |
|---|---|
| Compositor IPC (`.socket.sock` requests, `.socket2.sock` events) | `src/hypr.rs` |
| Event bus client/publisher (wold `subscribe` / `publish`) | `src/bus.rs` |
| Frontmost app -> QoS UserInteractive (`foreground <pid>`, allocation-free) | `src/qos.rs` |
| GPU choice (display GPU, iGPU on hybrids, `WOL_GPU`), session env, compositor fragment | `src/gpu_select.rs`, `src/gpu_probe.rs`, `src/gpu_env.rs` |
| Pure policy: animation curves, title bar, placement, consent, session lock, glass params | `src/anim.rs` ... `src/glass.rs`, `src/shaders/` |
| C ABI for `plugins/wol-hypr` | `src/ffi.rs`, `include/wol_realtime.h` |

```sh
wol-realtime                       # event loop: compositor -> QoS + bus
wol-realtime gpu env               # export lines, sourced by wol-session
wol-realtime gpu hyprconf [--vrr]  # compositor config fragment
cargo test -p realtime
```

Config: `/etc/wolos/{glass,consent,idle}.json`.
