//! Liquid-glass policy and parameters (ADR 0018). Pure: config + facts in, plan out.
//! The GLES passes live in the binary (`blur.rs`); this module decides what they do.

use crate::Rect;
use serde::Deserialize;

pub const MAX_PASSES: u32 = 5;

/// `/etc/wolos/glass.json`. Every field optional; missing or bad file = defaults.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GlassConfig {
    pub enabled: bool,
    /// Lower blur cost on integrated GPUs and on battery.
    pub degrade: bool,
    pub blur: Blur,
    pub glass: Glass,
    /// Layer-shell namespaces that get glass. Replaces the defaults when given.
    pub rules: Vec<Rule>,
}

/// Dual-Kawase blur: `passes` down + `passes` up, starting from the output scaled by 1/`downscale`.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Blur {
    pub passes: u32,
    /// Sample spread in texels per pass.
    pub offset: f32,
    pub downscale: u32,
}

/// Liquid-glass look. Lengths in logical px.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Glass {
    /// Max edge displacement (lensing).
    pub refraction: f32,
    /// Width of the lens band along the rim.
    pub edge: f32,
    /// Rim highlight strength, 0..1.
    pub specular: f32,
    /// Red/blue split at the rim, px.
    pub chroma: f32,
    /// RGBA, alpha = tint mix.
    pub tint: [f32; 4],
    /// 1 = unchanged, 2 = max.
    pub saturation: f32,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    /// Exact layer namespace, or a prefix ending in `*`.
    pub namespace: String,
    #[serde(default)]
    pub radius: f32,
    /// Glass shape inside the surface: top, right, bottom, left (e.g. a dock's shadow margin).
    #[serde(default)]
    pub inset: [i32; 4],
}

/// Facts about the machine right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub integrated: bool,
    pub on_battery: bool,
}

/// Flat shader inputs for one glass shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uniforms {
    pub size: [f32; 2],
    /// Shape in blurred-texture UV: x, y, w, h.
    pub uv_rect: [f32; 4],
    pub radius: f32,
    pub edge: f32,
    pub refraction: f32,
    pub specular: f32,
    pub chroma: f32,
    pub tint: [f32; 4],
    pub saturation: f32,
}

impl Default for GlassConfig {
    fn default() -> Self {
        GlassConfig {
            enabled: true,
            degrade: true,
            blur: Blur::default(),
            glass: Glass::default(),
            rules: vec![
                Rule {
                    namespace: "wol-menubar".into(),
                    radius: 0.0,
                    inset: [0; 4],
                },
                // Matches `window.dock > box` margins and radius in shell/Wol.Shell/theme.css.
                Rule {
                    namespace: "wol-dock".into(),
                    radius: 24.0,
                    inset: [28, 32, 10, 32],
                },
                Rule {
                    namespace: "wol-panel*".into(),
                    radius: 18.0,
                    inset: [0; 4],
                },
            ],
        }
    }
}

impl Default for Blur {
    fn default() -> Self {
        Blur {
            passes: 4,
            offset: 3.0,
            downscale: 2,
        }
    }
}

impl Default for Glass {
    fn default() -> Self {
        Glass {
            refraction: 10.0,
            edge: 14.0,
            specular: 0.35,
            chroma: 1.0,
            tint: [1.0, 1.0, 1.0, 0.08],
            saturation: 1.6,
        }
    }
}

impl GlassConfig {
    pub fn parse(json: &str) -> GlassConfig {
        let mut c: GlassConfig = serde_json::from_str(json).unwrap_or_default();
        c.blur.passes = c.blur.passes.clamp(1, MAX_PASSES);
        c.blur.offset = c.blur.offset.clamp(0.0, 16.0);
        c.blur.downscale = c.blur.downscale.clamp(1, 8);
        let g = &mut c.glass;
        g.refraction = g.refraction.clamp(0.0, 64.0);
        g.edge = g.edge.clamp(0.0, 64.0);
        g.specular = g.specular.clamp(0.0, 1.0);
        g.chroma = g.chroma.clamp(0.0, 8.0);
        g.saturation = g.saturation.clamp(0.0, 2.0);
        for t in &mut g.tint {
            *t = t.clamp(0.0, 1.0);
        }
        c
    }

