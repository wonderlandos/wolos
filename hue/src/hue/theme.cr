# SPDX-License-Identifier: GPL-2.0-or-later
# A 16-colour palette as a WolOS theme (ADR 0031 themes v2): the 13 tokens, the `terminal` list
# and the wallpaper, written as $XDG_CONFIG_HOME/wolos/themes/<id>/theme.json. The tokens are
# derived so the theme always passes the contrast contract the desktop checks
# (shared::theme::validate, crates/shared/src/theme.rs); `check` below is the same rule set.
require "json"
require "./color"

module Hue
  module Theme
    TOKENS = %w(surface sidebar card line text dim accent accentText glass control success warning danger)

    # (foreground, background, minimum) as in crates/shared/src/theme.rs CONTRAST_RULES.
    RULES = [
      {"text", "surface", 4.5}, {"text", "card", 4.5}, {"text", "sidebar", 4.5},
      {"dim", "surface", 3.0}, {"dim", "card", 3.0}, {"accentText", "accent", 3.0},
    ]

    ID = /\A[a-z0-9][a-z0-9-]{0,63}\z/

    class Invalid < Exception
    end

    record Rgba, r : Int32, g : Int32, b : Int32, a : Float64 do
      def hex : String
        Color.rgb_to_hex(r.to_f, g.to_f, b.to_f)
      end

      # Painted over an opaque colour (the desktop's `Color::over`, rounded half away from zero).
      def over(bg : Rgba) : Rgba
        m = ->(f : Int32, k : Int32) { Color.round(f * a + k * (1.0 - a)).to_i }
        Rgba.new(m.call(r, bg.r), m.call(g, bg.g), m.call(b, bg.b), 1.0)
      end
    end

    WHITE = Rgba.new(255, 255, 255, 1.0)

    # "#rgb", "#rrggbb", "#rrggbbaa", "rgb(r,g,b)" or "rgba(r,g,b,a)".
    def self.parse(css : String) : Rgba?
      s = css.strip
      if s.starts_with?('#')
        h = s[1..]
        return nil unless h.each_char.all?(&.hex?)
        h = h.each_char.map { |c| "#{c}#{c}" }.join if h.size == 3
        return nil unless h.size == 6 || h.size == 8
        a = h.size == 8 ? h[6, 2].to_i(16) / 255.0 : 1.0
        return Rgba.new(h[0, 2].to_i(16), h[2, 2].to_i(16), h[4, 2].to_i(16), a)
      end
      return nil unless s.starts_with?("rgb") && s.ends_with?(')') && (o = s.index('('))
      p = s[(o + 1)...-1].split(',').map(&.strip)
      return nil unless p.size == 3 || p.size == 4
      ch = p[0, 3].map { |x| x.to_i? }
      return nil if ch.any? { |x| x.nil? || x < 0 || x > 255 }
      a = p.size == 4 ? (p[3].to_f? || return nil) : 1.0
      return nil unless 0.0 <= a <= 1.0
      Rgba.new(ch[0].not_nil!, ch[1].not_nil!, ch[2].not_nil!, a)
    end

    # The desktop's WCAG luminance (its threshold is 0.03928, the WCAG 2.0 text).
    def self.lum(c : Rgba) : Float64
      l = ->(v : Int32) { x = v / 255.0; x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4 }
      0.2126 * l.call(c.r) + 0.7152 * l.call(c.g) + 0.0722 * l.call(c.b)
    end

    def self.ratio(a : Rgba, b : Rgba) : Float64
      x = lum(a)
      y = lum(b)
      (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
    end

    # Readability problems of a token set; empty = the desktop accepts it.
    def self.check(tokens : Hash(String, String)) : Array(String)
      errors = [] of String
      parsed = {} of String => Rgba
      TOKENS.each do |n|
        if v = tokens[n]?
          if c = parse(v)
            parsed[n] = c
          else
            errors << "#{n}: not a color (#{v})"
          end
        else
          errors << "#{n}: missing"
        end
      end
      surface = parsed["surface"]?.try(&.over(WHITE)) || WHITE
      RULES.each do |(fg, bg, min)|
        f = parsed[fg]?
        b = parsed[bg]?
        next unless f && b
        b = b.over(surface)
        r = ratio(f.over(b), b)
        errors << "#{fg}/#{bg}: contrast #{r.round(2)} < #{min}" if r < min
      end
      errors
    end

    private def self.mix(a : String, b : String, t : Float64) : String
      x = Color.hex_to_rgb(a)
      y = Color.hex_to_rgb(b)
      Color.rgb_to_hex(x.r + (y.r - x.r) * t, x.g + (y.g - x.g) * t, x.b + (y.b - x.b) * t)
    end

    private def self.rgba(hex : String, a : Float64) : String
      c = Color.hex_to_rgb(hex)
      "rgba(#{c.r.to_i},#{c.g.to_i},#{c.b.to_i},#{a})"
    end

    private def self.con(a : String, b : String) : Float64
      ratio(parse(a).not_nil!, parse(b).not_nil!)
    end

    # `c` moved toward black or white (whichever reads better on `bgs`) until it reaches `min`
    # on every one of them.
    private def self.readable(c : String, bgs : Array(String), min : Float64, away : String) : String
      worst = ->(x : String) { bgs.min_of { |b| con(x, b) } }
      return c if worst.call(c) >= min
      (1..20).each do |i|
        x = mix(c, away, i / 20.0)
        return x if worst.call(x) >= min
      end
      away
    end

    # 13 tokens from 16 colours: surfaces from the background, text from the foreground, the
    # accent from blue (4), status colours from green/yellow/red (2/3/1).
    #
    # A background too mid-toned to carry readable text is moved toward black (dark) or white
    # (light) in 10% steps until every rule holds; pure black/white always does.
    def self.tokens(p : Array(String), dark : Bool, accent : String? = nil) : Hash(String, String)
      ink = dark ? "#ffffff" : "#000000"
      paper = dark ? "#000000" : "#ffffff"
      err = [] of String
      11.times do |step|
        t = tokens_on(mix(p[0].downcase, paper, step / 10.0), p, dark, accent, ink)
        err = check(t)
        return t if err.empty?
      end
      raise Invalid.new("generated tokens fail the contrast contract: #{err.join("; ")}")
    end

    private def self.tokens_on(bg : String, p : Array(String), dark : Bool, accent : String?, ink : String) : Hash(String, String)
      fg = p[7].downcase
      card = dark ? mix(bg, fg, 0.07) : mix(bg, "#ffffff", 0.6)
      side = dark ? mix(bg, "#000000", 0.12) : mix(bg, fg, 0.03)
      sbar = parse(rgba(side, 0.92)).not_nil!.over(parse(bg).not_nil!).hex
      text = readable(fg, [bg, card, side, sbar], 4.6, ink)
      dim = readable(mix(text, bg, 0.35), [bg, card], 3.2, ink)
      acc = (accent || p[4]).downcase
      on = con("#ffffff", acc) >= con("#000000", acc) ? "#ffffff" : "#000000"
      away = on == "#ffffff" ? "#000000" : "#ffffff"
      acc = (0..20).map { |i| mix(acc, away, i / 20.0) }.find { |x| con(on, x) >= 3.2 } || away
      {
        "surface"    => bg,
        "sidebar"    => rgba(side, 0.92),
        "card"       => card,
        "line"       => rgba(text, 0.10),
        "text"       => text,
        "dim"        => dim,
        "accent"     => acc,
        "accentText" => on,
        "glass"      => rgba(bg, 0.84),
        "control"    => mix(bg, text, 0.12),
        "success"    => p[2].downcase,
        "warning"    => p[3].downcase,
        "danger"     => p[1].downcase,
      }
    end

    def self.json(id : String, name : String, p : Array(String), dark : Bool, wallpaper : String?,
                  accent : String? = nil) : String
      JSON.build(2) do |j|
        j.object do
          j.field "id", id
          j.field "name", name
          j.field "variant", dark ? "dark" : "light"
          j.field "tokens" do
            j.object { tokens(p, dark, accent).each { |k, v| j.field k, v } }
          end
          j.field "terminal", p.map(&.downcase)
          j.field "wallpaper", wallpaper if wallpaper
        end
      end + "\n"
    end

    def self.config_home : String
      x = ENV["XDG_CONFIG_HOME"]?
      x && !x.empty? ? x : File.join(ENV["HOME"]? || "/", ".config")
    end

    # $XDG_CONFIG_HOME/wolos/themes/<id> (shared::theme::USER_THEMES_DIR).
    def self.dir(id : String) : String
      raise Invalid.new("theme id “#{id}”: use a-z, 0-9 and -, up to 64 characters") unless id.matches?(ID)
      File.join(config_home, "wolos", "themes", id)
    end

    # Writes `text` to `path` through a temp file in the same folder and a rename.
    def self.atomic_write(path : String, text : String) : Nil
      Dir.mkdir_p(File.dirname(path))
      tmp = "#{path}.tmp.#{Process.pid}"
      File.write(tmp, text)
      File.rename(tmp, path)
    rescue e
      File.delete?(tmp) if tmp
      raise e
    end

    # A theme id from a name: lower case, runs of other characters become one "-".
    def self.slug(s : String) : String
      x = s.downcase.gsub(/[^a-z0-9]+/, "-").strip('-')
      x = x[0, 50].rstrip('-')
      x.empty? ? "picture" : x
    end
  end
end
