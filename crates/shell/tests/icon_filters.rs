//! No shipped icon uses an SVG `<filter>`: GTK, Qt, Firefox and the pixbuf loader clip the
//! filter region and draw a grey box behind the icon. Depth is baked as plain shapes.

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "svg") {
            out.push(p);
        }
    }
}

#[test]
fn no_icon_uses_an_svg_filter() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/icons"));
    let mut svgs = Vec::new();
    walk(root, &mut svgs);
    assert!(svgs.len() > 100, "found only {} icons", svgs.len());
    let bad: Vec<_> = svgs
        .iter()
        .filter(|p| {
            let s = std::fs::read_to_string(p).unwrap();
            s.contains("<filter") || s.contains("filter:url") || s.contains("filter=\"url")
        })
        .map(|p| p.strip_prefix(root).unwrap().display().to_string())
        .collect();
    assert!(bad.is_empty(), "{} icons use <filter>: {bad:?}", bad.len());
}
