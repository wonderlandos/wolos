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
    let perf = read("desktop/overlay/usr/local/share/wolos/hypr/wolos-perf.conf");
    assert!(perf.contains("direct_scanout = 2"));
    let pol = read("desktop/overlay/usr/local/lib/firefox/distribution/policies.json");
    for k in [
        r#""media.ffmpeg.vaapi.enabled": { "Value": true"#,
        r#""media.hardware-video-decoding.force-enabled": { "Value": true"#,
    ] {
        assert!(pol.contains(k), "policies.json lacks {k}");
    }
}
