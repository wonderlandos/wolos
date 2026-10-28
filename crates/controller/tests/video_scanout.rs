//! Memory-bandwidth #2: fullscreen buffers go straight to a KMS plane and browser video is decoded
//! by VA-API into dmabufs (NV12 1.5 B/px, no CPU copy) instead of software RGBA.
use std::path::PathBuf;

fn read(p: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(p),
    )
    .unwrap()
}

#[test]
fn direct_scanout_and_vaapi_decode_are_on() {
    let perf = read("desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf");
    assert!(perf.contains("direct_scanout = 2"));
    let pol = read("desktop/overlay/usr/lib/firefox/distribution/policies.json");
    for k in [
        r#""media.ffmpeg.vaapi.enabled": { "Value": true"#,
        r#""media.hardware-video-decoding.force-enabled": { "Value": true"#,
    ] {
        assert!(pol.contains(k), "policies.json lacks {k}");
    }
}

#[test]
fn wayland_latency_defaults_are_tuned() {
    let perf = read("desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf");
    for k in [
        "allow_tearing = true",
        "vrr = 2",
        "vfr = true",
        "force_zero_scaling = true",
        "match:class = ^steam_app_\\d+$",
        "immediate = on",
    ] {
        assert!(perf.contains(k), "wolos-perf.conf lacks {k}");
    }
}

#[test]
fn session_gtk_renderer_is_gpu_when_a_render_node_exists() {
    let s = read("desktop/overlay/usr/bin/wol-session");
    assert!(s.contains("/dev/dri/renderD"), "no render node probe");
    assert!(
        s.contains("GSK_RENDERER=${GSK_RENDERER:-ngl}"),
        "ngl not default"
    );
    assert!(
        s.contains("GSK_RENDERER=${GSK_RENDERER:-cairo}"),
        "no cairo fallback"
    );
}
