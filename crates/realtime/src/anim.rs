// SPDX-License-Identifier: GPL-2.0-or-later
//! Window animation curves. Pure: time in, visual state out.
//!
//! The motion table of the compositor: the shell's named curves (`kit::motion::Curve`, HIG section
//! 7), its three durations and its two window springs, and [`LEAVES`], which compositor animation
//! runs on which of them. [`lua`] renders that as the animations block of the shipped
//! `hyprland.lua` (a test holds the file to it), so a window, a layer and a shell panel move alike.

/// How a window looks at one instant of an animation. Applied by the renderer on top of its layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub scale: f32,
    pub opacity: f32,
    /// Minimize: the point the window shrinks toward (dock icon), blended in by progress.
    pub offset_to: Option<(f32, f32)>,
}

impl Visual {
    pub const REST: Visual = Visual {
        scale: 1.0,
        opacity: 1.0,
        offset_to: None,
    };
}

/// CSS-style cubic-bezier easing through (0,0), (x1,y1), (x2,y2), (1,1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier(pub f32, pub f32, pub f32, pub f32);

impl Bezier {
    /// Fast start, soft landing (the WolOS default): [`Curve::Ease`].
    pub const EASE_OUT: Bezier = Bezier(0.2, 0.8, 0.2, 1.0);
    /// Leaving: [`Curve::In`].
    pub const EASE_IN: Bezier = Bezier(0.4, 0.0, 1.0, 1.0);

    /// Progress (0..1, beyond for an overshooting curve) at time fraction `x` (0..1).
    pub fn ease(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let curve = |a: f32, b: f32, t: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // Find the curve parameter whose x matches, by bisection (x(t) is monotonic for 0<=x1,x2<=1).
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(self.0, self.2, mid) < x {
                lo = mid
            } else {
                hi = mid
            }
        }
        curve(self.1, self.3, (lo + hi) / 2.0)
    }
}

/// A named easing curve, the shell's `kit::motion::Curve` (names and control points verbatim).
/// The first five are the theme contract; `glass` and `overshoot` overshoot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Curve {
    /// `ease`: entering and moving, fast start, soft landing.
    Ease,
    /// `linear`: value-driven fades, never movement (the compositor's built-in `linear`).
    Linear,
    /// `in`: leaving.
    In,
    /// `out`: a gentler entrance for large surfaces.
    Out,
    /// `in-out`: a surface that moves and stays on screen.
    InOut,
    /// `glass`: a strong overshoot (peaks near 1.1), the Pop window style.
    Glass,
    /// `overshoot`: a light overshoot, window moves.
    Overshoot,
    /// `soft`: fades and workspace slides.
    Soft,
}

impl Curve {
    /// The theme contract: monotonic, no overshoot.
    pub const CONTRACT: [Curve; 5] = [
        Curve::Ease,
        Curve::Linear,
        Curve::In,
        Curve::Out,
        Curve::InOut,
    ];

    /// Every named curve: the contract, then `glass`, `overshoot`, `soft`.
    pub const ALL: [Curve; 8] = [
        Curve::Ease,
        Curve::Linear,
        Curve::In,
        Curve::Out,
        Curve::InOut,
        Curve::Glass,
        Curve::Overshoot,
        Curve::Soft,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Curve::Ease => "ease",
            Curve::Linear => "linear",
            Curve::In => "in",
            Curve::Out => "out",
            Curve::InOut => "in-out",
            Curve::Glass => "glass",
            Curve::Overshoot => "overshoot",
            Curve::Soft => "soft",
        }
    }

    pub fn from_name(name: &str) -> Option<Curve> {
        Curve::ALL.into_iter().find(|c| c.name() == name)
    }

    /// `cubic-bezier(x1, y1, x2, y2)`; `linear` is (0, 0, 1, 1).
    pub fn bezier(self) -> Bezier {
        match self {
            Curve::Ease => Bezier::EASE_OUT,
            Curve::Linear => Bezier(0.0, 0.0, 1.0, 1.0),
            Curve::In => Bezier::EASE_IN,
            Curve::Out => Bezier(0.0, 0.0, 0.2, 1.0),
            Curve::InOut => Bezier(0.4, 0.0, 0.2, 1.0),
            Curve::Glass => Bezier(0.34, 1.56, 0.64, 1.0),
            Curve::Overshoot => Bezier(0.22, 1.2, 0.36, 1.0),
            Curve::Soft => Bezier(0.25, 0.1, 0.25, 1.0),
        }
    }

    /// Progress at time fraction `t` (clamped to 0..1).
    pub fn at(self, t: f32) -> f32 {
        match self {
            Curve::Linear => t.clamp(0.0, 1.0),
            c => c.bezier().ease(t),
        }
    }
}

