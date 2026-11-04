// SPDX-License-Identifier: BSD-2-Clause
//! `src/shaders/ui.wgsl` is a port of the Vulkan renderer's GLSL (`crates/wolkit/shaders/ui.vert`
//! and `ui.frag`): the primitive kinds, the clip flag, the bindings and the vertex inputs must be
//! the same text-for-text numbers, or the two GPU backends would draw different things from the
//! same instances. (Pixel behaviour is held by the goldens in `wgpu_golden.rs`.)

use std::collections::BTreeMap;

fn glsl(name: &str) -> String {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../wolkit/shaders")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn wgsl() -> String {
    wolkit_render::wgpu_backend::SHADER.to_string()
}

/// `const NAME: uint = Nu;` (GLSL) or `const NAME: u32 = Nu;` (WGSL) as name -> number.
fn consts(src: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for l in src.lines().map(str::trim) {
        let Some(rest) = l.strip_prefix("const ") else {
            continue;
        };
        let (name, value) = if let Some(r) = rest.strip_prefix("uint ") {
            r.split_once('=')
        } else {
            rest.split_once(':')
                .and_then(|(n, t)| t.split_once('=').map(|(_, v)| (n, v)))
        }
        .map(|(n, v)| {
            (
                n.trim().to_string(),
                v.trim().trim_end_matches(';').trim_end_matches('u'),
            )
        })
        .unwrap_or_default();
        if let Ok(v) = value.parse::<u32>() {
            out.insert(name, v);
        }
    }
    out
}

#[test]
fn the_wgsl_has_the_same_kinds_and_flags_as_the_glsl() {
    let want = consts(&glsl("ui.frag"));
    assert!(
        want.len() >= 8,
        "the GLSL constants were not found: {want:?}"
    );
    let got = consts(&wgsl());
    for (name, v) in &want {
        assert_eq!(got.get(name), Some(v), "{name}");
    }
}

#[test]
fn the_wgsl_has_the_same_bindings_as_the_glsl() {
    let frag = glsl("ui.frag");
    let w = wgsl();
    for (n, name) in [
        "clips",
        "glyph_pages",
        "shadow_atlas",
        "layer_src",
        "image_pages",
    ]
    .iter()
    .enumerate()
    {
        // The GLSL line that declares `name` carries its binding number.
        let line = frag
            .lines()
            .find(|l| {
                l.contains("binding = ") && l.trim_end().trim_end_matches(';').ends_with(name)
            })
            .unwrap_or_else(|| panic!("no GLSL binding line for {name}"));
        assert!(
            line.contains(&format!("binding = {n}")),
            "{name} is not binding {n} in the GLSL: {line}"
        );
        let decl = w
            .lines()
            .find(|l| {
                l.contains(&format!("var {name}:"))
                    || l.contains(&format!("var<storage, read> {name}:"))
            })
            .unwrap_or_else(|| panic!("no WGSL declaration of {name}"));
        assert!(
            decl.contains(&format!("@group(0) @binding({n})")),
            "{name} is not binding {n} in the WGSL: {decl}"
        );
    }
}

#[test]
fn the_wgsl_has_the_same_vertex_inputs_as_the_glsl() {
    let vert = glsl("ui.vert");
    let w = wgsl();
    for (loc, name) in [
        "rect", "extra", "color0", "color1", "radii", "uv", "clip", "params",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            vert.contains(&format!("location = {loc}) in")) && vert.contains(&format!("i_{name};")),
            "i_{name}"
        );
        assert!(
            w.contains(&format!("@location({loc}) {name}:")),
            "location {loc} is {name} in the WGSL"
        );
    }
}
