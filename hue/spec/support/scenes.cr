# SPDX-License-Identifier: GPL-2.0-or-later
# Ten test pictures drawn from code (no binary fixtures in git): dark, light, grey, tinted, vivid
# and nearly empty scenes, one portrait, one with transparency. Deterministic to the byte.
require "../../src/hue/image"

module Scenes
  W = 480
  H = 270

  alias Px = {Float64, Float64, Float64, Float64}

  class Rng
    def initialize(@s : UInt32)
    end

    # -1..1
    def next : Float64
      @s = @s &* 1103515245_u32 &+ 12345_u32
      ((@s >> 16) & 0x7fff).to_f / 16383.5 - 1.0
    end
  end

  def self.lerp(a : Px, b : Px, t : Float64) : Px
    t = t.clamp(0.0, 1.0)
    {a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t}
  end

  def self.draw(w : Int32, h : Int32, seed : UInt32, noise : Float64, &f : Float64, Float64, Rng -> Px) : Hue::Picture
    rng = Rng.new(seed)
    res = Bytes.new(w * h * 4)
    h.times do |y|
      w.times do |x|
        c = f.call(x / w.to_f, y / h.to_f, rng)
        n = rng.next * noise
        o = (y * w + x) * 4
        3.times { |i| res[o + i] = (c[i] + n).round.clamp(0.0, 255.0).to_u8 }
        res[o + 3] = c[3].round.clamp(0.0, 255.0).to_u8
      end
    end
    Hue::Picture.new(w, h, res)
  end

  def self.disc?(x, y, cx, cy, r) : Bool
    ((x - cx) * 16 / 9) ** 2 + (y - cy) ** 2 < r * r
  end

  NAMES = %w(night-city sunset forest ocean desert grey-photo sepia pastel-portrait stripes snow-speck)

  def self.all : Hash(String, Hue::Picture)
    NAMES.to_h { |n| {n, scene(n)} }
  end

  def self.scene(name : String) : Hue::Picture
    case name
    when "night-city"
      draw(W, H, 1_u32, 4.0) do |x, y, r|
        c = lerp({8.0, 10.0, 30.0, 255.0}, {25.0, 35.0, 90.0, 255.0}, y)
        c = {250.0, 60.0, 170.0, 255.0} if ((x * 37).to_i % 7 == 0) && y > 0.55 && ((y * 41).to_i % 5 == 0)
        c = {40.0, 230.0, 240.0, 255.0} if ((x * 53).to_i % 11 == 0) && y > 0.6 && ((y * 29).to_i % 4 == 1)
        c
      end
    when "sunset"
      draw(W, H, 2_u32, 3.0) do |x, y, r|
        c = y < 0.5 ? lerp({90.0, 40.0, 120.0, 255.0}, {250.0, 120.0, 50.0, 255.0}, y * 2) : lerp({250.0, 170.0, 70.0, 255.0}, {120.0, 50.0, 60.0, 255.0}, (y - 0.5) * 2)
        c = {255.0, 230.0, 150.0, 255.0} if disc?(x, y, 0.5, 0.5, 0.08)
        c = {20.0, 12.0, 25.0, 255.0} if y > 0.8 - 0.05 * Math.sin(x * 12)
        c
      end
    when "forest"
      draw(W, H, 3_u32, 8.0) do |x, y, r|
        base = lerp({20.0, 60.0, 30.0, 255.0}, {70.0, 120.0, 50.0, 255.0}, (Math.sin(x * 40) + 1) / 2)
        base = lerp(base, {90.0, 60.0, 30.0, 255.0}, 0.7) if (x * 9).to_i.odd? && (x * 9 - (x * 9).floor) < 0.12
        y < 0.15 ? lerp({180.0, 200.0, 190.0, 255.0}, base, y / 0.15) : base
      end
    when "ocean"
      draw(W, H, 4_u32, 5.0) do |x, y, r|
        c = lerp({20.0, 120.0, 150.0, 255.0}, {5.0, 30.0, 70.0, 255.0}, y)
        c = {230.0, 245.0, 245.0, 255.0} if (Math.sin(x * 30 + y * 8) > 0.97)
        c
      end
    when "desert"
      draw(W, H, 5_u32, 4.0) do |x, y, r|
        y < 0.45 ? lerp({120.0, 180.0, 235.0, 255.0}, {200.0, 225.0, 245.0, 255.0}, y / 0.45) : lerp({235.0, 205.0, 150.0, 255.0}, {200.0, 150.0, 95.0, 255.0}, (y - 0.45) / 0.55 + 0.2 * Math.sin(x * 9))
      end
    when "grey-photo"
      draw(W, H, 6_u32, 10.0) do |x, y, r|
        v = 60 + 140 * (0.5 + 0.5 * Math.sin(x * 7) * Math.cos(y * 5))
        {v, v, v, 255.0}
      end
    when "sepia"
      draw(W, H, 7_u32, 6.0) do |x, y, r|
        v = 0.25 + 0.6 * (0.5 + 0.5 * Math.sin(x * 5 + y * 3))
        {v * 255, v * 215, v * 160, 255.0}
      end
    when "pastel-portrait"
      draw(H, W, 8_u32, 2.0) do |x, y, r|
        c = {250.0, 244.0, 230.0, 255.0}
        c = {245.0, 180.0, 190.0, 255.0} if disc?(x, y, 0.3, 0.3, 0.12)
        c = {170.0, 210.0, 240.0, 255.0} if disc?(x, y, 0.7, 0.55, 0.15)
        c = {190.0, 230.0, 190.0, 255.0} if disc?(x, y, 0.4, 0.8, 0.1)
        c = {c[0], c[1], c[2], 0.0} if y > 0.95 # transparent strip at the bottom
        c
      end
    when "stripes"
      draw(W, H, 9_u32, 3.0) do |x, y, r|
        k = (x * 8).to_i
        [{230.0, 50.0, 50.0}, {240.0, 160.0, 30.0}, {240.0, 220.0, 40.0}, {60.0, 190.0, 80.0},
         {40.0, 180.0, 200.0}, {50.0, 90.0, 220.0}, {150.0, 70.0, 200.0}, {30.0, 30.0, 35.0}][k].try { |c| {c[0], c[1], c[2], 255.0} }
      end
    else # snow-speck
      draw(W, H, 10_u32, 3.0) do |x, y, r|
        c = lerp({245.0, 247.0, 250.0, 255.0}, {215.0, 220.0, 228.0, 255.0}, y)
        c = {200.0, 30.0, 40.0, 255.0} if disc?(x, y, 0.8, 0.7, 0.02)
        c
      end
    end
  end
end