    pub fn rule_for(&self, namespace: &str) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|r| match r.namespace.strip_suffix('*') {
                Some(prefix) => namespace.starts_with(prefix) && namespace != prefix,
                None => r.namespace == namespace,
            })
    }
}

impl Blur {
    /// Fewer passes on an integrated GPU; also a smaller source on battery. Never below one pass.
    pub fn degrade(self, b: Budget) -> Blur {
        let cut = u32::from(b.integrated) + u32::from(b.on_battery);
        Blur {
            passes: self.passes.saturating_sub(cut).max(1),
            downscale: if b.on_battery {
                (self.downscale * 2).min(8)
            } else {
                self.downscale
            },
            ..self
        }
    }

    /// Texture sizes: the downscaled source, then one per down pass (halved, rounded up).
    pub fn levels(&self, w: i32, h: i32) -> Vec<(i32, i32)> {
        let ds = self.downscale.max(1) as i32;
        let mut size = (div_up(w, ds), div_up(h, ds));
        let mut out = vec![size];
        for _ in 0..self.passes {
            size = (div_up(size.0, 2), div_up(size.1, 2));
            out.push(size);
        }
        out
    }
}

fn div_up(v: i32, d: i32) -> i32 {
    ((v + d - 1) / d).max(1)
}

impl Rule {
    pub fn shape(&self, geo: Rect) -> Option<Rect> {
        let [t, r, b, l] = self.inset;
        let out = Rect {
            x: geo.x + l,
            y: geo.y + t,
            w: geo.w - l - r,
            h: geo.h - t - b,
        };
        (out.w > 0 && out.h > 0).then_some(out)
    }
}

impl Glass {
    /// `output`: logical output size; the blurred texture is it divided by `downscale` (rounded up).
    pub fn uniforms(
        &self,
        shape: Rect,
        radius: f32,
        output: (i32, i32),
        downscale: u32,
    ) -> Uniforms {
        let ds = downscale.max(1) as i32;
        let (tw, th) = (div_up(output.0, ds) as f32, div_up(output.1, ds) as f32);
        let d = ds as f32;
        let half_short = shape.w.min(shape.h) as f32 / 2.0;
        Uniforms {
            size: [shape.w as f32, shape.h as f32],
            uv_rect: [
                shape.x as f32 / d / tw,
                shape.y as f32 / d / th,
                shape.w as f32 / d / tw,
                shape.h as f32 / d / th,
            ],
            radius: radius.clamp(0.0, half_short),
            edge: self.edge.min(half_short),
            refraction: self.refraction,
            specular: self.specular,
            chroma: self.chroma,
            tint: self.tint,
            saturation: self.saturation,
        }
    }
}

