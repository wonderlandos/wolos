//! Mission Control (Super+Up, 3-finger swipe up): desktops strip + every window of the current
//! desktop scaled into a grid. Own layer surface: hyprexpo is not in the hyprland-plugins pin.
use crate::hypr::Client;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Grid of `sizes` (window w,h) inside `area`: near-square column count, each window scaled
/// to fit its cell keeping its aspect, never above 1:1, centred in the cell.
pub fn grid(sizes: &[(i32, i32)], area: Rect, gap: f64) -> Vec<Rect> {
    let n = sizes.len();
    let place = |cols: usize| -> Vec<Rect> {
        let rows = n.div_ceil(cols);
        let cw = (area.w - gap * (cols - 1) as f64) / cols as f64;
        let ch = (area.h - gap * (rows - 1) as f64) / rows as f64;
        sizes
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| {
                let (w, h) = (w.max(1) as f64, h.max(1) as f64);
                let k = (cw / w).min(ch / h).min(1.0);
                let (col, row) = ((i % cols) as f64, (i / cols) as f64);
                // A short last row is centred.
                let in_row = (n - (i / cols) * cols).min(cols);
                let shift = (cols - in_row) as f64 * (cw + gap) / 2.0;
                Rect {
                    x: area.x + shift + col * (cw + gap) + (cw - w * k) / 2.0,
                    y: area.y + row * (ch + gap) + (ch - h * k) / 2.0,
                    w: w * k,
                    h: h * k,
                }
            })
            .collect()
    };
    let covered = |r: &[Rect]| r.iter().map(|r| r.w * r.h).sum::<f64>();
    (1..=n)
        .map(place)
        .max_by(|a, b| covered(a).total_cmp(&covered(b)))
        .unwrap_or_default()
}

/// Desktops to show: workspace ids with windows plus the current one, ascending, positive only.
pub fn desktops(clients: &[Client], current: i64) -> Vec<i64> {
    let mut v: Vec<i64> = clients
        .iter()
        .map(|c| c.workspace.id)
        .chain([current])
        .filter(|&id| id > 0)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fits_and_keeps_aspect() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 600.0,
        };
        assert!(grid(&[], area, 20.0).is_empty());
        let one = grid(&[(800, 400)], area, 20.0);
        assert_eq!(
            one,
            [Rect {
                x: 100.0,
                y: 100.0,
                w: 800.0,
                h: 400.0
            }]
        );
        let sizes = [
            (1600, 900),
            (800, 800),
            (400, 1000),
            (1000, 500),
            (300, 200),
        ];
        let r = grid(&sizes, area, 20.0);
        assert_eq!(r.len(), 5);
        for (g, s) in r.iter().zip(sizes) {
            assert!(
                g.x >= 0.0 && g.y >= 0.0 && g.x + g.w <= 1000.0 + 1e-6 && g.y + g.h <= 600.0 + 1e-6,
                "{g:?}"
            );
            assert!(
                (g.w / g.h - s.0 as f64 / s.1 as f64).abs() < 1e-6,
                "aspect {g:?}"
            );
            assert!(g.w <= s.0 as f64 + 1e-6);
        }
        for (i, a) in r.iter().enumerate() {
            for b in &r[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn short_last_row_is_centred() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 900.0,
            h: 600.0,
        };
        let r = grid(&[(400, 300); 3], area, 0.0);
        let last = r.last().unwrap();
        assert!(((last.x + last.w / 2.0) - 450.0).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn desktops_listed() {
        let c = |ws: i64| {
            let mut c = Client::new("0x1", "a", "");
            c.workspace.id = ws;
            c
        };
        assert_eq!(desktops(&[c(3), c(1), c(3), c(-99)], 2), [1, 2, 3]);
        assert_eq!(desktops(&[], 1), [1]);
    }
}
