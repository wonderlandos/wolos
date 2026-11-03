# SPDX-License-Identifier: MIT
# Portions: see docs/THIRD-PARTY.md
require "./color"
require "./image"

module Hue
  # From pixels to dominant colours: sample a 400 px copy of the picture, weight vivid pixels,
  # median-cut in OKLab to 48 colours sorted by coverage. Plus the scoring helpers the palette
  # generators share (background/foreground choice, ANSI slot matching, monochrome detection).
  module Extract
    ANSI_SIZE      =    16
    SCALE_SIZE     =   400
    MIN_PIXELS     =  1000
    MAX_PIXELS     = 50000
    DOMINANT_COUNT =    48

    # A pixel above this OKLCH chroma gets 1..3 extra copies in the pool, graded linearly over
    # BOOST_RAMP, so a small saturated highlight is not drowned by a big muted background.
    BOOST_THRESHOLD = 0.04
    BOOST_MAX_EXTRA =    3
    BOOST_RAMP      = 0.16

    # How much a colour's share of the picture adds to its lightness score when choosing the
    # background (0 = only the most extreme colour wins).
    DOMINANCE_WEIGHT = 0.6

    MONO_CHROMA          =  0.04
    MONO_SHARE           =   0.6
    HUE_CLUSTER          =  0.85
    MONO_COVERAGE_FLOOR  =  0.06
    AUTO_MONO_ARC        =  46.0
    EXPLICIT_MONO_ARC    =  28.0
    MONO_CHROMA_FLOOR    =   0.4
    MONO_MIN_L_STEP      = 0.072
    ACCENT_CHROMA_FLOOR  =  0.05
    ACCENT_CHROMA_CEIL   =  0.16
    NEAR_ACHROMATIC      =  0.03
    ACCENT_MIN_DELTA_E   =  0.05
    ACCENT_SUPPORT       =  0.04
    HUE_SUPPORT_TOL      =  30.0
    MIN_HUE_SUPPORT      = 0.025
    MIN_TINT_CHROMA      = 0.008
    MIN_CHROMA_FOR_ANSI  = 0.035
    LOW_CHROMA           =  0.05
    IDEAL_CHROMA_MIN     =  0.06
    IDEAL_CHROMA_MAX     =  0.20
    TOO_DARK             =  0.25
    TOO_BRIGHT           =  0.87
    MIN_CONTRAST         =   4.5
    MIN_HIGH_CONTRAST    =   7.0
    MIN_COMMENT_CONTRAST =   3.0
    MIN_FG_BG_CONTRAST   =   7.0
    BRIGHT_L_BOOST       =  0.12
    BRIGHT_C_BOOST       =   1.1
    DARK_L               =  0.50

    # OKLCH hue of the pure sRGB primaries for red, green, yellow, blue, magenta, cyan.
    ANSI_HUES = StaticArray[29.2, 139.1, 111.3, 266.7, 326.4, 194.8]

    class Failed < Exception
    end

    # Opaque pixels of the 400 px copy, every `step`th row and column (at most ~50000).
    def self.sample(pic : Picture) : Array(Color::RGB)
      small = pic.fit(SCALE_SIZE)
      total = small.width * small.height
      step = Math.max(1, (total.to_f / MAX_PIXELS).floor.to_i)
      res = [] of Color::RGB
      px = small.rgba
      y = 0
      while y < small.height
        x = 0
        while x < small.width
          o = (y * small.width + x) * 4
          res << Color::RGB.new(px[o].to_f, px[o + 1].to_f, px[o + 2].to_f) if px[o + 3] >= 128
          x += step
        end
        y += step
      end
      res
    end

    def self.boost(pixels : Array(Color::Lab)) : Array(Color::Lab)
      res = Array(Color::Lab).new(pixels.size * 2)
      pixels.each do |p|
        res << p
        chroma = Math.sqrt(p.a * p.a + p.b * p.b)
        next if chroma <= BOOST_THRESHOLD
        t = (chroma - BOOST_THRESHOLD) / BOOST_RAMP
        extra = Math.max(1, Math.min(Color.round(t * BOOST_MAX_EXTRA).to_i, BOOST_MAX_EXTRA))
        extra.times { res << p }
      end
      res
    end

    # Total order on floats for sort blocks (NaN never occurs here).
    def self.cmp(x : Float64, y : Float64) : Int32
      x < y ? -1 : (x > y ? 1 : 0)
    end

    # The first comparison unless it ties, then the second.
    def self.tie(a : Int32, b : Int32) : Int32
      a != 0 ? a : b
    end

    private def self.axis(c : Color::Lab, i : Int32) : Float64
      i == 0 ? c.l : (i == 1 ? c.a : c.b)
    end

    # A median-cut box: a slice of the shared pool and its range on each OKLab axis.
    private struct Box
      getter from : Int32
      getter size : Int32
      getter lo : StaticArray(Float64, 3)
      getter hi : StaticArray(Float64, 3)

      def initialize(pool : Array(Color::Lab), @from, @size)
        @lo = StaticArray(Float64, 3).new(0.0)
        @hi = StaticArray(Float64, 3).new(0.0)
        return if @size == 0
        f = pool[@from]
        @lo = StaticArray[f.l, f.a, f.b]
        @hi = @lo
        (@from...(@from + @size)).each do |k|
          c = pool[k]
          v = StaticArray[c.l, c.a, c.b]
          3.times do |i|
            @lo[i] = v[i] if v[i] < @lo[i]
            @hi[i] = v[i] if v[i] > @hi[i]
          end
        end
      end

      def range(i) : Float64
        @hi[i] - @lo[i]
      end

      # Lightness counts 1.2x: it is the most visible difference.
      def longest : Int32
        l = range(0) * 1.2
        a = range(1)
        b = range(2)
        return 0 if l >= a && l >= b
        a >= b ? 1 : 2
      end

      def volume : Float64
        range(0) * range(1) * range(2) * @size
      end
    end

    # Median cut in OKLab: split the box with the largest volume at its median along its longest
    # axis until `n` boxes exist. Returns (hex, pixel count) per box.
    def self.median_cut(pool : Array(Color::Lab), n : Int32) : Array({String, Int32})
      return [] of {String, Int32} if pool.empty?
      return dedup(pool) if pool.size <= n
      boxes = [Box.new(pool, 0, pool.size)]
      while boxes.size < n
        best = -1
        vol = 0.0
        boxes.each_with_index do |b, i|
          next unless b.size > 1
          v = b.volume
          if v > vol
            vol = v
            best = i
          end
        end
        break if best < 0
        b = boxes[best]
        ax = b.longest
        part = pool[b.from, b.size].sort! { |x, y| cmp(axis(x, ax), axis(y, ax)) }
        pool[b.from, b.size] = part
        mid = b.size // 2
        boxes[best, 1] = [Box.new(pool, b.from, mid), Box.new(pool, b.from + mid, b.size - mid)]
      end
      boxes.compact_map do |bx|
        next if bx.size == 0
        l = a = bb = 0.0
        (bx.from...(bx.from + bx.size)).each do |k|
          c = pool[k]
          l += c.l
          a += c.a
          bb += c.b
        end
        {Color.lab_to_hex(Color::Lab.new(l / bx.size, a / bx.size, bb / bx.size)), bx.size}
      end
    end

    private def self.dedup(colors : Array(Color::Lab)) : Array({String, Int32})
      res = [] of {String, Int32}
      colors.each do |c|
        next if res.any? { |u| Color.distance(c, Color.hex_to_lab(u[0])) < 0.01 }
        res << {Color.lab_to_hex(c), 1}
      end
      res
    end

    # Dominant colours ("#RRGGBB", by count, largest first) and their counts.
    def self.dominant(pixels : Array(Color::RGB), n = DOMINANT_COUNT) : {Array(String), Array(Int32)}
      raise Failed.new("not enough pixels to read colours from") if pixels.size < MIN_PIXELS // 10
      lab = pixels.map { |p| Color.rgb_to_lab(p.r, p.g, p.b) }
      q = median_cut(boost(lab), n)
      raise Failed.new("no colours found in the picture") if q.empty?
      q = q.each_with_index.to_a.sort! { |x, y| tie(y[0][1] <=> x[0][1], x[1] <=> y[1]) }.map(&.[0])
      {q.map(&.[0].upcase), q.map(&.[1])}
    end

    def self.weights(counts : Array(Int32)) : Array(Float64)?
      total = counts.sum
      return if total == 0
      counts.map { |c| c.to_f / total }
    end

    # ---- shared analysis ----

    def self.hue_distance(a : Float64, b : Float64) : Float64
      d = (a - b).abs
      d > 180 ? 360 - d : d
    end

    def self.monochrome?(colors : Array(String)) : Bool
      return false if colors.empty?
      low = 0
      sn = cs = 0.0
      chromatic = 0
      colors.each do |c|
        lch = Color.hex_to_lch(c)
        if lch.c < MONO_CHROMA
          low += 1
          next
        end
        rad = lch.h * Math::PI / 180
        sn += Math.sin(rad)
        cs += Math.cos(rad)
        chromatic += 1
      end
      return true if low.to_f / colors.size > MONO_SHARE
      chromatic >= 4 && Math.sqrt(sn * sn + cs * cs) / chromatic > HUE_CLUSTER
    end

    # Coverage-weighted: a picture whose colour covers under 6% of it, or whose colour is one
    # hue family, is monochrome (the vivid-pixel boost would fool a plain count).
    def self.monochrome_weighted?(colors : Array(String), weights : Array(Float64)?) : Bool
      return false if colors.empty?
      return monochrome?(colors) unless weights
      achro = chroma = sn = cs = 0.0
      colors.each_with_index do |c, i|
        w = weights[i]? || 0.0
        lch = Color.hex_to_lch(c)
        if lch.c < MONO_CHROMA
          achro += w
          next
        end
        chroma += w
        rad = lch.h * Math::PI / 180
        sn += Math.sin(rad) * w
        cs += Math.cos(rad) * w
      end
      total = achro + chroma
      return false if total == 0
      return true if chroma / total < MONO_COVERAGE_FLOOR
      Math.sqrt(sn * sn + cs * cs) / chroma > HUE_CLUSTER
    end

    def self.by_lightness(colors : Array(String), weights : Array(Float64)?, lightest : Bool,
                          exclude : Set(Int32)? = nil) : {String, Int32}
      best = 0
      score_best = -Float64::INFINITY
      colors.each_with_index do |c, i|
        next if exclude && exclude.includes?(i)
        l = Color.hex_to_lab(c).l
        score = lightest ? l : 1 - l
        score += DOMINANCE_WEIGHT * weights[i] if weights && i < weights.size
        if score > score_best
          score_best = score
          best = i
        end
      end
      {colors[best], best}
    end

    def self.background(colors, weights, light : Bool) : {String, Int32}
      by_lightness(colors, weights, light)
    end

    def self.foreground(colors : Array(String), weights : Array(Float64)?, light : Bool, bg : String,
                        used : Set(Int32)) : {String, Int32}
      fg, idx = by_lightness(colors, weights, !light, used)
      con = Color.contrast(bg, fg)
      return {fg, idx} if con >= MIN_FG_BG_CONTRAST
      cands = [] of {Int32, Float64}
      colors.each_with_index do |c, i|
        next if used.includes?(i)
        cr = Color.contrast(bg, c)
        cands << {i, cr} if cr > con
      end
      cands.sort! { |x, y| tie(cmp(y[1], x[1]), x[0] <=> y[0]) }
      return {colors[cands[0][0]], cands[0][0]} if !cands.empty? && cands[0][1] >= MIN_CONTRAST
      bl = Color.hex_to_lab(bg)
      fl = Color.hex_to_lab(fg)
      {Color.lab_to_hex(fl.copy_with(l: bl.l < 0.5 ? 0.97 : 0.05)), -1}
    end

    # Lower is a better match of `lch` for an ANSI slot of hue `target`.
    def self.score(lch : Color::LCH, target : Float64, light : Bool) : Float64
      hue = hue_distance(lch.h, target) * 2.0
      chroma = if lch.c < MIN_CHROMA_FOR_ANSI
                 80.0
               elsif lch.c < LOW_CHROMA
                 40.0
               elsif lch.c < IDEAL_CHROMA_MIN
                 15.0
               elsif lch.c <= IDEAL_CHROMA_MAX
                 0.0
               else
                 (lch.c - IDEAL_CHROMA_MAX) * 50
               end
      ideal = light ? 0.45 : 0.60
      lightness = if lch.l < TOO_DARK
                    (TOO_DARK - lch.l) * 200
                  elsif lch.l > TOO_BRIGHT
                    (lch.l - TOO_BRIGHT) * 150
                  else
                    (lch.l - ideal).abs * 20
                  end
      hue + chroma + lightness
    end

    def self.best_match(target : Float64, pool : Array(String), used : Set(Int32), light : Bool) : Int32
      best = -1
      s_best = Float64::INFINITY
      pool.each_with_index do |c, i|
        next if used.includes?(i)
        s = score(Color.hex_to_lch(c), target, light)
        if s < s_best
          s_best = s
          best = i
        end
      end
      best < 0 ? 0 : best
    end

    # The "bright" twin of ANSI 1-6: lighter, or (already light) more chromatic.
    def self.bright(hex : String) : String
      lab = Color.hex_to_lab(hex)
      lch = Color.lab_to_lch(lab)
      if lab.l >= 0.78
        return Color.lch(Math.min(0.94, lab.l + 0.04), Math.min(0.32, Math.max(lch.c + 0.04, lch.c * 1.3)), lch.h)
      end
      room = 0.92 - lab.l
      boost = Math.max(0.04, Math.min(BRIGHT_L_BOOST, room * 0.6))
      Color.lch(Math.min(0.92, lab.l + boost), Math.min(0.30, lch.c * BRIGHT_C_BOOST), lch.h)
    end

    record Lit, color : String, l : Float64, h : Float64

    def self.sort_by_lightness(colors : Array(String)) : Array(Lit)
      colors.map { |c| lch = Color.hex_to_lch(c); Lit.new(c, lch.l, lch.h) }
        .each_with_index.to_a.sort! { |x, y| tie(cmp(x[0].l, y[0].l), x[1] <=> y[1]) }.map(&.[0])
    end

    # Global greedy: repeatedly take the best (slot, colour) pair over all open slots.
    def self.assign(pool : Array(String), used : Set(Int32), light : Bool) : Array(Int32?)
      table = (0...6).map do |a|
        pool.each_with_index.map do |c, i|
          {i, used.includes?(i) ? Float64::INFINITY : score(Color.hex_to_lch(c), ANSI_HUES[a], light)}
        end.to_a.sort! { |x, y| tie(cmp(x[1], y[1]), x[0] <=> y[0]) }
      end
      res = Array(Int32?).new(6, nil)
      taken = used.dup
      6.times do
        ba = -1
        bi = -1
        bs = Float64::INFINITY
        6.times do |a|
          next if res[a]
          table[a].each do |(i, s)|
            next if taken.includes?(i)
            if s < bs
              bs = s
              ba = a
              bi = i
            end
            break
          end
        end
        break if ba < 0
        res[ba] = bi
        taken << bi
      end
      res
    end
  end
end
