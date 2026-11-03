# SPDX-License-Identifier: MIT
# Portions: see docs/THIRD-PARTY.md
require "./extract"

module Hue
  # The 16 ANSI colours from a picture's dominant colours, in one of eight modes. Slot 0 is the
  # background, 7 the foreground, 1-6 the accents, 8 the comment grey, 9-15 the bright twins.
  module Palette
    alias P = Array(String)
    include Extract
    extend self

    MODES = %w(normal monochromatic analogous pastel material colorful muted bright)

    private def clampf(v, lo, hi)
      Color.clamp(v, lo, hi)
    end

    private def fmod(x, y)
      Color.fmod(x, y)
    end

    def generate(dom : P, weights : Array(Float64)?, light : Bool, mode : String) : P
      p = case mode
          when "monochromatic" then monochromatic(dom, light)
          when "analogous"     then analogous(dom, light)
          when "pastel"        then pastel(dom, light)
          when "material"      then material(dom, light)
          when "colorful"      then colorful(dom, light)
          when "muted"         then muted(dom, light)
          when "bright"        then bright_mode(dom, light)
          when "normal"
            Extract.monochrome_weighted?(dom, weights) ? monochrome(dom, light) : chromatic(dom, weights, light)
          else
            raise ArgumentError.new("unknown mode “#{mode}” (#{MODES.join(", ")})")
          end
      normalize(p)
    end

    # Every accent reads against the background (WCAG AA), its bright twin follows.
    def normalize(p : P) : P
      p = p.dup
      (1..6).each do |i|
        if Color.contrast(p[0], p[i]) < MIN_CONTRAST
          p[i] = boost_contrast(p[i], p[0], MIN_CONTRAST)
          p[i + 8] = Extract.bright(p[i])
        end
      end
      p
    end

    # ---- the default (chromatic) path ----

    def chromatic_hues(dom : P, weights : Array(Float64)?, light : Bool) : P
      top = Math.min(12, dom.size)
      tw = weights.try &.[0, top]
      bg, bi = Extract.background(dom[0, top], tw, light)
      bg = mid_bg(bg, light)
      used = Set{bi}
      fg, fi = Extract.foreground(dom, weights, light, bg, used)
      used << fi if fi >= 0
      p = P.new(16, "")
      p[0] = bg
      p[7] = fg
      Extract.assign(dom, used, light).each_with_index do |idx, i|
        p[i + 1] = dom[idx || 0]
      end
      p
    end

    private def mid_bg(bg : String, light : Bool) : String
      lch = Color.hex_to_lch(bg)
      if light
        return bg if lch.l >= 0.85
        return Color.lch(0.94, Math.min(lch.c, 0.04), lch.h)
      end
      return bg if lch.l <= 0.20
      Color.lch(0.12, Math.min(lch.c, 0.05), lch.h)
    end

    def chromatic(dom : P, weights : Array(Float64)?, light : Bool) : P
      p = chromatic_hues(dom, weights, light)
      accents!(p, dom, weights, light)
      finalize!(p)
      p
    end

    record Cluster, hue : Float64, coverage : Float64

    # Coverage-weighted 30-degree hue bands that hold at least 2.5% of the picture.
    def clusters(dom : P, weights : Array(Float64)?) : Array(Cluster)
      w = Array(Float64).new(12, 0.0)
      sn = Array(Float64).new(12, 0.0)
      cs = Array(Float64).new(12, 0.0)
      dom.each_with_index do |c, k|
        lch = Color.hex_to_lch(c)
        next if lch.c < ACCENT_SUPPORT
        wt = weights && k < weights.size ? weights[k] : 1.0 / dom.size
        b = (lch.h / 30).to_i % 12
        b += 12 if b < 0
        rad = lch.h * Math::PI / 180
        w[b] += wt
        sn[b] += Math.sin(rad) * wt
        cs[b] += Math.cos(rad) * wt
      end
      (0...12).select { |b| w[b] >= MIN_HUE_SUPPORT }.map do |b|
        Cluster.new(fmod(Math.atan2(sn[b], cs[b]) * 180 / Math::PI + 360, 360), w[b])
      end
    end

    # Accents stay inside the picture's hues: a grey or unsupported pick folds to the nearest
    # populated hue band, chroma lifts toward the picture's salient chroma, then near-equal
    # accents are separated by lightness only.
    def accents!(p : P, dom : P, weights : Array(Float64)?, light : Bool) : Nil
      target = clampf(salient_chroma(dom) * 0.85, ACCENT_CHROMA_FLOOR, ACCENT_CHROMA_CEIL)
      cl = clusters(dom, weights)
      (1..6).each do |i|
        lch = Color.hex_to_lch(p[i])
        gray = lch.c < NEAR_ACHROMATIC
        supported = cl.any? { |c| Extract.hue_distance(lch.h, c.hue) <= HUE_SUPPORT_TOL }
        if gray || !supported
          if cl.empty?
            lch = lch.copy_with(c: 0.0) if gray
          else
            t = ANSI_HUES[i - 1]
            best = Float64::INFINITY
            hue = 0.0
            cl.each do |c|
              d = Extract.hue_distance(t, c.hue)
              if d < best
                best = d
                hue = c.hue
              end
            end
            lch = lch.copy_with(h: hue)
          end
        end
        lch = lch.copy_with(c: target) if lch.c > 0 && lch.c < target
        p[i] = Color.lch_to_hex(lch)
      end
      (1..6).each do |i|
        ((i + 1)..6).each do |j|
          next unless Color.delta_e(p[i], p[j]) < ACCENT_MIN_DELTA_E
          lch = Color.hex_to_lch(p[j])
          l = light ? clampf(lch.l - 0.10, 0.20, 0.85) : clampf(lch.l + 0.10, 0.30, 0.92)
          p[j] = Color.lch_to_hex(lch.copy_with(l: l))
        end
      end
    end

    # Slots 8 (comments), 9-14 and 15 from the rest, then AA contrast for the accents.
    def finalize!(p : P) : Nil
      p[8] = comment(p[0])
      (1..6).each { |i| p[i + 8] = Extract.bright(p[i]) }
      p[15] = Extract.bright(p[7])
      (1..6).each do |i|
        if Color.contrast(p[0], p[i]) < MIN_CONTRAST
          p[i] = boost_contrast(p[i], p[0], MIN_CONTRAST)
          p[i + 8] = Extract.bright(p[i])
        end
      end
    end

    def comment(bg : String) : String
      lab = Color.hex_to_lab(bg)
      lch = Color.lab_to_lch(lab)
      dark = lab.l < 0.5
      l = dark ? Math.min(1.0, lab.l + 0.30) : Math.max(0.0, lab.l - 0.30)
      c = Color.lch(l, 0.01, lch.h)
      con = Color.contrast(bg, c)
      if con < MIN_COMMENT_CONTRAST
        if dark
          while l < 0.95 && con < MIN_COMMENT_CONTRAST
            l += 0.05
            c = Color.lch(l, 0.01, lch.h)
            con = Color.contrast(bg, c)
          end
        else
          while l > 0.05 && con < MIN_COMMENT_CONTRAST
            l -= 0.05
            c = Color.lch(l, 0.01, lch.h)
            con = Color.contrast(bg, c)
          end
        end
      end
      c
    end

    # Moves `hex` in OKLab lightness (hue and chroma kept) until it reaches `ratio` on `bg`.
    def boost_contrast(hex : String, bg : String, ratio : Float64) : String
      lch = Color.hex_to_lch(hex)
      if Color.hex_to_lab(bg).l < 0.5
        while lch.l < 0.95
          lch = lch.copy_with(l: lch.l + 0.03)
          c = Color.lch_to_hex(lch)
          return c if Color.contrast(bg, c) >= ratio
        end
      else
        while lch.l > 0.05
          lch = lch.copy_with(l: lch.l - 0.03)
          c = Color.lch_to_hex(lch)
          return c if Color.contrast(bg, c) >= ratio
        end
      end
      Color.lch_to_hex(lch)
    end

    # ---- mode helpers ----

    # Hue of the most chromatic dominant colour (0 for a grey picture).
    def dominant_hue(dom : P) : Float64
      hue = 0.0
      best = 0.0
      dom.each do |c|
        lch = Color.hex_to_lch(c)
        if lch.c > best
          best = lch.c
          hue = lch.h
        end
      end
      hue
    end

    alias Rule = Color::LCH -> Color::LCH

    # Picture hues reshaped: bg, fg and accents get the mode's lightness/chroma, accents a
    # lightness stagger so they stay apart when the mode flattens chroma.
    private def reshape(dom : P, light : Bool, bg : {Rule, Rule}, fg : {Rule, Rule}, ansi : {Rule, Rule},
                        stagger : StaticArray(Float64, 6)) : P
      base = chromatic_hues(dom, nil, light)
      pick = ->(r : {Rule, Rule}) { light ? r[0] : r[1] }
      res = P.new(16, "")
      res[0] = Color.lch_to_hex(pick.call(bg).call(Color.hex_to_lch(base[0])))
      res[7] = Color.lch_to_hex(pick.call(fg).call(Color.hex_to_lch(base[7])))
      6.times do |i|
        s = pick.call(ansi).call(Color.hex_to_lch(base[i + 1]))
        res[i + 1] = Color.lch_to_hex(s.copy_with(l: clampf(s.l + stagger[i], 0.05, 0.95)))
      end
      finalize!(res)
      res
    end

    private def fixed(l : Float64, c : Float64) : Rule
      ->(x : Color::LCH) { Color::LCH.new(l, c, x.h) }
    end

    private def ansi_rule(l : Float64, &f : Float64 -> Float64) : Rule
      ->(x : Color::LCH) { Color::LCH.new(l, f.call(x.c), x.h) }
    end

    def pastel(dom : P, light : Bool) : P
      reshape(dom, light, {fixed(0.96, 0.012), fixed(0.20, 0.022)}, {fixed(0.32, 0.05), fixed(0.85, 0.04)},
        {ansi_rule(0.55) { |c| clampf(c, 0.045, 0.07) }, ansi_rule(0.78) { |c| clampf(c, 0.05, 0.075) }},
        StaticArray[-0.04, 0.02, 0.05, -0.03, -0.01, 0.03])
    end

    def colorful(dom : P, light : Bool) : P
      reshape(dom, light, {fixed(0.97, 0.012), fixed(0.10, 0.022)}, {fixed(0.18, 0.04), fixed(0.92, 0.03)},
        {ansi_rule(0.50) { |c| clampf(Math.max(c, 0.14), 0.14, 0.20) },
         ansi_rule(0.65) { |c| clampf(Math.max(c, 0.14), 0.14, 0.20) }},
        StaticArray[-0.05, 0.02, 0.06, -0.03, -0.02, 0.04])
    end

    def muted(dom : P, light : Bool) : P
      reshape(dom, light, {fixed(0.94, 0.010), fixed(0.16, 0.018)}, {fixed(0.28, 0.045), fixed(0.84, 0.035)},
        {ansi_rule(0.48) { |c| clampf(c * 0.5, 0.035, 0.065) }, ansi_rule(0.62) { |c| clampf(c * 0.5, 0.035, 0.065) }},
        StaticArray[-0.10, -0.04, 0.04, 0.10, -0.07, 0.07])
    end

    def bright_mode(dom : P, light : Bool) : P
      reshape(dom, light, {fixed(0.98, 0.010), fixed(0.10, 0.020)}, {fixed(0.15, 0.04), fixed(0.92, 0.025)},
        {ansi_rule(0.42) { |c| clampf(Math.max(c, 0.10), 0.10, 0.18) },
         ansi_rule(0.78) { |c| clampf(Math.max(c, 0.10), 0.10, 0.18) }},
        StaticArray[-0.04, 0.01, 0.06, -0.03, -0.02, 0.03])
    end

    # Fixed neutral surfaces with HSL-tuned accents (material design's contract is in HSL).
    def material(dom : P, light : Bool) : P
      p = P.new(16, "")
      used = Set(Int32).new
      p[0], p[7] = light ? {"#fafafa", "#212121"} : {"#121212", "#ffffff"}
      6.times do |i|
        m = Extract.best_match(ANSI_HUES[i], dom, used, light)
        hsl = Color.hex_to_hsl(dom[m])
        sat = Math.max(hsl.s, 35.0)
        l = light ? Math.max(35.0, Math.min(60.0, hsl.l)) : Math.max(45.0, Math.min(70.0, hsl.l))
        p[i + 1] = Color.hsl_to_hex(hsl.h, sat, l)
        used << m
      end
      p[8] = light ? "#757575" : "#9e9e9e"
      (1..6).each do |i|
        hsl = Color.hex_to_hsl(p[i])
        l = light ? Math.max(30.0, hsl.l - 8) : Math.min(75.0, hsl.l + 8)
        p[i + 8] = Color.hsl_to_hex(hsl.h, Math.min(100.0, hsl.s + 8), l)
      end
      p[15] = light ? "#000000" : "#ffffff"
      p
    end

    # Six hues within +-30 degrees of the picture's main hue.
    def analogous(dom : P, light : Bool) : P
      base = dominant_hue(dom)
      s = Extract.sort_by_lightness(dom)
      darkest = s.first
      lightest = s.last
      p = P.new(16, "")
      if light
        p[0] = Color.lch(Math.max(0.92, lightest.l), 0.02, base)
        p[7] = Color.lch(Math.min(0.28, darkest.l + 0.05), 0.04, base)
      else
        p[0] = Color.lch(Math.min(0.16, darkest.l), 0.03, base)
        p[7] = Color.lch(Math.max(0.88, lightest.l - 0.05), 0.025, base)
      end
      offsets = StaticArray[-30.0, -18.0, -6.0, 6.0, 18.0, 30.0]
      chroma = StaticArray[0.10, 0.13, 0.09, 0.14, 0.10, 0.12]
      stagger = StaticArray[-0.06, 0.04, -0.04, 0.06, -0.05, 0.05]
      lb = light ? 0.50 : 0.62
      6.times do |i|
        hue = fmod(base + offsets[i] + 360, 360)
        p[i + 1] = Color.lch(clampf(lb + stagger[i], 0.30, 0.85), chroma[i], hue)
      end
      p[8] = comment(p[0])
      6.times do |i|
        hue = fmod(base + offsets[i] + 360, 360)
        bl = light ? clampf(lb + stagger[i] - 0.10, 0.20, 0.70) : clampf(lb + stagger[i] + 0.10, 0.30, 0.92)
        p[i + 9] = Color.lch(bl, chroma[i] + 0.02, hue)
      end
      p[15] = light ? Color.lch(0.10, 0.04, base) : Color.lch(0.97, 0.015, base)
      p
    end

    # ---- monochrome ----

    private def mono_bg(bg : String, light : Bool) : String
      lch = Color.hex_to_lch(bg)
      if light
        return bg if lch.l >= 0.75
        return Color.lch(0.94, Math.min(lch.c, 0.04), lch.h)
      end
      return bg if lch.l <= 0.35
      Color.lch(0.14, Math.min(lch.c, 0.06), lch.h)
    end

    # Chroma-weighted mean hue of a mostly grey picture; nil below the JPEG-noise floor.
    def tint(colors : P) : Float64?
      return if colors.empty?
      sn = cs = sum = 0.0
      colors.each do |c|
        lch = Color.hex_to_lch(c)
        next unless lch.c > 0.005
        rad = lch.h * Math::PI / 180
        sn += Math.sin(rad) * lch.c
        cs += Math.cos(rad) * lch.c
        sum += lch.c
      end
      return if sum / colors.size < MIN_TINT_CHROMA
      fmod(Math.atan2(sn / sum, cs / sum) * 180 / Math::PI + 360, 360)
    end

    # ~90th percentile chroma: how vivid the most colourful parts are.
    def salient_chroma(colors : P) : Float64
      return 0.0 if colors.empty?
      ch = colors.map { |c| Color.hex_to_lch(c).c }.sort!
      ch[Color.round(0.9 * (ch.size - 1)).to_i]
    end

    private def fold(canonical : Float64, base : Float64, arc : Float64) : Float64
      d = fmod(canonical - base + 540, 360) - 180
      fmod(base + (d / 180.0) * arc + 360, 360)
    end

    # Blue darkest, then red, magenta, green, cyan, yellow lightest.
    LIGHTNESS_ORDER = StaticArray[3, 0, 4, 1, 5, 2]

    private def ramp(bg : String, light : Bool) : StaticArray(Float64, 6)
      if light
        start = readable_l(bg, true, MIN_CONTRAST) - 0.02
        step = -Math.max(MONO_MIN_L_STEP, (start - 0.16) / 5.0)
      else
        start = readable_l(bg, false, MIN_CONTRAST) + 0.02
        step = Math.max(MONO_MIN_L_STEP, (0.92 - start) / 5.0)
      end
      res = StaticArray(Float64, 6).new(0.0)
      LIGHTNESS_ORDER.each_with_index { |slot, pos| res[slot] = clampf(start + step * pos, 0.06, 0.97) }
      res
    end

    private def headroom(bg : String, light : Bool) : String
      lch = Color.hex_to_lch(bg)
      if light
        return Color.lch(0.95, Math.min(lch.c, 0.04), lch.h) if readable_l(bg, true, MIN_CONTRAST) < 0.42
        return bg
      end
      return Color.lch(0.13, Math.min(lch.c, 0.06), lch.h) if readable_l(bg, false, MIN_CONTRAST) > 0.60
      bg
    end

    # First neutral grey lightness (0.01 steps away from the background) that reaches `ratio`.
    def readable_l(bg : String, light : Bool, ratio : Float64) : Float64
      unless light
        l = 0.20
        while l <= 0.95
          return l if Color.contrast(bg, Color.lch(l, 0.0, 0.0)) >= ratio
          l += 0.01
        end
        return 0.55
      end
      l = 0.80
      while l >= 0.05
        return l if Color.contrast(bg, Color.lch(l, 0.0, 0.0)) >= ratio
        l -= 0.01
      end
      0.45
    end

    def monochrome(dom : P, light : Bool) : P
      if h = tint(dom)
        tinted(dom, light, h, AUTO_MONO_ARC)
      else
        grayscale(dom, light)
      end
    end

    def monochromatic(dom : P, light : Bool) : P
      if h = tint(dom)
        tinted(dom, light, h, EXPLICIT_MONO_ARC)
      else
        grayscale(dom, light)
      end
    end

    private def grayscale(dom : P, light : Bool) : P
      s = Extract.sort_by_lightness(dom)
      p = P.new(16, "")
      if light
        p[0] = Color.lch(Math.max(0.94, s.last.l), 0.0, 0.0)
        p[7] = Color.lch(Math.min(0.22, s.first.l), 0.0, 0.0)
      else
        p[0] = Color.lch(Math.min(0.14, s.first.l), 0.0, 0.0)
        p[7] = Color.lch(Math.max(0.88, s.last.l), 0.0, 0.0)
      end
      p[0] = headroom(p[0], light)
      sl = ramp(p[0], light)
      6.times { |i| p[i + 1] = Color.lch(sl[i], 0.0, 0.0) }
      p[8] = comment(p[0])
      bump = light ? -0.06 : 0.06
      6.times { |i| p[i + 9] = Color.lch(Math.max(0.05, Math.min(0.97, sl[i] + bump)), 0.0, 0.0) }
      p[15] = Color.lch(light ? 0.04 : 0.99, 0.0, 0.0)
      p
    end

    private def tinted(dom : P, light : Bool, base : Float64, arc : Float64) : P
      s = Extract.sort_by_lightness(dom)
      p = P.new(16, "")
      if light
        p[0] = mono_bg(s.last.color, light)
        p[7] = s.first.color
      else
        p[0] = mono_bg(s.first.color, light)
        p[7] = s.last.color
      end
      p[0] = headroom(p[0], light)
      if Color.contrast(p[0], p[7]) < MIN_FG_BG_CONTRAST
        bl = Color.hex_to_lab(p[0])
        fl = Color.hex_to_lab(p[7])
        p[7] = Color.lab_to_hex(fl.copy_with(l: bl.l < 0.5 ? 0.97 : 0.05))
      end
      chroma = StaticArray[0.09, 0.11, 0.13, 0.10, 0.12, 0.14]
      bright_c = StaticArray[0.11, 0.14, 0.16, 0.12, 0.15, 0.17]
      mean = chroma.sum / 6
      factor = clampf(salient_chroma(dom) / mean, MONO_CHROMA_FLOOR, 1.0)
      sl = ramp(p[0], light)
      6.times { |i| p[i + 1] = Color.lch(sl[i], chroma[i] * factor, fold(ANSI_HUES[i], base, arc)) }
      p[8] = comment(p[0])
      adj = light ? -0.07 : 0.07
      6.times do |i|
        p[i + 9] = Color.lch(clampf(sl[i] + adj, 0.18, 0.96), bright_c[i] * factor, fold(ANSI_HUES[i], base, arc))
      end
      p[15] = light ? Color.lch(0.08, 0.03, base) : Color.lch(0.97, 0.015, base)
      p
    end
  end
end
