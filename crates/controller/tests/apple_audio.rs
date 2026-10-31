//! AirPods and other Bluetooth headphones: the AAC, LDAC and aptX codecs come from pipewire-audio.

fn shipped() -> Vec<String> {
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    let mut v = Vec::new();
    for f in ["image/arch/packages.x86_64", "desktop/packages.txt"] {
        for l in std::fs::read_to_string(format!("{root}/{f}")).unwrap().lines() {
            let l = l.split('#').next().unwrap().trim();
            if !l.is_empty() {
                v.push(l.to_owned());
            }
        }
    }
    v
}

#[test]
fn bluetooth_audio_codecs_ship_offline() {
    let pkgs = shipped();
    for p in ["pipewire-audio", "bluez", "bluez-utils"] {
        assert!(pkgs.iter().any(|x| x == p), "{p} ships offline");
    }
}
