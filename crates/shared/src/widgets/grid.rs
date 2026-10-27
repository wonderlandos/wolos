//! Desktop widget grid: cells of `cell` px, `gap` apart, `margin` from the area edge. Positions
//! are cells (stable across resolutions); drops snap to the nearest free cell.
use super::{Placed, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub cell: i32,
    pub gap: i32,
    pub margin: i32,
}

impl Metrics {
    pub fn pitch(&self) -> i32 {
        self.cell + self.gap
    }
    /// Pixel size of a widget.
    pub fn size(&self, s: Size) -> (i32, i32) {
        let (c, r) = s.cells();
        let span = |n: u16| n as i32 * self.cell + (n as i32 - 1) * self.gap;
        (span(c), span(r))
    }
    /// Top-left pixel of a cell.
    pub fn origin(&self, col: u16, row: u16) -> (i32, i32) {
        (
            self.margin + col as i32 * self.pitch(),
            self.margin + row as i32 * self.pitch(),
        )
    }
}

/// Desktop cell metrics (the shell's Kit card tokens); `wol widgets` places with the same ones.
pub const METRICS: Metrics = Metrics {
    cell: 164,
    gap: 16,
    margin: 24,
};
/// Menu bar and Dock height a headless placement leaves out.
pub const HEADLESS_BARS: i32 = 118;

/// Free-spot ranking: squared distance, then column, then row.
type Key = (i32, u16, u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub m: Metrics,
}

impl Grid {
    /// The grid `wol widgets add` places into when no screen is measured: a 1080p work area. The
    /// shell moves widgets inward on smaller screens (`arrange`), so a spot here is only a hint.
    pub fn headless(m: Metrics) -> Grid {
        Grid::new(1920, 1080 - HEADLESS_BARS, m)
    }

    /// Cells that fit an area of `w` x `h` px (at least one).
    pub fn new(w: i32, h: i32, m: Metrics) -> Grid {
        let n = |len: i32| ((len - 2 * m.margin + m.gap) / m.pitch()).max(1) as u16;
        Grid {
            cols: n(w),
            rows: n(h),
            m,
        }
    }

    pub fn contains(&self, col: u16, row: u16, s: Size) -> bool {
        let (c, r) = s.cells();
        col + c <= self.cols && row + r <= self.rows
    }

    /// Nearest top-left cell for a widget dragged to pixel `(x, y)`, kept inside the grid.
    pub fn snap(&self, x: f64, y: f64, s: Size) -> (u16, u16) {
        let (c, r) = s.cells();
        let p = self.m.pitch() as f64;
        let near = |v: f64, max: u16| {
            ((v - self.m.margin as f64) / p)
                .round()
                .clamp(0.0, max.saturating_sub(1) as f64) as u16
        };
        (
            near(x, (self.cols + 1).saturating_sub(c)),
            near(y, (self.rows + 1).saturating_sub(r)),
        )
    }

    /// Cell occupancy of `widgets` (in-grid parts only), skipping `except`.
    fn taken(&self, widgets: &[Placed], except: Option<&str>) -> Vec<bool> {
        let mut t = vec![false; self.cols as usize * self.rows as usize];
        for w in widgets.iter().filter(|w| Some(w.id.as_str()) != except) {
            let (c, r) = w.size.cells();
            for row in w.row..(w.row + r).min(self.rows) {
                for col in w.col..(w.col + c).min(self.cols) {
                    t[row as usize * self.cols as usize + col as usize] = true;
                }
            }
        }
        t
    }

    fn free_in(&self, taken: &[bool], col: u16, row: u16, s: Size) -> bool {
        let (c, r) = s.cells();
        self.contains(col, row, s)
            && (row..row + r).all(|y| {
                (col..col + c).all(|x| !taken[y as usize * self.cols as usize + x as usize])
            })
    }

    /// Whether `s` fits at `(col, row)` without covering any widget but `except`.
    pub fn is_free(
        &self,
        widgets: &[Placed],
        except: Option<&str>,
        col: u16,
        row: u16,
        s: Size,
    ) -> bool {
        self.free_in(&self.taken(widgets, except), col, row, s)
    }