/// The three durations of the theme contract (`--motion-fast`, `-normal`, `-slow`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Duration {
    /// 150 ms: anything leaving.
    Fast,
    /// 250 ms: panels, layers, window moves, fades in.
    Normal,
    /// 400 ms: large surfaces (workspaces).
    Slow,
}

/// [`Duration`] in milliseconds, the shell's `kit::motion::DURATIONS_MS`.
pub const DURATIONS_MS: [u64; 3] = [150, 250, 400];

impl Duration {
    pub const ALL: [Duration; 3] = [Duration::Fast, Duration::Normal, Duration::Slow];

    pub fn ms(self) -> u64 {
        let i = match self {
            Duration::Fast => 0,
            Duration::Normal => 1,
            Duration::Slow => 2,
        };
        DURATIONS_MS[i]
    }

    pub fn name(self) -> &'static str {
        match self {
            Duration::Fast => "fast",
            Duration::Normal => "normal",
            Duration::Slow => "slow",
        }
    }
}

/// A damped spring (target 1 from rest at 0), solved in closed form like the compositor does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub name: &'static str,
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

impl Spring {
    /// Opening a window: the shell's `kit::motion::window::OPEN` (HIG `window-open`).
    pub const WINDOW_OPEN: Spring = Spring {
        name: "window-open",
        stiffness: 385.5,
        damping: 31.4,
        mass: 1.0,
    };
    /// Closing a window: `kit::motion::window::CLOSE` (HIG `window-close`).
    pub const WINDOW_CLOSE: Spring = Spring {
        name: "window-close",
        stiffness: 322.3,
        damping: 30.5,
        mass: 1.0,
    };
    pub const ALL: [Spring; 2] = [Spring::WINDOW_OPEN, Spring::WINDOW_CLOSE];

    pub fn named(name: &str) -> Option<Spring> {
        Spring::ALL.into_iter().find(|s| s.name == name)
    }

    pub fn damping_ratio(self) -> f64 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }

    /// Position and velocity `secs` after leaving 0 at rest toward 1.
    pub fn sample(self, secs: f64) -> (f64, f64) {
        let t = secs.max(0.0);
        let w0 = (self.stiffness / self.mass).sqrt();
        let g = self.damping / (2.0 * self.mass);
        let (x0, v0) = (-1.0, 0.0); // displacement from the target, velocity
        let (d, v) = if g < w0 {
            let wd = (w0 * w0 - g * g).sqrt();
            let (e, s, c) = ((-g * t).exp(), (wd * t).sin(), (wd * t).cos());
            (
                e * (x0 * c + (v0 + g * x0) / wd * s),
                e * (v0 * c - (g * v0 + w0 * w0 * x0) / wd * s),
            )
        } else {
            // Critically damped or slower: the windows' springs never are, kept for completeness.
            let e = (-g * t).exp();
            let b = v0 + g * x0;
            (e * (x0 + b * t), e * (v0 - g * b * t))
        };
        (1.0 + d, v)
    }

    /// Milliseconds after which the move stays within 2% of the target: what the eye sees as the
    /// duration (the HIG settle column; the compositor stops later, at 0.1%).
    pub fn settle_ms(self) -> u64 {
        (0..=5_000u64)
            .rev()
            .find(|&ms| (self.sample(ms as f64 / 1000.0).0 - 1.0).abs() > 0.02)
            .map_or(0, |ms| ms + 1)
    }

    /// Largest value reached (1 + overshoot).
    pub fn peak(self) -> f64 {
        (0..=1_000u64)
            .map(|ms| self.sample(ms as f64 / 1000.0).0)
            .fold(0.0, f64::max)
    }
}

