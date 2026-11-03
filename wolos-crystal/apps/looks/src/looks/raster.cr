# SPDX-License-Identifier: GPL-2.0-or-later
# Preview pictures drawn in code from a look's tokens: the mode cards (dark and light side by side),
# and a terminal, a Settings page, a document and a browser-like frame. Wolkit's style language
# takes theme tokens and no literal colours (WK-001), so a candidate that is not the active theme
# can only be shown as a picture; the pictures are PNG files the window shows as images.
#
# Concept: a Canvas of RGBA bytes with anti-aliased rounded rectangles; each preview is a few
# dozen shapes in the candidate's tokens. Why: the person must see the candidate on the kinds of
# surface they use, before the whole desktop changes. How: shapes are measured against a distance
# field (half a pixel of coverage at the edge), so no resampling and the output is the same on every
# machine; text is drawn as bars the way a wireframe does it, since real text would need the font.
require "./studio"

module Looks
  alias Rgba = Hue::Theme::Rgba

  class Canvas
    getter width : Int32
    getter height : Int32
    getter rgba : Bytes

    def initialize(@width : Int32, @height : Int32, fill : Rgba)
      @rgba = Bytes.new(@width * @height * 4)
      (@width * @height).times do |i|
        @rgba[i * 4] = fill.r.to_u8
        @rgba[i * 4 + 1] = fill.g.to_u8
        @rgba[i * 4 + 2] = fill.b.to_u8
        @rgba[i * 4 + 3] = 255_u8
      end
    end

    def picture : Hue::Picture
      Hue::Picture.new(@width, @height, @rgba)
    end

    # The colour of one pixel, for specs.
    def at(x : Int32, y : Int32) : {Int32, Int32, Int32}
      o = (y * @width + x) * 4
      {@rgba[o].to_i, @rgba[o + 1].to_i, @rgba[o + 2].to_i}
    end

    private def blend(x : Int32, y : Int32, c : Rgba, cover : Float64) : Nil
      return if x < 0 || y < 0 || x >= @width || y >= @height
      a = cover * c.a
      return if a <= 0.0
      o = (y * @width + x) * 4
      @rgba[o] = (@rgba[o] + (c.r - @rgba[o]) * a).round.to_u8
      @rgba[o + 1] = (@rgba[o + 1] + (c.g - @rgba[o + 1]) * a).round.to_u8
      @rgba[o + 2] = (@rgba[o + 2] + (c.b - @rgba[o + 2]) * a).round.to_u8
    end

    # A rounded rectangle; radius 0 is a plain one.
    def rect(x : Number, y : Number, w : Number, h : Number, c : Rgba, radius : Number = 0) : Nil
      x0 = x.to_f
      y0 = y.to_f
      w = w.to_f
      h = h.to_f
      r = Math.min(radius.to_f, Math.min(w, h) / 2)
      cx = x0 + w / 2
      cy = y0 + h / 2
      ix = w / 2 - r
      iy = h / 2 - r
      (y0.floor.to_i...(y0 + h).ceil.to_i).each do |py|
        (x0.floor.to_i...(x0 + w).ceil.to_i).each do |px|
          qx = (px + 0.5 - cx).abs - ix
          qy = (py + 0.5 - cy).abs - iy
          d = Math.sqrt(Math.max(qx, 0.0) ** 2 + Math.max(qy, 0.0) ** 2) + Math.min(Math.max(qx, qy), 0.0) - r
          cover = (0.5 - d).clamp(0.0, 1.0)
          blend(px, py, c, cover)
        end
      end
    end

    def circle(cx : Number, cy : Number, d : Number, c : Rgba) : Nil
      rect(cx.to_f - d / 2.0, cy.to_f - d / 2.0, d, d, c, d / 2.0)
    end
  end

  # Painting helpers over the tokens.
  module Raster
    CARD_W    = 128
    CARD_H    = 128
    PREVIEW_W = 152
    PREVIEW_H =  96

    def self.mix(a : Rgba, b : Rgba, t : Float64) : Rgba
      Rgba.new((a.r + (b.r - a.r) * t).round.to_i, (a.g + (b.g - a.g) * t).round.to_i, (a.b + (b.b - a.b) * t).round.to_i, 1.0)
    end

    def self.alpha(c : Rgba, a : Float64) : Rgba
      Rgba.new(c.r, c.g, c.b, a)
    end

    def self.hex(s : String) : Rgba
      Hue::Theme.parse!(s)
    end

    # The grey the preview shows for a candidate that cannot make a readable theme.
    UNREADABLE = Rgba.new(128, 128, 128, 1.0)

    # --- the mode card: dark panel, light panel

    # `dark` and `light` are the candidates of the mode in each variant (either may be unreadable).
    def self.card(dark : Candidate, light : Candidate) : Hue::Picture
      c = Canvas.new(CARD_W, CARD_H, UNREADABLE)
      panel(c, 0, CARD_W // 2, dark)
      panel(c, CARD_W // 2, CARD_W - CARD_W // 2, light)
      c.picture
    end

    private def self.panel(c : Canvas, x : Int32, w : Int32, cand : Candidate) : Nil
      t = cand.tokens
      unless t
        c.rect(x, 0, w, CARD_H, UNREADABLE)
        (0...CARD_H).step(8) { |y| c.rect(x, y, w, 2, mix(UNREADABLE, Rgba.new(255, 255, 255, 1.0), 0.3)) }
        return
      end
      pal = cand.final.map { |h| hex(h) }
      c.rect(x, 0, w, CARD_H, t.surface)
      # The 16 terminal colours: two rows of eight.
      16.times do |i|
        c.rect(x + 4 + (i % 8) * 7, 5 + (i // 8) * 7, 6, 6, pal[i], 1.5)
      end
      # A small window: card, text, a button and a pill, a switch, a slider and the status colours.
      c.rect(x + 3, 24, w - 6, 100, t.card, 5)
      c.rect(x + 8, 31, w - 18, 4, t.text, 2)
      c.rect(x + 8, 39, w - 24, 3, t.dim, 1.5)
      c.rect(x + 8, 45, w - 28, 3, t.dim, 1.5)
      c.rect(x + 8, 55, 24, 11, t.accent, 5.5)
      c.rect(x + 12, 59, 16, 3, t.accent_text, 1.5)
      c.rect(x + 35, 55, w - 44, 11, t.control, 5.5)
      c.rect(x + 8, 73, 18, 10, t.accent, 5)
      c.circle(x + 21, 78, 8, t.accent_text)
      c.rect(x + 8, 91, w - 22, 3, t.control, 1.5)
      c.rect(x + 8, 91, (w - 22) // 2, 3, t.accent, 1.5)
      c.circle(x + 8 + (w - 22) // 2, 92.5, 8, t.accent_text)
      c.circle(x + 13, 109, 8, t.success)
      c.circle(x + 27, 109, 8, t.warning)
      c.circle(x + 41, 109, 8, t.danger)
      c.rect(x + w - 19, 105, 13, 8, t.sidebar, 3)
    end

    # --- per-app previews

    private def self.window_frame(c : Canvas, t : Tokens, title_w : Int32 = 34) : Nil
      c.rect(0, 0, c.width, c.height, t.surface)
      c.rect(0, 0, c.width, 11, t.sidebar)
      c.circle(7, 5.5, 4, t.danger)
      c.circle(15, 5.5, 4, t.warning)
      c.circle(23, 5.5, 4, t.success)
      c.rect(c.width // 2 - title_w // 2, 4, title_w, 3, t.dim, 1.5)
      c.rect(0, 11, c.width, 1, t.line)
    end

    # A terminal: its background is colour 0, its text colour 7, the prompt green, the path blue.
    def self.terminal(cand : Candidate) : Hue::Picture
      t = cand.tokens
      return unreadable(PREVIEW_W, PREVIEW_H) unless t
      pal = cand.final.map { |h| hex(h) }
      c = Canvas.new(PREVIEW_W, PREVIEW_H, pal[0])
      c.rect(0, 0, c.width, 11, t.sidebar)
      c.circle(7, 5.5, 4, t.danger)
      c.circle(15, 5.5, 4, t.warning)
      c.circle(23, 5.5, 4, t.success)
      c.rect(c.width // 2 - 17, 4, 34, 3, t.dim, 1.5)
      y = 18
      # prompt, path, command
      line = ->(yy : Int32, segs : Array({Int32, Int32, Rgba})) do
        x = 8
        segs.each do |(w, gap, col)|
          c.rect(x, yy, w, 4, col, 2)
          x += w + gap
        end
      end
      line.call(y, [{14, 3, pal[2]}, {26, 3, pal[4]}, {10, 3, pal[7]}, {28, 0, pal[7]}])
      line.call(y + 9, [{30, 4, pal[4]}, {22, 4, pal[4]}, {26, 4, pal[6]}, {20, 4, pal[4]}, {18, 0, pal[5]}])
      line.call(y + 18, [{18, 4, pal[7]}, {40, 4, pal[3]}, {22, 0, pal[7]}])
      line.call(y + 27, [{14, 3, pal[1]}, {44, 3, pal[1]}, {16, 0, pal[9]}])
      line.call(y + 36, [{14, 3, pal[2]}, {26, 3, pal[4]}, {10, 3, pal[7]}])
      # the 16 colours as a strip of blocks
      16.times { |i| c.rect(8 + i * 8.5, PREVIEW_H - 16, 8, 8, pal[i]) }
      c.rect(8 + 5 * 8.5, y + 36, 6, 4, pal[7])
      c.picture
    end

    # A Settings page: a sidebar, a card of rows with a switch, a slider and a button.
    def self.settings(cand : Candidate) : Hue::Picture
      t = cand.tokens
      return unreadable(PREVIEW_W, PREVIEW_H) unless t
      c = Canvas.new(PREVIEW_W, PREVIEW_H, t.surface)
      window_frame(c, t)
      c.rect(0, 12, 44, PREVIEW_H - 12, t.sidebar)
      # sidebar rows, the second one selected
      4.times do |i|
        sel = i == 1
        c.rect(4, 17 + i * 12, 36, 9, mix(t.sidebar, t.accent, 0.28), 4) if sel
        c.rect(8, 20 + i * 12, 4, 4, sel ? t.accent : t.dim, 2)
        c.rect(15, 20.5 + i * 12, 20, 3, sel ? t.text : t.dim, 1.5)
      end
      # a card with three rows
      c.rect(52, 20, 92, 58, t.card, 6)
      3.times do |i|
        y = 24 + i * 18
        c.rect(58, y + 2, 34, 3, t.text, 1.5)
        c.rect(58, y + 8, 24, 2.5, t.dim, 1.25)
        c.rect(60, y + 17, 80, 1, t.line) if i < 2
      end
      # switch (on), slider, button
      c.rect(122, 26, 16, 9, t.accent, 4.5)
      c.circle(133.5, 30.5, 7, t.accent_text)
      c.rect(104, 49, 34, 3, t.control, 1.5)
      c.rect(104, 49, 20, 3, t.accent, 1.5)
      c.circle(124, 50.5, 8, t.accent_text)
      c.rect(104, 62, 34, 11, t.control, 5.5)
      c.rect(52, 82, 38, 9, t.accent, 4.5)
      c.rect(58, 85, 26, 3, t.accent_text, 1.5)
      c.picture
    end

    # A document: a heading, paragraphs, a link, a selection, a quote.
    def self.document(cand : Candidate) : Hue::Picture
      t = cand.tokens
      return unreadable(PREVIEW_W, PREVIEW_H) unless t
      c = Canvas.new(PREVIEW_W, PREVIEW_H, t.surface)
      window_frame(c, t, 26)
      c.rect(14, 20, 70, 7, t.text, 3)
      lines = [118, 124, 110, 120]
      lines.each_with_index { |w, i| c.rect(14, 34 + i * 8, w, 3, t.dim, 1.5) }
      # a selected phrase and a link
      c.rect(14, 33, 44, 5, mix(t.surface, t.accent, 0.35), 2)
      c.rect(14, 34.5, 44, 3, t.text, 1.5)
      c.rect(78, 58, 30, 3, t.accent, 1.5)
      c.rect(78, 61, 30, 1, t.accent)
      # a quote
      c.rect(14, 72, 2, 14, t.accent, 1)
      c.rect(20, 73, 90, 3, t.dim, 1.5)
      c.rect(20, 80, 64, 3, t.dim, 1.5)
      c.picture
    end

    # A browser-like frame: tabs, an address bar, a page with a banner and cards.
    def self.browser(cand : Candidate) : Hue::Picture
      t = cand.tokens
      return unreadable(PREVIEW_W, PREVIEW_H) unless t
      c = Canvas.new(PREVIEW_W, PREVIEW_H, t.surface)
      c.rect(0, 0, c.width, 22, t.glass)
      c.rect(0, 0, c.width, 22, t.sidebar)
      c.circle(7, 5, 4, t.danger)
      c.circle(15, 5, 4, t.warning)
      c.circle(23, 5, 4, t.success)
      # tabs: the first one is current
      c.rect(34, 3, 40, 8, t.surface, 3)
      c.rect(40, 5.5, 22, 3, t.text, 1.5)
      c.rect(78, 3, 40, 8, mix(t.sidebar, t.surface, 0.2), 3)
      c.rect(84, 5.5, 22, 3, t.dim, 1.5)
      # address bar
      c.rect(6, 13, c.width - 12, 8, t.control, 4)
      c.rect(12, 15.5, 50, 3, t.dim, 1.5)
      c.rect(0, 22, c.width, 1, t.line)
      # the page: a banner in the accent, then three cards
      c.rect(8, 28, c.width - 16, 24, t.accent, 5)
      c.rect(16, 34, 60, 5, t.accent_text, 2.5)
      c.rect(16, 43, 40, 3, t.accent_text, 1.5)
      3.times do |i|
        x = 8 + i * 46
        c.rect(x, 58, 42, 30, t.card, 5)
        c.rect(x + 5, 63, 22, 3, t.text, 1.5)
        c.rect(x + 5, 70, 30, 2.5, t.dim, 1.25)
        c.rect(x + 5, 76, 18, 2.5, t.dim, 1.25)
      end
      c.picture
    end

    def self.unreadable(w : Int32, h : Int32) : Hue::Picture
      c = Canvas.new(w, h, UNREADABLE)
      (0...h).step(8) { |y| c.rect(0, y, w, 2, mix(UNREADABLE, Rgba.new(255, 255, 255, 1.0), 0.3)) }
      c.picture
    end

    # A one-row strip of a look's colours, for lists: the 16 colours as blocks.
    def self.strip(colors : Array(String), w : Int32 = 96, h : Int32 = 12) : Hue::Picture
      c = Canvas.new(w, h, UNREADABLE)
      n = colors.size
      n.times { |i| c.rect(i * w / n.to_f, 0, w / n.to_f + 1, h, hex(colors[i])) }
      c.picture
    end
  end
end
