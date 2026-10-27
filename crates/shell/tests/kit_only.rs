//! HIG §2: assistant surfaces (Setup, installer) are built from Kit components only; no GTK widget
//! is constructed in them directly, so every page shares the Kit's sizes, insets and states.
use std::path::Path;

const FILES: [&str; 2] = ["src/ui/setup.rs", "src/ui/installer.rs"];
const CONSTRUCTORS: [&str; 6] = [
    "::new(",
    "::builder(",
    "::with_label(",
    "::for_filename(",
    "::from_strings(",
    "::with_mnemonic(",
];

fn raw_widgets(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        for (i, _) in code.match_indices("gtk::") {
            let rest = &code[i + 5..];
            let ty: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let starts_upper = ty.starts_with(|c: char| c.is_ascii_uppercase());
            if starts_upper && CONSTRUCTORS.iter().any(|c| rest[ty.len()..].starts_with(c)) {
                out.push(format!("line {}: {}", n + 1, line.trim()));
            }
        }
    }
    out
}

#[test]
fn detector_catches_raw_widgets() {
    assert_eq!(
        raw_widgets("let b = gtk::Button::with_label(\"x\");").len(),
        1
    );
    assert_eq!(
        raw_widgets("let s = gtk::Stack::builder().build();").len(),
        1
    );
    assert!(raw_widgets("let b = w::button(\"x\", w::Kind::Primary, || {});").is_empty());
    assert!(raw_widgets("s.set_transition_type(gtk::StackTransitionType::None);").is_empty());
    assert!(raw_widgets("// gtk::Box::new( in a comment").is_empty());
}

#[test]
fn assistants_use_kit_components_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    for f in FILES {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        bad.extend(raw_widgets(&src).into_iter().map(|v| format!("{f}: {v}")));
    }
    assert!(
        bad.is_empty(),
        "raw GTK widgets outside the Kit:\n{}",
        bad.join("\n")
    );
}

/// System Settings panes too: every row, card and control comes from the Kit. Only the app and its
/// window (`settings::run`) are made directly.
#[test]
fn settings_panes_use_kit_components_only() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/settings");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let src = std::fs::read_to_string(e.path()).unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        bad.extend(
            raw_widgets(&src)
                .into_iter()
                .filter(|v| !v.contains("gtk::Application"))
                .map(|v| format!("settings/{name}: {v}")),
        );
    }
    assert!(
        bad.is_empty(),
        "raw GTK widgets in Settings panes:\n{}",
        bad.join("\n")
    );
}
