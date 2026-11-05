// SPDX-License-Identifier: BSD-2-Clause
//! The shim's new-frame step (`src/engine/frame.rs`) without the engine. Defect 2 of WEBAPP-bench:
//! the shim painted each frame but never presented it, so the engine said `load=3 frames=5` and the
//! compositor saw no buffer on the page's subsurface.
#[path = "../src/engine/frame.rs"]
mod frame;

use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Rec(RefCell<Vec<&'static str>>);
impl frame::Frame for Rec {
    fn make_current(&self) {
        self.0.borrow_mut().push("make_current");
    }
    fn paint(&self) {
        self.0.borrow_mut().push("paint");
    }
    fn present(&self) {
        self.0.borrow_mut().push("present");
    }
}

#[test]
fn every_new_frame_is_painted_then_presented() {
    let frames = Cell::new(0);
    let r = Rec::default();
    frame::ready(&frames, &r);
    frame::ready(&frames, &r);
    assert_eq!(frames.get(), 2);
    assert_eq!(
        *r.0.borrow(),
        [
            "make_current",
            "paint",
            "present",
            "make_current",
            "paint",
            "present"
        ],
        "a frame that is painted but not presented never reaches the Wayland surface"
    );
}
