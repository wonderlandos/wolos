// SPDX-License-Identifier: BSD-2-Clause
//! What the person's desktop says about appearance: the inputs that restyle or repaint everything
//! at once. The native backend reads them from the files and preferences the shell uses; the
//! headless backend gets them from `wolkit_engine_set_environment`.

pub const FLAG_REDUCE_MOTION: u32 = 1;
pub const FLAG_REDUCE_TRANSPARENCY: u32 = 2;
pub const FLAG_CONTRAST: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub dark: bool,
    /// 0xRRGGBB.
    pub accent_rgb: u32,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    pub contrast: bool,
    /// The text-size preference, 1.0 by default.
    pub text_scale: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            dark: false,
            accent_rgb: 0x7c5cff,
            reduce_motion: false,
            reduce_transparency: false,
            contrast: false,
            text_scale: 1.0,
        }
    }
}

impl Environment {
    pub fn flags(&self) -> u32 {
        u32::from(self.reduce_motion) * FLAG_REDUCE_MOTION
            | u32::from(self.reduce_transparency) * FLAG_REDUCE_TRANSPARENCY
            | u32::from(self.contrast) * FLAG_CONTRAST
    }

    pub fn set(&mut self, dark: bool, accent_rgb: u32, flags: u32) {
        self.dark = dark;
        self.accent_rgb = accent_rgb & 0x00ff_ffff;
        self.reduce_motion = flags & FLAG_REDUCE_MOTION != 0;
        self.reduce_transparency = flags & FLAG_REDUCE_TRANSPARENCY != 0;
        self.contrast = flags & FLAG_CONTRAST != 0;
    }
}
