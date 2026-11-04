// SPDX-License-Identifier: BSD-2-Clause
//! Design tokens as the style system sees them: the 13 theme colours, the spacing, radius, type
//! and motion scales, the shadow and spring materials. Values mirror `shared::theme` (colours in
//! `TOKENS`, the scale in `SCALE`, the metrics in `scaffold`); `tests/style_tokens.rs` fails the
//! build when either side drifts.
use super::num::f32_to_u16;
use crate::scene::ids::{Ix, to_u8};

/// The 13 semantic colours of a theme, in `shared::theme::TOKENS` order (ids are append-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Token {
    Surface,
    Sidebar,
    Card,
    Line,
    Text,
    Dim,
    Accent,
    AccentText,
    Glass,
    Control,
    Success,
    Warning,
    Danger,
}

impl Token {
    pub const ALL: [Token; 13] = [
        Token::Surface,
        Token::Sidebar,
        Token::Card,
        Token::Line,
        Token::Text,
        Token::Dim,
        Token::Accent,
        Token::AccentText,
        Token::Glass,
        Token::Control,
        Token::Success,
        Token::Warning,
        Token::Danger,
    ];
    pub const NAMES: [&'static str; 13] = [
        "surface",
        "sidebar",
        "card",
        "line",
        "text",
        "dim",
        "accent",
        "accentText",
        "glass",
        "control",
        "success",
        "warning",
        "danger",
    ];
    pub fn name(self) -> &'static str {
        Self::NAMES[(self as u64).ix()]
    }
    pub fn parse(s: &str) -> Option<Token> {
        Self::NAMES
            .iter()
            .position(|n| *n == s)
            .map(|i| Self::ALL[i])
    }
    pub fn to_u8(self) -> u8 {
        to_u8(self as u64)
    }
    pub fn from_u8(b: u8) -> Option<Token> {
        Self::ALL.get(b.ix()).copied()
    }
}

/// One entry of a named scale: the name, the pixel (or millisecond) value and the `shared::theme`
/// variable it mirrors (`None` for a name that is only an alias of 0).
pub struct Step {
    pub name: &'static str,
    pub value: f32,
    pub var: Option<&'static str>,
}

/// Spacing: padding, margin, gap, inset. `--space-1` to `--space-6` (there is no `--space-5`).
pub const SPACE: [Step; 6] = [
    Step {
        name: "none",
        value: 0.0,
        var: None,
    },
    Step {
        name: "xs",
        value: 4.0,
        var: Some("--space-1"),
    },
    Step {
        name: "sm",
        value: 8.0,
        var: Some("--space-2"),
    },
    Step {
        name: "md",
        value: 12.0,
        var: Some("--space-3"),
    },
    Step {
        name: "lg",
        value: 16.0,
        var: Some("--space-4"),
    },
    Step {
        name: "xl",
        value: 24.0,
        var: Some("--space-6"),
    },
];

/// Corner radius. `full` is a pill: the shorter side halved.
pub const RADIUS: [Step; 6] = [
    Step {
        name: "none",
        value: 0.0,
        var: None,
    },
    Step {
        name: "s",
        value: 8.0,
        var: Some("--radius-s"),
    },
    Step {
        name: "m",
        value: 12.0,
        var: Some("--radius-m"),
    },
    Step {
        name: "l",
        value: 16.0,
        var: Some("--radius-l"),
    },
    Step {
        name: "xl",
        value: 24.0,
        var: Some("--radius-xl"),
    },
    Step {
        name: "full",
        value: RADIUS_FULL,
        var: None,
    },
];
/// Stored radius meaning "pill" (clamped to half the box at paint).
pub const RADIUS_FULL: f32 = 9999.0;

/// Motion durations in milliseconds (`--motion-*`).
pub const MOTION: [Step; 3] = [
    Step {
        name: "fast",
        value: 150.0,
        var: Some("--motion-fast"),
    },
    Step {
        name: "normal",
        value: 250.0,
        var: Some("--motion-normal"),
    },
    Step {
        name: "slow",
        value: 400.0,
        var: Some("--motion-slow"),
    },
];

/// Type scale roles (`shared::view::Role`): name, size px, weight.
pub struct RoleSpec {
    pub name: &'static str,
    pub size: f32,
    pub weight: u16,
    pub line_height: f32,
}

pub const ROLES: [RoleSpec; 6] = [
    RoleSpec {
        name: "display",
        size: 34.0,
        weight: 700,
        line_height: 1.15,
    },
    RoleSpec {
        name: "large",
        size: 27.0,
        weight: 600,
        line_height: 1.2,
    },
    RoleSpec {
        name: "title",
        size: 17.0,
        weight: 600,
        line_height: 1.3,
    },
    RoleSpec {
        name: "body",
        size: 13.0,
        weight: 400,
        line_height: 1.4,
    },
    RoleSpec {
        name: "secondary",
        size: 11.0,
        weight: 400,
        line_height: 1.4,
    },
    RoleSpec {
        name: "caption",
        size: 11.0,
        weight: 400,
        line_height: 1.3,
    },
];

/// Smallest font size the compiler accepts (RU-059).
pub const MIN_FONT_PX: f32 = 11.0;