/// Signed distance to a rounded rect centred at the origin (same formula as the shader).
pub fn sdf_rounded_rect(p: (f32, f32), half: (f32, f32), r: f32) -> f32 {
    let qx = p.0.abs() - half.0 + r;
    let qy = p.1.abs() - half.1 + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Outward unit normal by central differences (same as the shader).
pub fn sdf_normal(p: (f32, f32), half: (f32, f32), r: f32) -> (f32, f32) {
    let f = |x, y| sdf_rounded_rect((x, y), half, r);
    let nx = f(p.0 + 1.0, p.1) - f(p.0 - 1.0, p.1);
    let ny = f(p.0, p.1 + 1.0) - f(p.0, p.1 - 1.0);
    let len = (nx * nx + ny * ny).sqrt().max(1e-6);
    (nx / len, ny / len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_shell_bar_and_dock() {
        let c = GlassConfig::default();
        assert!(c.enabled);
        assert!(c.rule_for("wol-menubar").is_some());
        let dock = c.rule_for("wol-dock").unwrap();
        assert_eq!(dock.radius, 24.0);
        assert!(c.rule_for("firefox").is_none());
    }

    #[test]
    fn namespace_rules_match_exact_or_prefix_star() {
        let c = GlassConfig::parse(r#"{"rules":[{"namespace":"wol-panel-*","radius":12}]}"#);
        assert_eq!(c.rule_for("wol-panel-wifi").unwrap().radius, 12.0);
        assert!(c.rule_for("wol-panel").is_none());
        assert!(
            c.rule_for("wol-dock").is_none(),
            "rules replace the defaults"
        );
    }

    #[test]
    fn bad_json_falls_back_to_defaults_and_values_are_clamped() {
        assert_eq!(GlassConfig::parse("{oops"), GlassConfig::default());
        let c = GlassConfig::parse(
            r#"{"blur":{"passes":40,"offset":-3,"downscale":0},"glass":{"saturation":9}}"#,
        );
        assert_eq!(c.blur.passes, MAX_PASSES);
        assert!(c.blur.offset >= 0.0);
        assert_eq!(c.blur.downscale, 1);
        assert_eq!(c.glass.saturation, 2.0);
    }

    #[test]
    fn power_budget_degrades_blur() {
        let blur = Blur {
            passes: 4,
            offset: 3.0,
            downscale: 2,
        };
        let cases = [
            (Budget::default(), 4, 2),
            (
                Budget {
                    integrated: true,
                    on_battery: false,
                },
                3,
                2,
            ),
            (
                Budget {
                    integrated: true,
                    on_battery: true,
                },
                2,
                4,
            ),
            (
                Budget {
                    integrated: false,
                    on_battery: true,
                },
                3,
                4,
            ),
        ];
        for (budget, passes, downscale) in cases {
            let b = blur.degrade(budget);
            assert_eq!((b.passes, b.downscale), (passes, downscale), "{budget:?}");
        }
        let one = Blur { passes: 1, ..blur };
        assert_eq!(
            one.degrade(Budget {
                integrated: true,
                on_battery: true
            })
            .passes,
            1
        );
    }

    #[test]
    fn kawase_levels_halve_and_never_reach_zero() {
        let b = Blur {
            passes: 3,
            offset: 3.0,
            downscale: 2,
        };
        assert_eq!(
            b.levels(1920, 1080),
            vec![(960, 540), (480, 270), (240, 135), (120, 68)]
        );
        assert_eq!(b.levels(4, 2).last(), Some(&(1, 1)));
    }

    #[test]
    fn glass_rect_applies_insets_and_drops_empty() {
        let rule = Rule {
            namespace: "wol-dock".into(),
            radius: 24.0,
            inset: [28, 32, 10, 32],
        };
        let geo = Rect {
            x: 0,
            y: 600,
            w: 400,
            h: 120,
        };
        assert_eq!(
            rule.shape(geo),
            Some(Rect {
                x: 32,
                y: 628,
                w: 336,
                h: 82
            })
        );
        assert_eq!(
            rule.shape(Rect {
                x: 0,
                y: 0,
                w: 60,
                h: 30
            }),
            None
        );
    }

    #[test]
    fn uniforms_clamp_radius_and_map_texture_space() {
        let p = Glass::default();
        let r = Rect {
            x: 100,
            y: 0,
            w: 200,
            h: 20,
        };
        let u = p.uniforms(r, 50.0, (1000, 500), 2);
        assert_eq!(u.size, [200.0, 20.0]);
        assert_eq!(u.radius, 10.0, "at most half the short side");
        // Texture is the output downscaled by 2: rect (100,0,200,20) -> uv over a 500x250 texture.
        assert_eq!(u.uv_rect, [0.1, 0.0, 0.2, 0.04]);
        assert!(u.edge <= 10.0, "lens band fits inside the shape");
    }

    #[test]
    fn sdf_is_negative_inside_zero_on_edge_and_normal_points_out() {
        let half = (50.0, 20.0);
        assert!(sdf_rounded_rect((0.0, 0.0), half, 8.0) < 0.0);
        assert!(sdf_rounded_rect((50.0, 0.0), half, 8.0).abs() < 1e-4);
        assert!(sdf_rounded_rect((60.0, 0.0), half, 8.0) > 0.0);
        // Exact corner of the box lies outside a rounded corner.
        assert!(sdf_rounded_rect((50.0, 20.0), half, 8.0) > 0.0);
        let (nx, ny) = sdf_normal((49.0, 0.0), half, 8.0);
        assert!(nx > 0.99 && ny.abs() < 1e-3);
        let (nx, ny) = sdf_normal((0.0, -19.0), half, 8.0);
        assert!(ny < -0.99 && nx.abs() < 1e-3);
    }
}
