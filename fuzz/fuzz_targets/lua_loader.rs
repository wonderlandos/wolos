// SPDX-License-Identifier: GPL-2.0-or-later
//! Lua widget loader (`wol_lua::Widget::load`; audit B10): any script, in the sandbox with its
//! instruction budget, ends in Ok or Err, never a panic or a hang past the budget.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::rc::Rc;
use wol_lua::{Services, Stamp, Widget};

struct Fixed;

impl Services for Fixed {
    fn now(&self) -> Stamp {
        Stamp {
            year: 2026,
            month: 10,
            day: 26,
            weekday: 1,
            hour: 14,
            minute: 5,
            second: 9,
        }
    }
    fn system(&self) -> wol::system::Info {
        wol::system::Info {
            os_name: "WolOS".into(),
            os_version: "0.12".into(),
            kernel: "WolKernel".into(),
            appearance: wol::system::Appearance::Dark,
            accent_name: "blue".into(),
            accent_rgb: 0x6aaef2,
        }
    }
    fn notify(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    if let Ok(mut w) = Widget::load("fuzz", &src, Rc::new(Fixed)) {
        let _ = w.render();
    }
});