    /// Free cell nearest to `prefer` (top-left first on ties, column by column like the Dock side
    /// of macOS). None when the desktop is full.
    pub fn free_spot(
        &self,
        widgets: &[Placed],
        except: Option<&str>,
        s: Size,
        prefer: (u16, u16),
    ) -> Option<(u16, u16)> {
        let taken = self.taken(widgets, except);
        // ((distance, col, row), cell)
        let mut best: Option<(Key, (u16, u16))> = None;
        for col in 0..self.cols {
            for row in 0..self.rows {
                if !self.free_in(&taken, col, row, s) {
                    continue;
                }
                let d =
                    (col as i32 - prefer.0 as i32).pow(2) + (row as i32 - prefer.1 as i32).pow(2);
                let key = (d, col, row);
                if best.is_none_or(|(k, _)| key < k) {
                    best = Some((key, (col, row)));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// Where a widget lands when dropped at `(col, row)`: there if free, else the nearest free cell,
    /// else where it was.
    pub fn drop_at(&self, widgets: &[Placed], id: &str, col: u16, row: u16) -> (u16, u16) {
        let Some(w) = widgets.iter().find(|w| w.id == id) else {
            return (col, row);
        };
        self.free_spot(widgets, Some(id), w.size, (col, row))
            .unwrap_or((w.col, w.row))
    }

    /// Display positions: widgets outside a smaller screen (or overlapping after a resize) move to
    /// the nearest free cell; the saved layout is untouched.
    pub fn arrange(&self, widgets: &[Placed]) -> Vec<(u16, u16)> {
        let mut shown: Vec<Placed> = Vec::with_capacity(widgets.len());
        let mut out = Vec::with_capacity(widgets.len());
        for w in widgets {
            let at = if self.is_free(&shown, None, w.col, w.row, w.size) {
                (w.col, w.row)
            } else {
                self.free_spot(&shown, None, w.size, (w.col, w.row))
                    .unwrap_or((w.col, w.row))
            };
            let mut p = w.clone();
            (p.col, p.row) = at;
            shown.push(p);
            out.push(at);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{Builtin, Kind, Options};

    const M: Metrics = Metrics {
        cell: 164,
        gap: 16,
        margin: 24,
    };

    fn w(id: &str, size: Size, col: u16, row: u16) -> Placed {
        Placed {
            id: id.into(),
            kind: Kind::Builtin(Builtin::Clock),
            size,
            col,
            row,
            options: Options::default(),
        }
    }

    #[test]
    fn headless_grid_is_a_1080p_desktop() {
        assert_eq!(M, METRICS);
        let g = Grid::headless(METRICS);
        assert_eq!((g.cols, g.rows), (10, 5));
    }

    #[test]
    fn metrics_and_grid_size() {
        assert_eq!(M.size(Size::Small), (164, 164));
        assert_eq!(M.size(Size::Medium), (344, 164));
        assert_eq!(M.size(Size::Large), (344, 344));
        assert_eq!(M.origin(2, 1), (24 + 360, 24 + 180));
        // 1280 x 692 (menu bar and Dock reserved): 6 x 3 cells (7 need 1292 px).
        let g = Grid::new(1280, 692, M);
        assert_eq!((g.cols, g.rows), (6, 3));
        assert_eq!(Grid::new(1292, 692, M).cols, 7);
        assert_eq!(
            (Grid::new(100, 100, M).cols, Grid::new(100, 100, M).rows),
            (1, 1)
        );
    }

    #[test]
    fn snaps_to_the_nearest_cell_inside_the_grid() {
        let g = Grid::new(1280, 692, M);
        for (x, y, s, want) in [
            (24.0, 24.0, Size::Small, (0, 0)),
            (110.0, 30.0, Size::Small, (0, 0)),
            (120.0, 30.0, Size::Small, (1, 0)),
            (-500.0, -500.0, Size::Small, (0, 0)),
            (5000.0, 5000.0, Size::Small, (5, 2)),
            (5000.0, 5000.0, Size::Medium, (4, 2)),
            (5000.0, 5000.0, Size::Large, (4, 1)),
            (
                24.0 + 2.0 * 180.0 + 80.0,
                24.0 + 180.0 + 95.0,
                Size::Small,
                (2, 2),
            ),
        ] {
            assert_eq!(g.snap(x, y, s), want, "{x},{y} {s:?}");
        }
    }

    #[test]
    fn finds_free_cells_and_never_overlaps() {
        let g = Grid::new(1280, 692, M);
        let ws = vec![w("a", Size::Large, 0, 0), w("b", Size::Small, 2, 0)];
        assert!(!g.is_free(&ws, None, 1, 1, Size::Small));
        assert!(g.is_free(&ws, Some("a"), 1, 1, Size::Small));
        assert!(!g.is_free(&ws, None, 6, 0, Size::Medium), "off the grid");
        for (s, prefer, want) in [
            (Size::Small, (0, 0), Some((0, 2))),
            (Size::Small, (2, 0), Some((2, 1))),
            (Size::Medium, (0, 0), Some((0, 2))),
            (Size::Large, (0, 0), Some((2, 1))),
        ] {
            assert_eq!(g.free_spot(&ws, None, s, prefer), want, "{s:?} {prefer:?}");
        }
        let full: Vec<Placed> = (0..6)
            .flat_map(|c| (0..3).map(move |r| w(&format!("{c}{r}"), Size::Small, c, r)))
            .collect();
        assert_eq!(g.free_spot(&full, None, Size::Small, (0, 0)), None);
    }

    #[test]
    fn drops_land_on_the_nearest_free_cell() {
        let g = Grid::new(1280, 692, M);
        let ws = vec![w("a", Size::Small, 0, 0), w("b", Size::Small, 1, 0)];
        assert_eq!(g.drop_at(&ws, "a", 3, 1), (3, 1));
        let ws = vec![
            w("a", Size::Small, 0, 0),
            w("b", Size::Small, 1, 0),
            w("c", Size::Small, 0, 1),
        ];
        assert_eq!(
            g.drop_at(&ws, "c", 1, 0),
            (1, 1),
            "occupied: nearest free cell"
        );
        assert_eq!(g.drop_at(&ws, "a", 0, 0), (0, 0), "own cell is free");
        assert_eq!(g.drop_at(&ws, "zzz", 4, 2), (4, 2));
    }

    #[test]
    fn smaller_screens_rearrange_without_overlap() {
        let small = Grid::new(600, 600, M);
        assert_eq!((small.cols, small.rows), (3, 3));
        let ws = vec![
            w("a", Size::Small, 0, 0),
            w("b", Size::Medium, 6, 0),
            w("c", Size::Small, 0, 0),
        ];
        assert_eq!(small.arrange(&ws), vec![(0, 0), (1, 0), (0, 1)]);
    }
}
