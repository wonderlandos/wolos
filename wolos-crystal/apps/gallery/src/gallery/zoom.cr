# SPDX-License-Identifier: GPL-2.0-or-later
# Zoom and pan of the viewer as numbers: a pure value, so the specs can check every edge and the view
# only turns it into a size and a position.
#
# Concept: the scale is screen pixels per picture pixel (1.0 is 100 %); the offset is where the
# picture's centre sits relative to the stage's centre. Why: zooming around the pointer or the stage
# centre and clamping the pan become two lines of arithmetic each. How: every change returns a new
# `Zoom`, clamped so the picture never leaves an empty band on a side it could cover.
module Gallery
  struct Zoom
    # The steps Zoom in and Zoom out move through, after "fit".
    STEPS = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0]
    MAX   = 8.0

    getter stage : {Float64, Float64}
    getter picture : {Float64, Float64}
    getter scale : Float64
    getter offset : {Float64, Float64}

    # A zoom that fits `picture` (width, height in pixels) into `stage` (logical pixels).
    def self.fit(stage : {Number, Number}, picture : {Number, Number}?) : Zoom
      s = {stage[0].to_f.clamp(1.0, nil), stage[1].to_f.clamp(1.0, nil)}
      p = picture.try { |q| {q[0].to_f.clamp(1.0, nil), q[1].to_f.clamp(1.0, nil)} } || s
      new(s, p, fit_scale(s, p), {0.0, 0.0})
    end

    # The scale at which the whole picture shows; small pictures are not blown up.
    def self.fit_scale(stage : {Float64, Float64}, picture : {Float64, Float64}) : Float64
      Math.min(1.0, Math.min(stage[0] / picture[0], stage[1] / picture[1]))
    end

    def initialize(@stage, @picture, @scale, @offset)
    end

    def fit_scale : Float64
      Zoom.fit_scale(@stage, @picture)
    end

    def fitted? : Bool
      (@scale - fit_scale).abs < 1e-6 && @offset == {0.0, 0.0}
    end

    def min : Float64
      fit_scale
    end

    # The next step up (around the stage centre, or `at`, a point relative to the stage's top left).
    def zoom_in(at : {Float64, Float64}? = nil) : Zoom
      target = STEPS.find { |s| s > @scale + 1e-6 } || MAX
      to(target, at)
    end

    # The next step down; below the smallest step, back to fit.
    def zoom_out(at : {Float64, Float64}? = nil) : Zoom
      target = STEPS.reverse.find { |s| s < @scale - 1e-6 } || min
      target = min if target < min
      return reset if target <= min + 1e-6
      to(target, at)
    end

    # Actual pixels (100 %), or back to fit when already there.
    def toggle_actual(at : {Float64, Float64}? = nil) : Zoom
      (@scale - 1.0).abs < 1e-6 ? reset : to(1.0, at)
    end

    def reset : Zoom
      Zoom.new(@stage, @picture, fit_scale, {0.0, 0.0})
    end

    # Zoom to `target`, keeping the picture point under `at` where it is.
    def to(target : Float64, at : {Float64, Float64}? = nil) : Zoom
      z = target.clamp(min, MAX)
      p = at ? {at[0] - @stage[0] / 2, at[1] - @stage[1] / 2} : {0.0, 0.0}
      k = z / @scale
      off = {p[0] - (p[0] - @offset[0]) * k, p[1] - (p[1] - @offset[1]) * k}
      Zoom.new(@stage, @picture, z, off).clamped
    end

    # Move the picture by `dx`, `dy` logical pixels.
    def pan(dx : Float64, dy : Float64) : Zoom
      Zoom.new(@stage, @picture, @scale, {@offset[0] + dx, @offset[1] + dy}).clamped
    end

    # Pan by a tenth of the stage (the arrow keys).
    def nudge(x : Int32, y : Int32) : Zoom
      pan(-x * @stage[0] / 10, -y * @stage[1] / 10)
    end

    # The same zoom in a stage of another size.
    def resize(stage : {Number, Number}) : Zoom
      s = {stage[0].to_f.clamp(1.0, nil), stage[1].to_f.clamp(1.0, nil)}
      was_fit = fitted?
      z = Zoom.new(s, @picture, @scale, @offset)
      was_fit ? z.reset : z.to(Math.max(@scale, z.min))
    end

    # Whether panning does anything (the picture is larger than the stage on some side).
    def pannable? : Bool
      width > @stage[0] + 0.5 || height > @stage[1] + 0.5
    end

    def width : Float64
      @picture[0] * @scale
    end

    def height : Float64
      @picture[1] * @scale
    end

    # The picture's top left corner in stage coordinates.
    def left : Float64
      (@stage[0] - width) / 2 + @offset[0]
    end

    def top : Float64
      (@stage[1] - height) / 2 + @offset[1]
    end

    # "Fit" or "150 %".
    def label : String
      fitted? ? "Fit" : "#{(@scale * 100).round.to_i} %"
    end

    protected def clamped : Zoom
      mx = Math.max(0.0, (width - @stage[0]) / 2)
      my = Math.max(0.0, (height - @stage[1]) / 2)
      Zoom.new(@stage, @picture, @scale, {@offset[0].clamp(-mx, mx), @offset[1].clamp(-my, my)})
    end
  end

  # Index arithmetic for the viewer, the filmstrip and the slideshow.
  module Step
    # `index` moved by `delta` in a list of `size`; `wrap` goes round (the slideshow), else it stops.
    def self.move(index : Int32, size : Int32, delta : Int32, wrap : Bool = false) : Int32
      return 0 if size <= 0
      n = index + delta
      wrap ? n % size : n.clamp(0, size - 1)
    end

    # The window of the filmstrip: up to `count` indices centred on `index`.
    def self.window(index : Int32, size : Int32, count : Int32) : Range(Int32, Int32)
      return 0...0 if size <= 0
      count = Math.min(count, size)
      start = (index - count // 2).clamp(0, size - count)
      start...(start + count)
    end
  end
end
