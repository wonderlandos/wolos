//! Compositor scene core: z-ordered surfaces, occlusion culling, damage tracking.
pub mod anim;
pub mod bus;
pub mod cache;
pub mod ffi;
pub mod frame_stats;
pub mod frontmost;
pub mod glass;
pub mod gpu_env;
pub mod gpu_probe;
pub mod gpu_select;
pub mod gpu_tune;
pub mod hypr;
pub mod identity;
pub mod idle;
pub mod memsaver;
pub mod placement;
pub mod policy;
pub mod power;
pub mod qos;
pub mod session_lock;
pub mod titlebar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn contains(&self, o: &Rect) -> bool {
        o.x >= self.x
            && o.y >= self.y
            && o.x + o.w <= self.x + self.w
            && o.y + o.h <= self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceId(u32);

struct Surface {
    id: SurfaceId,
    rect: Rect,
    opaque: bool,
}

/// Surfaces bottom to top. Damage is screen area needing repaint since the last `take_damage`.
#[derive(Default)]
pub struct Scene {
    surfaces: Vec<Surface>,
    damage: Vec<Rect>,
    next: u32,
}

impl Scene {
    pub fn add(&mut self, rect: Rect, opaque: bool) -> SurfaceId {
        let id = SurfaceId(self.next);
        self.next += 1;
        self.surfaces.push(Surface { id, rect, opaque });
        self.damage.push(rect);
        id
    }

    pub fn move_to(&mut self, id: SurfaceId, x: i32, y: i32) {
        if let Some(s) = self.surfaces.iter_mut().find(|s| s.id == id) {
            self.damage.push(s.rect);
            s.rect.x = x;
            s.rect.y = y;
            self.damage.push(s.rect);
        }
    }

    pub fn take_damage(&mut self) -> Vec<Rect> {
        std::mem::take(&mut self.damage)
    }

    /// Surfaces to draw, bottom to top, skipping ones fully covered by a single opaque surface above.
    pub fn visible(&self) -> Vec<SurfaceId> {
        self.surfaces
            .iter()
            .enumerate()
            .filter(|(i, s)| {
                !self.surfaces[i + 1..]
                    .iter()
                    .any(|a| a.opaque && a.rect.contains(&s.rect))
            })
            .map(|(_, s)| s.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn opaque_window_hides_fully_covered_one_below() {
        let mut s = Scene::default();
        let back = s.add(r(10, 10, 100, 100), true);
        let front = s.add(r(0, 0, 200, 200), true);
        assert_eq!(s.visible(), vec![front]);
        let _ = back;
    }

    #[test]
    fn translucent_window_does_not_occlude() {
        let mut s = Scene::default();
        let back = s.add(r(10, 10, 100, 100), true);
        let glass = s.add(r(0, 0, 200, 200), false);
        assert_eq!(s.visible(), vec![back, glass]);
    }

    #[test]
    fn moving_damages_old_and_new_area_once() {
        let mut s = Scene::default();
        let w = s.add(r(0, 0, 10, 10), true);
        s.take_damage();
        s.move_to(w, 100, 100);
        assert_eq!(s.take_damage(), vec![r(0, 0, 10, 10), r(100, 100, 10, 10)]);
        assert!(s.take_damage().is_empty());
    }
}
