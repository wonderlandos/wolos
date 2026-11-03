// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit_window_dump`: the scene, layout and accessibility tree as text. This is the format the
//! goldens compare (the shard's golden patch-stream specs decode their bytes into it), so it is
//! stable and documented in `docs/modules/libwolkit.md`.
//!
//! ```text
//! scene seq=7 nodes=3
//! 0 box
//!   1 box classes=3,4 state=disabled grow=1
//!     2 text "Hello"
//! ```

use crate::scene::prop::{self, DIM_AUTO, DIM_PERCENT, Ty};
use crate::scene::state;
use crate::scene::{Idx, PropValue, SceneRead, walk};
use std::fmt::Write;

pub const SCENE: u32 = 1;
pub const LAYOUT: u32 = 2;
pub const A11Y: u32 = 3;

fn state_names(bits: u32) -> String {
    let names: Vec<_> = state::NAMES.iter().filter(|(_, b)| bits & b != 0 && state::APP & b != 0).map(|(n, _)| n.to_ascii_lowercase()).collect();
    names.join("|")
}

fn value(s: &dyn SceneRead, d: &prop::PropDef, v: &PropValue) -> String {
    let f = |i: usize| f32::from_bits(v.w[i]);
    let token = |t: u32| shared::theme::TOKENS.get(t as usize).copied().unwrap_or("?");
    match d.ty {
        Ty::F32 => format!("{}", f(0)),
        Ty::I32 => format!("{}", v.w[0] as i32),
        Ty::Bool => format!("{}", v.w[0] == 1),
        Ty::Enum => d.enums.get(v.w[0] as usize).copied().unwrap_or("?").to_owned(),
        Ty::Token => token(v.w[0]).to_owned(),
        Ty::Str => format!("{:?}", s.string(v.w[0])),
        Ty::U32 => format!("{}", v.w[0]),
        Ty::Dim => match v.w[0] {
            DIM_AUTO => "auto".into(),
            DIM_PERCENT => format!("{}%", f(1)),
            _ => format!("{}px", f(1)),
        },
        Ty::Pair => format!("{},{}", f(0), f(1)),
        Ty::Border => format!("{}px {}", f(0), token(v.w[1])),
        Ty::Gradient => {
            let stops: Vec<_> = (0..v.w[1] as usize).map(|i| token(v.w[2 + i])).collect();
            format!("{}deg {}", f(0), stops.join(">"))
        }
        Ty::Shadow => format!("{} {} {} {}", f(0), f(1), f(2), token(v.w[3])),
        Ty::Transition => format!("{} {} {}", v.w[0], ["fast", "normal", "slow"].get(v.w[1] as usize).copied().unwrap_or("?"), v.w[2]),
    }
}

/// The dump of kind `what` (`SCENE`, `LAYOUT`, `A11Y`).
pub fn dump(scene: &dyn SceneRead, what: u32) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} nodes={}", ["", "scene", "layout", "a11y"].get(what as usize).copied().unwrap_or("?"), scene.node_count());
    walk(scene, 0, &mut |i: Idx, depth| {
        let pad = "  ".repeat(depth);
        let id = scene.id(i).0;
        let kind = scene.kind(i).name();
        let _ = write!(out, "{pad}{id} {kind}");
        match what {
            LAYOUT => {
                let r = scene.rect(i);
                let _ = write!(out, " {} {} {} {}", r.x, r.y, r.w, r.h);
            }
            A11Y => {
                let a = scene.a11y(i);
                let role = crate::scene::kind::ROLES.iter().find(|r| r.0 == a.role).map_or("?", |r| r.1).to_ascii_lowercase();
                let _ = write!(out, " role={role} name={:?}", a.name);
                if !a.description.is_empty() {
                    let _ = write!(out, " description={:?}", a.description);
                }
                let st = scene.state(i);
                if st & state::FOCUS != 0 {
                    out.push_str(" focused");
                }
                let names = state_names(st);
                if !names.is_empty() {
                    let _ = write!(out, " state={names}");
                }
            }
            _ => {
                let cls = scene.classes(i);
                if !cls.is_empty() {
                    let list: Vec<_> = cls.iter().map(u32::to_string).collect();
                    let _ = write!(out, " classes={}", list.join(","));
                }
                let names = state_names(scene.state(i));
                if !names.is_empty() {
                    let _ = write!(out, " state={names}");
                }
                if scene.key(i) != 0 {
                    let _ = write!(out, " key={}", scene.key(i));
                }
                if scene.event_mask(i) != 0 {
                    let _ = write!(out, " on={:#x}", scene.event_mask(i));
                }
                scene.each_prop(i, &mut |p, v| {
                    if let Some(d) = prop::def(u32::from(p)) {
                        let _ = write!(out, " {}={}", d.name, value(scene, d, v));
                    }
                });
                if !scene.text(i).is_empty() {
                    let _ = write!(out, " {:?}", scene.text(i));
                }
            }
        }
        out.push('\n');
    });
    out
}
