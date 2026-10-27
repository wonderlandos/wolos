//! HIG §2: surfaces (src/ui, src/bin) use kit tokens only. No colours, no px, no numeric sizes/spacing.
use std::path::Path;

const SIZED_CALLS: [&str; 12] = [
    "hbox(",
    "vbox(",
    "glass_panel(",
    "set_pixel_size(",
    "set_spacing(",
    "set_width_request(",
    "set_height_request(",
    "set_size_request(",
    "set_margin_top(",
    "set_margin_bottom(",
    "set_margin_start(",
    "set_margin_end(",
];
const ICON_CALLS: [&str; 3] = ["icon(", "app_icon(", "avatar("];

fn violations(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let hit = |why: &str| format!("line {}: {why}: {}", n + 1, line.trim());
        if code.contains("rgba(") || code.contains("rgb(") {
            out.push(hit("colour literal"));
        }
        if let Some(i) = code.find('#') {
            let hex: String = code[i + 1..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if [3, 6, 8].contains(&hex.len()) && code[..i].ends_with('"') {
                out.push(hit("hex colour"));
            }
        }
        if code.contains("px\"") || code.contains("px;") {
            out.push(hit("px literal"));
        }
        for c in SIZED_CALLS {
            for (i, _) in code.match_indices(c) {
                let arg = code[i + c.len()..].trim_start();
                if arg.starts_with(|ch: char| ch.is_ascii_digit()) && !arg.starts_with("0)") {
                    out.push(hit(&format!("number in {c}")));
                }
            }
        }
        for c in ICON_CALLS {
            for (i, _) in code.match_indices(c) {
                let args = &code[i + c.len()..];
                let first = args.split(')').next().unwrap_or("");
                if first
                    .rsplit(',')
                    .next()
                    .is_some_and(|a| a.trim().starts_with(|ch: char| ch.is_ascii_digit()))
                    && first.contains(',')
                {
                    out.push(hit(&format!("numeric size in {c}")));
                }
            }
        }
    }
    out
}

#[test]
fn detector_catches_literals() {
    assert_eq!(violations("let b = w::hbox(8);").len(), 1);
    assert_eq!(violations("i.set_pixel_size(48);").len(), 1);
    assert_eq!(violations("let c = \"#ff0000\";").len(), 1);
    assert_eq!(violations("w::icon(\"x\", 24)").len(), 1);
    assert!(
        violations("let b = w::hbox(w::S2); let z = w::vbox(0); w::icon(\"x\", w::ICON)")
            .is_empty()
    );
}

#[test]
fn surfaces_use_tokens_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    // Module directories (ui/settings/, ui/control_center/) included.
    let mut files = Vec::new();
    let mut dirs: Vec<std::path::PathBuf> = ["ui", "bin"].iter().map(|d| root.join(d)).collect();
    while let Some(d) = dirs.pop() {
        for f in std::fs::read_dir(d).unwrap().flatten() {
            if f.path().is_dir() {
                dirs.push(f.path());
            } else {
                files.push(f.path());
            }
        }
    }
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        for v in violations(&src) {
            bad.push(format!("{}: {v}", f.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "literals outside the kit:\n{}",
        bad.join("\n")
    );
}
