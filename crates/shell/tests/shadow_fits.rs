//! Glass shadows must fit inside the surface: GTK cannot draw outside a layer surface, so a shadow wider
//! than the transparent margin around a panel ends in a hard edge (FreeBSD sway shots: a band 24 px under
//! Control Center, the switcher and banners). Margins around every glass card cover the shadow extent.
const CSS: &str = include_str!("../src/kit/kit.css");

fn px(s: &str) -> f64 {
    s.trim().trim_end_matches("px").parse().unwrap_or(0.0)
}

/// Body of the first rule whose selector list contains `sel` exactly.
fn rule(sel: &str) -> String {
    let mut rest = CSS;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}').unwrap();
        let head = rest[..open].rsplit(['}', '/']).next().unwrap_or("");
        if head.split(',').any(|h| h.trim() == sel) {
            return rest[open + 1..close].to_string();
        }
        rest = &rest[close + 1..];
    }
    panic!("no rule {sel}");
}

fn prop(body: &str, name: &str) -> String {
    body.split(';')
        .find_map(|d| {
            d.trim()
                .strip_prefix(&format!("{name}:"))
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_else(|| panic!("no {name} in {body}"))
}

/// CSS margin shorthand -> [top, right, bottom, left].
fn sides(v: &str) -> [f64; 4] {
    let n: Vec<f64> = v.split_whitespace().map(px).collect();
    match n.len() {
        1 => [n[0]; 4],
        2 => [n[0], n[1], n[0], n[1]],
        3 => [n[0], n[1], n[2], n[1]],
        _ => [n[0], n[1], n[2], n[3]],
    }
}

/// Extent of `--glass-shadow` past the box: [top, right, bottom, left].
fn shadow_extent() -> [f64; 4] {
    let v = prop(&rule(":root"), "--glass-shadow");
    let mut e = [0f64; 4];
    for layer in v.split("),") {
        let n: Vec<f64> = layer.split_whitespace().take(3).map(px).collect();
        let (x, y, blur) = (n[0], n[1], n[2]);
        for (i, d) in [blur - y, blur + x, blur + y, blur - x]
            .into_iter()
            .enumerate()
        {
            e[i] = e[i].max(d);
        }
    }
    e
}

#[test]
fn glass_shadows_fit_in_the_surface_margin() {
    let e = shadow_extent();
    let add = |a: [f64; 4], b: [f64; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    // Banners: the stack's padding is the surface edge room; groups add their own margin.
    let banners = add(
        sides(&prop(&rule(".banner-stack"), "padding")),
        sides(&prop(&rule("window.banners .banner-group"), "margin")),
    );
    for (what, m) in [
        (
            ".glass-panel",
            sides(&prop(&rule(".glass-panel"), "margin")),
        ),
        ("banner stack", banners),
    ] {
        for i in 0..4 {
            assert!(m[i] >= e[i], "{what}: room {m:?} < shadow extent {e:?}");
        }
    }
}