/// What a compositor animation runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timing {
    Curve(Curve, Duration),
    Spring(Spring),
}

impl Timing {
    /// The visible length in ms (a spring's settle time).
    pub fn ms(self) -> u64 {
        match self {
            Timing::Curve(_, d) => d.ms(),
            Timing::Spring(s) => s.settle_ms(),
        }
    }

    /// The compositor's `speed` (deciseconds, one decimal). A spring ignores it; it carries the
    /// settle time so the line still reads as a duration.
    pub fn speed(self) -> String {
        format!("{:.1}", (self.ms() as f64 / 100.0).max(0.1))
    }
}

/// One compositor animation leaf and its timing. Parents come before their children (a parent
/// sets every child the config has not set).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaf {
    pub leaf: &'static str,
    pub timing: Timing,
    pub style: Option<&'static str>,
}

const fn curve(leaf: &'static str, c: Curve, d: Duration, style: Option<&'static str>) -> Leaf {
    Leaf {
        leaf,
        timing: Timing::Curve(c, d),
        style,
    }
}

/// The compositor's animations: enter with `ease` at normal, leave with `in` one step faster,
/// window open and close on the window springs, large surfaces slow (HIG section 7).
pub const LEAVES: [Leaf; 12] = [
    Leaf {
        leaf: "windowsIn",
        timing: Timing::Spring(Spring::WINDOW_OPEN),
        style: Some("popin 90%"),
    },
    Leaf {
        leaf: "windowsOut",
        timing: Timing::Spring(Spring::WINDOW_CLOSE),
        style: Some("popin 90%"),
    },
    curve("windowsMove", Curve::Overshoot, Duration::Normal, None),
    curve("fade", Curve::Soft, Duration::Normal, None),
    curve("fadeIn", Curve::Ease, Duration::Normal, None),
    curve("fadeOut", Curve::In, Duration::Fast, None),
    curve("layers", Curve::Ease, Duration::Normal, Some("fade")),
    curve("layersOut", Curve::In, Duration::Fast, Some("fade")),
    curve("fadeLayersIn", Curve::Ease, Duration::Normal, None),
    curve("fadeLayersOut", Curve::In, Duration::Fast, None),
    curve("workspaces", Curve::Soft, Duration::Slow, Some("slide")),
    curve(
        "specialWorkspace",
        Curve::Soft,
        Duration::Slow,
        Some("slidevert"),
    ),
];

/// The leaf named `leaf` in [`LEAVES`].
pub fn leaf(leaf: &str) -> Option<Leaf> {
    LEAVES.into_iter().find(|l| l.leaf == leaf)
}

/// Effects & Looks "Motion" (`wol_core::looks::Motion`, by its serde name): how a window opens.
/// Zoom and Slide open on the window spring; Pop springs past full size on `glass`.
pub fn window_in(motion: &str) -> Option<Leaf> {
    let open = Timing::Spring(Spring::WINDOW_OPEN);
    let (timing, style) = match motion {
        "zoom" => (open, "popin 90%"),
        "pop" => (Timing::Curve(Curve::Glass, Duration::Normal), "popin 80%"),
        "slide" => (open, "slide"),
        _ => return None,
    };
    Some(Leaf {
        leaf: "windowsIn",
        timing,
        style: Some(style),
    })
}

fn lua_num(v: f64) -> String {
    format!("{}", (v * 1000.0).round() / 1000.0)
}

