//! Window animation curves. Pure: time in, visual state out.

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
#[derive(Clone, Copy, Debug)]
pub struct Bezier(pub f32, pub f32, pub f32, pub f32);

impl Bezier {
    /// macOS-like: fast start, soft landing.
    pub const EASE_OUT: Bezier = Bezier(0.2, 0.8, 0.2, 1.0);
    pub const EASE_IN: Bezier = Bezier(0.4, 0.0, 1.0, 1.0);

    /// Progress (0..1) at time fraction `x` (0..1).
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

#[derive(Clone, Copy, Debug)]
enum Kind {
    Open,
    Close,
    Minimize((f32, f32)),
}

/// One running window animation, started at `start_ms` on the compositor clock.
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    kind: Kind,
    start_ms: u64,
    pub duration_ms: u64,
}

impl Animation {
    pub fn open(start_ms: u64) -> Self {
        Self {
            kind: Kind::Open,
            start_ms,
            duration_ms: 220,
        }
    }
    pub fn close(start_ms: u64) -> Self {
        Self {
            kind: Kind::Close,
            start_ms,
            duration_ms: 180,
        }
    }
    pub fn minimize(start_ms: u64, dock: (f32, f32)) -> Self {
        Self {
            kind: Kind::Minimize(dock),
            start_ms,
            duration_ms: 380,
        }
    }

    pub fn done(&self, now_ms: u64) -> bool {
        now_ms >= self.start_ms + self.duration_ms
    }

    pub fn sample(&self, now_ms: u64) -> Visual {
        let x = now_ms.saturating_sub(self.start_ms) as f32 / self.duration_ms as f32;
        let lerp = |a: f32, b: f32, p: f32| a + (b - a) * p;
        match self.kind {
            Kind::Open => {
                let p = Bezier::EASE_OUT.ease(x);
                Visual {
                    scale: lerp(0.92, 1.0, p),
                    opacity: p,
                    offset_to: None,
                }
            }
            Kind::Close => {
                let p = Bezier::EASE_IN.ease(x);
                Visual {
                    scale: lerp(1.0, 0.92, p),
                    opacity: 1.0 - p,
                    offset_to: None,
                }
            }
            Kind::Minimize(dock) => {
                let p = Bezier::EASE_OUT.ease(x);
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
    fn open_overshoots_nothing_and_is_monotonic() {
        let a = Animation::open(0);
        let mut last = a.sample(0);
        for t in (0..=a.duration_ms).step_by(4) {
            let v = a.sample(t);
            assert!(
                v.scale >= last.scale - 1e-6 && v.opacity >= last.opacity - 1e-6,
                "t={t}"
            );
            assert!(v.scale <= 1.0 + 1e-6);
            last = v;
        }
    }

    #[test]
    fn close_runs_the_other_way_and_ends_invisible() {
        let a = Animation::close(100);
        assert_eq!(a.sample(100), Visual::REST);
        let end = a.sample(100 + a.duration_ms);
        assert!(end.opacity < 0.01 && end.scale < 0.95);
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
        assert!((150..=250).contains(&Animation::close(0).duration_ms));
        assert!((300..=500).contains(&Animation::minimize(0, (0.0, 0.0)).duration_ms));
    }
}