pub fn space_value(name: &str) -> Option<f32> {
    SPACE.iter().find(|s| s.name == name).map(|s| s.value)
}
pub fn radius_value(name: &str) -> Option<f32> {
    RADIUS.iter().find(|s| s.name == name).map(|s| s.value)
}
pub fn motion_ms(name: &str) -> Option<u16> {
    MOTION
        .iter()
        .find(|s| s.name == name)
        .map(|s| f32_to_u16(s.value))
}
pub fn role(name: &str) -> Option<&'static RoleSpec> {
    ROLES.iter().find(|r| r.name == name)
}
pub fn on_space_scale(px: f32) -> bool {
    SPACE.iter().any(|s| s.value == px)
}
pub fn on_radius_scale(px: f32) -> bool {
    RADIUS.iter().any(|s| s.value == px)
}

/// Elevation: fixed material, not themable (`docs/RULES-ui.md`). One light upper left: a shadow falls right and down at
/// 1:2, `sm`/`md`/`lg` are the Kit's `--elev-1/2/3`. Black at `alpha` percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum ShadowTok {
    #[default]
    None,
    Sm,
    Md,
    Lg,
}

/// One shadow layer: offsets x and y (1:2), blur radius, black alpha in percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowParams {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub alpha: u8,
}

impl ShadowTok {
    pub const ALL: [ShadowTok; 4] = [ShadowTok::None, ShadowTok::Sm, ShadowTok::Md, ShadowTok::Lg];
    pub fn name(self) -> &'static str {
        ["none", "sm", "md", "lg"][(self as u64).ix()]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.name() == s)
    }
    pub fn to_u8(self) -> u8 {
        to_u8(self as u64)
    }
    pub fn from_u8(b: u8) -> Option<Self> {
        Self::ALL.get(b.ix()).copied()
    }
    /// Layers, mirroring `--elev-1`, `--elev-2` and `--elev-3` of the Kit.
    pub fn layers(self) -> &'static [ShadowParams] {
        const SM: [ShadowParams; 1] = [ShadowParams {
            dx: 1.0,
            dy: 2.0,
            blur: 4.0,
            alpha: 14,
        }];
        const MD: [ShadowParams; 1] = [ShadowParams {
            dx: 2.0,
            dy: 4.0,
            blur: 12.0,
            alpha: 18,
        }];
        const LG: [ShadowParams; 1] = [ShadowParams {
            dx: 3.0,
            dy: 6.0,
            blur: 24.0,
            alpha: 24,
        }];
        match self {
            ShadowTok::None => &[],
            ShadowTok::Sm => &SM,
            ShadowTok::Md => &MD,
            ShadowTok::Lg => &LG,
        }
    }
}

/// Spring material for `spring:`: stiffness, damping (per second), mass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

pub fn spring_params(s: super::value::Spring) -> Option<SpringParams> {
    use super::value::Spring;
    match s {
        Spring::None => None,
        Spring::Gentle => Some(SpringParams {
            stiffness: 170.0,
            damping: 26.0,
            mass: 1.0,
        }),
        Spring::Snappy => Some(SpringParams {
            stiffness: 320.0,
            damping: 30.0,
            mass: 1.0,
        }),
        Spring::Bouncy => Some(SpringParams {
            stiffness: 260.0,
            damping: 14.0,
            mass: 1.0,
        }),
    }
}

/// What `@contrast` does to a token: it is replaced by `from` at `pct` percent alpha. Mirrors
/// `crates/shell/src/kit/contrast.css` (`--dim: var(--text)`, `--line: color-mix(text 55%)`,
/// `.glass-panel { background: var(--surface) }`).
pub struct ContrastOverride {
    pub token: Token,
    pub from: Token,
    pub pct: u8,
}

pub const CONTRAST: [ContrastOverride; 3] = [
    ContrastOverride {
        token: Token::Dim,
        from: Token::Text,
        pct: 100,
    },
    ContrastOverride {
        token: Token::Line,
        from: Token::Text,
        pct: 55,
    },
    ContrastOverride {
        token: Token::Glass,
        from: Token::Surface,
        pct: 100,
    },
];

/// Resolve a colour reference under the increase-contrast preference.
pub fn contrast_remap(c: super::value::ColorRef) -> super::value::ColorRef {
    use super::value::ColorRef;
    let (t, a) = match c {
        ColorRef::Token(t) => (t, 100u32),
        ColorRef::Alpha(t, a) => (t, u32::from(a)),
        ColorRef::Transparent | ColorRef::Shade(_) => return c,
    };
    match CONTRAST.iter().find(|o| o.token == t) {
        Some(o) => {
            let pct = to_u8(u32::from(o.pct) * a / 100);
            if pct == 100 {
                ColorRef::Token(o.from)
            } else {
                ColorRef::Alpha(o.from, pct)
            }
        }
        None => c,
    }
}

/// Closest names for a "did you mean" suggestion (edit distance at most 2, best first).
pub fn suggest<'a>(word: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for n in names {
        let d = edit_distance(&word.to_ascii_lowercase(), &n.to_ascii_lowercase());
        if d <= 2 && d < word.len().max(1) && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, n));
        }
    }
    best.map(|(_, n)| n)
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}
