# SPDX-License-Identifier: BSD-2-Clause
# Wolkit::Env: the read-only signals a view can depend on to follow the person's settings: dark mode,
# the accent colour, reduced motion, high contrast, the display scale and the text scale. The engine
# reports them (a theme-changed event whenever the desktop changes, a resize event when the scale
# changes); a view reads them like any signal and its effects re-run.
require "./reactive/signal"

module Wolkit
  module Env
    # `THEME_CHANGED` flags (WOLKIT_ENV_*).
    REDUCE_MOTION       = 1_u32
    REDUCE_TRANSPARENCY = 2_u32
    CONTRAST            = 4_u32

    @@dark = Signal(Bool).new(false)
    @@accent = Signal(UInt32).new(0x7c5cff_u32)
    @@reduce_motion = Signal(Bool).new(false)
    @@reduce_transparency = Signal(Bool).new(false)
    @@contrast = Signal(Bool).new(false)
    @@scale = Signal(Float32).new(1.0_f32)
    @@text_scale = Signal(Float32).new(1.0_f32)

    # Dark appearance is on.
    def self.dark : Readable(Bool)
      @@dark
    end

    # The accent colour as 0xRRGGBB.
    def self.accent : Readable(UInt32)
      @@accent
    end

    def self.reduce_motion : Readable(Bool)
      @@reduce_motion
    end

    def self.reduce_transparency : Readable(Bool)
      @@reduce_transparency
    end

    # High contrast is on.
    def self.contrast : Readable(Bool)
      @@contrast
    end

    # Device pixels per logical pixel of the window the app opened first.
    def self.scale : Readable(Float32)
      @@scale
    end

    def self.text_scale : Readable(Float32)
      @@text_scale
    end

    # A theme-changed event arrived (or the headless engine's environment was set).
    def self.apply_theme(dark : Bool, accent : UInt32, flags : UInt32) : Nil
      Wolkit.batch do
        @@dark.value = dark
        @@accent.value = accent
        @@reduce_motion.value = flags & REDUCE_MOTION != 0
        @@reduce_transparency.value = flags & REDUCE_TRANSPARENCY != 0
        @@contrast.value = flags & CONTRAST != 0
      end
    end

    def self.apply_scale(scale : Float32) : Nil
      @@scale.value = scale
    end
  end
end
