# SPDX-License-Identifier: MIT
# Portions: see docs/THIRD-PARTY.md
module Hue
  lib LibM
    fun fmod(x : Float64, y : Float64) : Float64
  end

  # Colour maths for the palette engine: sRGB <-> OKLab/OKLCH, HSL, WCAG contrast and the
  # 12-step adjustment pipeline. Colours travel as "#rrggbb" strings between steps on purpose:
  # every step quantises to 8 bits, exactly like the reference engine the goldens come from.
  module Color
    record RGB, r : Float64, g : Float64, b : Float64
    record HSL, h : Float64, s : Float64, l : Float64
    record Lab, l : Float64, a : Float64, b : Float64
    record LCH, l : Float64, c : Float64, h : Float64

    # C fmod (sign of the dividend), not Crystal's floored `%`.
    def self.fmod(x : Float64, y : Float64) : Float64
      LibM.fmod(x, y)
    end

    # Round half away from zero (Crystal's default is half to even).
    def self.round(x : Float64) : Float64
      x.round(:ties_away)
    end

    def self.clamp(v : Float64, lo : Float64, hi : Float64) : Float64
      Math.max(lo, Math.min(hi, v))
    end

    private def self.nibble(c : UInt8) : Int32
      case c
      when '0'.ord..'9'.ord then c.to_i32 - '0'.ord
      when 'a'.ord..'f'.ord then c.to_i32 - 'a'.ord + 10
      when 'A'.ord..'F'.ord then c.to_i32 - 'A'.ord + 10
      else                       -1
      end
    end

    # "#rrggbb" or "rrggbb"; anything else is black (as the reference does).
    def self.hex_to_rgb(hex : String) : RGB
      s = hex.to_slice
      off = s.size == 7 && s[0] == '#'.ord ? 1 : (s.size == 6 ? 0 : -1)
      return RGB.new(0.0, 0.0, 0.0) if off < 0
      v = StaticArray(Int32, 6).new { |i| nibble(s[off + i]) }
      return RGB.new(0.0, 0.0, 0.0) if v.any?(&.<(0))
      RGB.new((v[0] * 16 + v[1]).to_f, (v[2] * 16 + v[3]).to_f, (v[4] * 16 + v[5]).to_f)
    end

    def self.valid_hex?(s : String) : Bool
      s.matches?(/\A#?[0-9a-fA-F]{6}\z/)
    end

    def self.byte(v : Float64) : Int32
      return 0 if v <= 0
      return 255 if v >= 255
      round(v).to_i
    end

    def self.rgb_to_hex(r : Float64, g : Float64, b : Float64) : String
      String.build(7) do |io|
        io << '#'
        {r, g, b}.each { |c| io << byte(c).to_s(16).rjust(2, '0') }
      end
    end

    # ---- HSL (H 0-360, S and L 0-100) ----

    def self.rgb_to_hsl(r : Float64, g : Float64, b : Float64) : HSL
      r /= 255
      g /= 255
      b /= 255
      mx = Math.max(r, Math.max(g, b))
      mn = Math.min(r, Math.min(g, b))
      l = (mx + mn) / 2
      h = 0.0
      s = 0.0
      if mx != mn
        d = mx - mn
        s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn)
        if mx == r
          h = (g - b) / d
          h += 6 if g < b
          h /= 6
        elsif mx == g
          h = ((b - r) / d + 2) / 6
        else
          h = ((r - g) / d + 4) / 6
        end
      end
      HSL.new(h * 360, s * 100, l * 100)
    end

    private def self.hue2rgb(p : Float64, q : Float64, t : Float64) : Float64
      t += 1 if t < 0
      t -= 1 if t > 1
      return p + (q - p) * 6 * t if t < 1.0 / 6.0
      return q if t < 1.0 / 2.0
      return p + (q - p) * (2.0 / 3.0 - t) * 6 if t < 2.0 / 3.0
      p
    end

    def self.hsl_to_rgb(h : Float64, s : Float64, l : Float64) : RGB
      h /= 360
      s /= 100
      l /= 100
      if s == 0
        return RGB.new(l * 255, l * 255, l * 255)
      end
      q = l < 0.5 ? l * (1 + s) : l + s - l * s
      p = 2 * l - q
      RGB.new(hue2rgb(p, q, h + 1.0 / 3.0) * 255, hue2rgb(p, q, h) * 255, hue2rgb(p, q, h - 1.0 / 3.0) * 255)
    end

    def self.hsl_to_hex(h : Float64, s : Float64, l : Float64) : String
      c = hsl_to_rgb(h, s, l)
      rgb_to_hex(c.r, c.g, c.b)
    end

    def self.hex_to_hsl(hex : String) : HSL
      c = hex_to_rgb(hex)
      rgb_to_hsl(c.r, c.g, c.b)
    end

    # ---- OKLab / OKLCH ----

    # The published OKLab M1 matrix. `legacy_lms = true` swaps in the third row the reference
    # engine was recorded with (0.2164557844 / 0.6952417517 instead of 0.2817188376 /
    # 0.6299787005): that row does not round-trip (#123456 -> #143359), so it exists only to
    # compare palettes against recordings made with it.
    class_property? legacy_lms = false

    private def self.to_linear(c : Float64) : Float64
      c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
    end

    private def self.to_srgb(c : Float64) : Float64
      c <= 0.0031308 ? 12.92 * c : 1.055 * (c ** (1.0 / 2.4)) - 0.055
    end

    def self.rgb_to_lab(r : Float64, g : Float64, b : Float64) : Lab
      r = to_linear(r / 255.0)
      g = to_linear(g / 255.0)
      b = to_linear(b / 255.0)
      l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
      m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
      s = if @@legacy_lms
            0.0883024619 * r + 0.2164557844 * g + 0.6952417517 * b
          else
            0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
          end
      l_ = Math.cbrt(l)
      m_ = Math.cbrt(m)
      s_ = Math.cbrt(s)
      Lab.new(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_)
    end

    def self.lab_to_rgb(lab : Lab) : RGB
      l_ = lab.l + 0.3963377774 * lab.a + 0.2158037573 * lab.b
      m_ = lab.l - 0.1055613458 * lab.a - 0.0638541728 * lab.b
      s_ = lab.l - 0.0894841775 * lab.a - 1.2914855480 * lab.b
      l = l_ * l_ * l_
      m = m_ * m_ * m_
      s = s_ * s_ * s_
      r = +4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
      g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
      b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
      r = Math.max(0.0, Math.min(1.0, r))
      g = Math.max(0.0, Math.min(1.0, g))
      b = Math.max(0.0, Math.min(1.0, b))
      RGB.new(round(to_srgb(r) * 255), round(to_srgb(g) * 255), round(to_srgb(b) * 255))
    end

    def self.lab_to_lch(lab : Lab) : LCH
      c = Math.sqrt(lab.a * lab.a + lab.b * lab.b)
      h = Math.atan2(lab.b, lab.a) * 180.0 / Math::PI
      h += 360 if h < 0
      LCH.new(lab.l, c, h)
    end

    def self.lch_to_lab(lch : LCH) : Lab
      rad = lch.h * Math::PI / 180.0
      Lab.new(lch.l, lch.c * Math.cos(rad), lch.c * Math.sin(rad))
    end

    def self.hex_to_lab(hex : String) : Lab
      c = hex_to_rgb(hex)
      rgb_to_lab(c.r, c.g, c.b)
    end

    def self.hex_to_lch(hex : String) : LCH
      lab_to_lch(hex_to_lab(hex))
    end

    def self.lab_to_hex(lab : Lab) : String
      c = lab_to_rgb(lab)
      rgb_to_hex(c.r, c.g, c.b)
    end

    def self.lch_to_hex(lch : LCH) : String
      lab_to_hex(lch_to_lab(lch))
    end

    def self.lch(l : Float64, c : Float64, h : Float64) : String
      lch_to_hex(LCH.new(l, c, h))
    end

    # Euclidean OKLab distance (deltaE OK).
    def self.distance(a : Lab, b : Lab) : Float64
      dl = a.l - b.l
      da = a.a - b.a
      db = a.b - b.b
      Math.sqrt(dl * dl + da * da + db * db)
    end

    def self.delta_e(x : String, y : String) : Float64
      distance(hex_to_lab(x), hex_to_lab(y))
    end

    # ---- WCAG 2.0 ----

    def self.luminance(hex : String) : Float64
      c = hex_to_rgb(hex)
      ch = ->(v : Float64) { v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4 }
      0.2126 * ch.call(c.r / 255) + 0.7152 * ch.call(c.g / 255) + 0.0722 * ch.call(c.b / 255)
    end

    def self.contrast(a : String, b : String) : Float64
      x = luminance(a)
      y = luminance(b)
      (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
    end

    # ---- RGB blends ----

    def self.darken(hex : String, percent : Float64) : String
      c = hex_to_rgb(hex)
      rgb_to_hex(round(c.r * percent / 100), round(c.g * percent / 100), round(c.b * percent / 100))
    end

    def self.lighten(hex : String, percent : Float64) : String
      c = hex_to_rgb(hex)
      rgb_to_hex(round(c.r + (255 - c.r) * percent / 100), round(c.g + (255 - c.g) * percent / 100),
        round(c.b + (255 - c.b) * percent / 100))
    end

    # ---- the 12 sliders ----

    # Slider names (as in look files) with their range and neutral value.
    SLIDERS = {
      "vibrance"    => {-50.0, 50.0, 0.0},
      "saturation"  => {-100.0, 100.0, 0.0},
      "contrast"    => {-30.0, 30.0, 0.0},
      "brightness"  => {-30.0, 30.0, 0.0},
      "shadows"     => {-50.0, 50.0, 0.0},
      "highlights"  => {-50.0, 50.0, 0.0},
      "hueShift"    => {-180.0, 180.0, 0.0},
      "temperature" => {-50.0, 50.0, 0.0},
      "tint"        => {-50.0, 50.0, 0.0},
      "gamma"       => {0.5, 2.0, 1.0},
      "blackPoint"  => {-30.0, 30.0, 0.0},
      "whitePoint"  => {-30.0, 30.0, 0.0},
    }

    private def self.cl(l : Float64) : Float64
      Math.max(0.0, Math.min(100.0, l))
    end

    private def self.toward(h : HSL, target : Float64, amount : Float64) : HSL
      t = amount.abs / 100
      diff = fmod(target - h.h + 540, 360) - 180
      h.copy_with(h: fmod(h.h + diff * t * 0.3 + 360, 360))
    end

    # The fixed pipeline order: hue shift, temperature, tint, vibrance, saturation, brightness,
    # shadows, highlights, black point, white point, contrast, gamma. Missing sliders are neutral.
    def self.adjust(hex : String, adj : Hash(String, Float64)) : String
      v = ->(k : String) { adj[k]? || SLIDERS[k][2] }
      c = hex_to_rgb(hex)
      h = rgb_to_hsl(c.r, c.g, c.b)
      h = h.copy_with(h: fmod(h.h + v.call("hueShift") + 360, 360))
      if (t = v.call("temperature")) != 0
        h = toward(h, t > 0 ? 30.0 : 210.0, t)
      end
      if (t = v.call("tint")) != 0
        h = toward(h, t > 0 ? 300.0 : 120.0, t)
      end
      h = h.copy_with(s: cl(h.s + v.call("vibrance")))
      if (s = v.call("saturation")) != 0
        h = h.copy_with(s: cl(h.s * (100 + s) / 100))
      end
      h = h.copy_with(l: cl(h.l + v.call("brightness")))
      if (s = v.call("shadows")) != 0 && h.l < 30
        h = h.copy_with(l: cl(h.l + s * (1 - h.l / 30)))
      end
      if (s = v.call("highlights")) != 0 && h.l > 70
        h = h.copy_with(l: cl(h.l + s * ((h.l - 70) / 30)))
      end
      if (bp = v.call("blackPoint")) != 0
        mn = Math.max(0.0, bp)
        amn = Math.max(0.0, -bp)
        h = h.copy_with(l: cl(amn + (h.l * (100 - amn - mn)) / 100 + mn))
      end
      if (wp = v.call("whitePoint")) != 0
        l = wp > 0 ? Math.min(100 - wp, h.l) : cl(h.l - wp * (1 - h.l / 100))
        h = h.copy_with(l: cl(l))
      end
      factor = v.call("contrast") / 100
      h = h.copy_with(l: cl(50 + (h.l - 50) * (1 + factor)))
      gamma = v.call("gamma")
      return hsl_to_hex(h.h, h.s, h.l) if gamma == 1.0
      c = hsl_to_rgb(h.h, h.s, h.l)
      inv = 1 / gamma
      g = ->(x : Float64) { Math.max(0.0, Math.min(255.0, ((x / 255) ** inv) * 255)) }
      rgb_to_hex(g.call(c.r), g.call(c.g), g.call(c.b))
    end
  end
end