impl Leaf {
    /// The `hl.animation` call.
    pub fn lua(&self) -> String {
        let curve = match self.timing {
            Timing::Curve(c, _) => format!("bezier = \"{}\"", c.name()),
            Timing::Spring(s) => format!("spring = \"{}\"", s.name),
        };
        let style = self
            .style
            .map_or(String::new(), |s| format!(", style = \"{s}\""));
        format!(
            "hl.animation({{ leaf = \"{}\", enabled = true, speed = {}, {curve}{style} }})",
            self.leaf,
            self.timing.speed()
        )
    }
}

/// The animations block of the compositor's Lua config: switch, curves (every named curve but the
/// built-in `linear`), springs, then [`LEAVES`]. One line each, newline-terminated.
pub fn lua() -> String {
    let mut s = String::from("hl.config({ animations = { enabled = true } })\n");
    s += "-- Generated from crates/realtime/src/anim.rs (the shell's curves and durations, HIG section 7).\n";
    s += "-- Speed in deciseconds; a spring ignores it (the value is its settle time).\n";
    for c in Curve::ALL.into_iter().filter(|c| *c != Curve::Linear) {
        let Bezier(a, b, x, y) = c.bezier();
        let n = |v: f32| lua_num(v.into());
        s += &format!(
            "hl.curve(\"{}\", {{ type = \"bezier\", points = {{ {{ {}, {} }}, {{ {}, {} }} }} }})\n",
            c.name(),
            n(a),
            n(b),
            n(x),
            n(y)
        );
    }
    for sp in Spring::ALL {
        s += &format!(
            "hl.curve(\"{}\", {{ type = \"spring\", stiffness = {}, damping = {}, mass = {} }})\n",
            sp.name,
            lua_num(sp.stiffness),
            lua_num(sp.damping),
            lua_num(sp.mass)
        );
    }
    for l in LEAVES {
        s += &l.lua();
        s.push('\n');
    }
    s
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Open,
    Close,
    Minimize((f32, f32)),
}

/// One running window animation, started at `start_ms` on the compositor clock: open and close
/// are the compositor's `windowsIn`/`windowsOut` (`popin 90%` on the window springs, with the
/// `fadeIn`/`fadeOut` alpha), minimize flies to the Dock on `ease` (slow).
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    kind: Kind,
    start_ms: u64,
    pub duration_ms: u64,
}

/// `popin 90%`: the scale a window opens from and closes to.
pub const POPIN_SCALE: f32 = 0.9;

impl Animation {
    pub fn open(start_ms: u64) -> Self {
        Self {
            kind: Kind::Open,
            start_ms,
            duration_ms: Spring::WINDOW_OPEN.settle_ms().max(Duration::Normal.ms()),
        }
    }
    pub fn close(start_ms: u64) -> Self {
        Self {
            kind: Kind::Close,
            start_ms,
            duration_ms: Spring::WINDOW_CLOSE.settle_ms().max(Duration::Fast.ms()),
        }
    }
    pub fn minimize(start_ms: u64, dock: (f32, f32)) -> Self {
        Self {
            kind: Kind::Minimize(dock),
            start_ms,
            duration_ms: Duration::Slow.ms(),
        }
    }

    pub fn done(&self, now_ms: u64) -> bool {
        now_ms >= self.start_ms + self.duration_ms
    }

    pub fn sample(&self, now_ms: u64) -> Visual {
        if self.done(now_ms) {
            return match self.kind {
                Kind::Open => Visual::REST,
                Kind::Close => Visual {
                    scale: POPIN_SCALE,
                    opacity: 0.0,
                    offset_to: None,
                },
                Kind::Minimize(dock) => Visual {
                    scale: 0.1,
                    opacity: 0.6,
                    offset_to: Some(dock),
                },
            };
        }
        let ms = now_ms.saturating_sub(self.start_ms);
        let secs = ms as f64 / 1000.0;
        let frac = |d: Duration| ms as f32 / d.ms() as f32;
        let lerp = |a: f32, b: f32, p: f32| a + (b - a) * p;
        match self.kind {
            Kind::Open => {
                let p = Spring::WINDOW_OPEN.sample(secs).0 as f32;
                Visual {
                    scale: lerp(POPIN_SCALE, 1.0, p),
                    opacity: Curve::Ease.at(frac(Duration::Normal)),
                    offset_to: None,
                }
            }
            Kind::Close => {
                let p = Spring::WINDOW_CLOSE.sample(secs).0 as f32;
                Visual {
                    scale: lerp(1.0, POPIN_SCALE, p),
                    opacity: 1.0 - Curve::In.at(frac(Duration::Fast)),
                    offset_to: None,
                }
            }
            Kind::Minimize(dock) => {
                let p = Curve::Ease.at(frac(Duration::Slow));
                Visual {
                    scale: lerp(1.0, 0.1, p),
                    opacity: lerp(1.0, 0.6, p),
                    offset_to: Some(dock),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_starts_small_and_clear_and_ends_at_rest() {
        let a = Animation::open(0);
        let start = a.sample(0);
        assert!(start.scale < 0.95 && start.opacity < 0.05);
        assert_eq!(a.sample(a.duration_ms), Visual::REST);
        assert!(a.done(a.duration_ms));
    }

    #[test]
    fn open_settles_with_at_most_a_hint_of_overshoot() {
        let a = Animation::open(0);
        let mut last = a.sample(0);
        for t in (0..=a.duration_ms).step_by(4) {
            let v = a.sample(t);
            assert!(v.opacity >= last.opacity - 1e-6, "t={t}");
            // window-open is lightly underdamped: past full size by under 0.5% of a pixel scale.
            assert!(v.scale <= 1.0 + 0.005, "t={t}: {}", v.scale);
            last = v;
        }
    }

    #[test]
    fn close_runs_the_other_way_and_ends_invisible() {
        let a = Animation::close(100);
        assert_eq!(a.sample(100), Visual::REST);
        let end = a.sample(100 + a.duration_ms);
        assert!(end.opacity < 0.01 && end.scale < 0.95);
        // Gone (alpha) one step faster than it shrinks.
        assert!(a.sample(100 + Duration::Fast.ms()).opacity < 0.01);
    }

    #[test]
    fn minimize_flies_toward_the_dock_target() {
        let a = Animation::minimize(0, (600.0, 700.0));
        let end = a.sample(a.duration_ms);
        assert_eq!(end.offset_to, Some((600.0, 700.0)));
        assert!(end.scale < 0.2);
    }

    #[test]
    fn mac_like_timing() {
        assert!((180..=280).contains(&Animation::open(0).duration_ms));
        assert!((150..=280).contains(&Animation::close(0).duration_ms));
        assert_eq!(Animation::minimize(0, (0.0, 0.0)).duration_ms, 400);
    }

    #[test]
    fn curves_are_the_shells_names_and_points() {
        let names: Vec<&str> = Curve::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(
            names,
            [
                "ease",
                "linear",
                "in",
                "out",
                "in-out",
                "glass",
                "overshoot",
                "soft"
            ]
        );
        for c in Curve::ALL {
            assert_eq!(Curve::from_name(c.name()), Some(c));
            assert!(
                c.at(0.0).abs() < 1e-3 && (c.at(1.0) - 1.0).abs() < 1e-3,
                "{c:?}"
            );
        }
        assert_eq!(Curve::Ease.bezier(), Bezier::EASE_OUT);
        assert_eq!(Curve::In.bezier(), Bezier::EASE_IN);
        assert_eq!(Curve::Glass.bezier(), Bezier(0.34, 1.56, 0.64, 1.0));
        assert_eq!(Curve::Overshoot.bezier(), Bezier(0.22, 1.2, 0.36, 1.0));
        assert_eq!(Curve::Soft.bezier(), Bezier(0.25, 0.1, 0.25, 1.0));
        assert_eq!(DURATIONS_MS, [150, 250, 400]);
        // The contract never overshoots; glass and overshoot do.
        let peak = |c: Curve| {
            (0..=100)
                .map(|i| c.at(i as f32 / 100.0))
                .fold(0.0, f32::max)
        };
        for c in Curve::CONTRACT {
            assert!(peak(c) <= 1.0 + 1e-4, "{c:?}");
        }
        assert!(peak(Curve::Glass) > 1.05 && peak(Curve::Overshoot) > 1.0);
    }

    #[test]
    fn window_springs_are_the_shells_and_settle_like_the_hig_says() {
        let o = Spring::WINDOW_OPEN;
        let c = Spring::WINDOW_CLOSE;
        assert_eq!((o.stiffness, o.damping, o.mass), (385.5, 31.4, 1.0));
        assert_eq!((c.stiffness, c.damping, c.mass), (322.3, 30.5, 1.0));
        // HIG section 7 spring table: 200 ms and 240 ms, ratio 0.80 and 0.85.
        assert!((190..=210).contains(&o.settle_ms()), "{}", o.settle_ms());
        assert!((230..=250).contains(&c.settle_ms()), "{}", c.settle_ms());
        assert!((o.damping_ratio() - 0.80).abs() < 0.01);
        assert!((c.damping_ratio() - 0.85).abs() < 0.01);
        for s in Spring::ALL {
            assert!(s.peak() > 1.0 && s.peak() < 1.03, "{s:?}: {}", s.peak());
            assert_eq!(Spring::named(s.name), Some(s));
        }
    }

    #[test]
    fn every_leaf_is_on_the_scale_and_a_parent_comes_first() {
        for l in LEAVES {
            match l.timing {
                Timing::Curve(_, d) => assert!(DURATIONS_MS.contains(&d.ms()), "{l:?}"),
                Timing::Spring(s) => assert!(Spring::named(s.name).is_some()),
            }
        }
        let pos = |n: &str| LEAVES.iter().position(|l| l.leaf == n).unwrap();
        for (parent, child) in [
            ("fade", "fadeIn"),
            ("fade", "fadeOut"),
            ("fade", "fadeLayersIn"),
            ("layers", "layersOut"),
            ("workspaces", "specialWorkspace"),
        ] {
            assert!(pos(parent) < pos(child), "{parent} before {child}");
        }
        // Leaving is one step faster than entering.
        assert_eq!(leaf("fadeIn").unwrap().timing.ms(), 250);
        assert_eq!(leaf("fadeOut").unwrap().timing.ms(), 150);
        assert_eq!(leaf("layersOut").unwrap().timing.ms(), 150);
    }

    #[test]
    fn looks_motions_map_onto_the_table() {
        assert_eq!(window_in("zoom"), leaf("windowsIn"));
        assert_eq!(
            window_in("pop").unwrap().timing,
            Timing::Curve(Curve::Glass, Duration::Normal)
        );
        assert_eq!(window_in("slide").unwrap().style, Some("slide"));
        assert_eq!(window_in("bounce"), None);
    }

    #[test]
    fn the_lua_block_defines_every_curve_it_names() {
        let lua = lua();
        assert!(lua.starts_with("hl.config({ animations = { enabled = true } })\n"));
        assert!(lua.contains(
            "hl.curve(\"ease\", { type = \"bezier\", points = { { 0.2, 0.8 }, { 0.2, 1 } } })\n"
        ));
        assert!(lua.contains(
            "hl.curve(\"window-open\", { type = \"spring\", stiffness = 385.5, damping = 31.4, mass = 1 })\n"
        ));
        assert!(lua.contains(
            "hl.animation({ leaf = \"windowsIn\", enabled = true, speed = 1.9, spring = \"window-open\", style = \"popin 90%\" })\n"
        ));
        assert!(lua.contains(
            "hl.animation({ leaf = \"workspaces\", enabled = true, speed = 4.0, bezier = \"soft\", style = \"slide\" })\n"
        ));
        for l in LEAVES {
            let name = match l.timing {
                Timing::Curve(c, _) => c.name(),
                Timing::Spring(s) => s.name,
            };
            assert!(
                name == "linear" || lua.contains(&format!("hl.curve(\"{name}\", ")),
                "{name}"
            );
        }
    }
}
