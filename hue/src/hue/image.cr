# SPDX-License-Identifier: GPL-2.0-or-later
# Picture decoding through gdk-pixbuf (PNG, JPEG, WebP, AVIF: whatever loaders the desktop has)
# and the Catmull-Rom downscale the palette engine samples from.
module Hue
  @[Link("gdk-pixbuf-2.0")]
  lib LibPixbuf
    type Pixbuf = Void*

    struct GError
      domain : UInt32
      code : Int32
      message : UInt8*
    end

    fun get_file_info = gdk_pixbuf_get_file_info(filename : UInt8*, width : Int32*, height : Int32*) : Void*
    fun new_from_file = gdk_pixbuf_new_from_file(filename : UInt8*, error : GError**) : Pixbuf
    fun new_from_file_at_scale = gdk_pixbuf_new_from_file_at_scale(filename : UInt8*, width : Int32, height : Int32,
                                                                   preserve_aspect_ratio : Int32, error : GError**) : Pixbuf
    fun new = gdk_pixbuf_new(colorspace : Int32, has_alpha : Int32, bits : Int32, width : Int32, height : Int32) : Pixbuf
    fun scale_simple = gdk_pixbuf_scale_simple(src : Pixbuf, width : Int32, height : Int32, interp : Int32) : Pixbuf
    fun width = gdk_pixbuf_get_width(p : Pixbuf) : Int32
    fun height = gdk_pixbuf_get_height(p : Pixbuf) : Int32
    fun rowstride = gdk_pixbuf_get_rowstride(p : Pixbuf) : Int32
    fun n_channels = gdk_pixbuf_get_n_channels(p : Pixbuf) : Int32
    fun has_alpha = gdk_pixbuf_get_has_alpha(p : Pixbuf) : Int32
    fun bits_per_sample = gdk_pixbuf_get_bits_per_sample(p : Pixbuf) : Int32
    fun read_pixels = gdk_pixbuf_read_pixels(p : Pixbuf) : UInt8*
    fun pixels = gdk_pixbuf_get_pixels(p : Pixbuf) : UInt8*
    fun save = gdk_pixbuf_save(p : Pixbuf, filename : UInt8*, type : UInt8*, error : GError**, ...) : Int32
    fun unref = g_object_unref(p : Void*)
    fun error_free = g_error_free(e : GError*)
  end

  class ImageError < Exception
  end

  # An 8-bit, non-premultiplied RGBA picture in memory.
  class Picture
    getter width : Int32
    getter height : Int32
    getter rgba : Bytes

    def initialize(@width, @height, @rgba)
    end

    # Pictures above this are refused (a decompression bomb, not a wallpaper).
    MAX_PIXELS = 300_000_000_i64
    # Above this many pixels the decoder downscales while it reads (a JPEG decodes at 1/2..1/8
    # size); the palette then comes from a 1600 px picture instead of the full one. 24 MP covers
    # every 5K wallpaper at full size; a 50 MP photo decodes in a fraction of the time.
    PREVIEW_PIXELS = 24_000_000_i64
    PREVIEW_SIDE   =           1600

    def self.load(path : String) : Picture
      raise ImageError.new("#{path}: no such file") unless File.file?(path)
      w = 0
      h = 0
      fmt = LibPixbuf.get_file_info(path, pointerof(w), pointerof(h))
      raise ImageError.new("#{path}: not a picture this system can read") if fmt.null? || w <= 0 || h <= 0
      px = w.to_i64 * h
      raise ImageError.new("#{path}: #{w}x#{h} is too large") if px > MAX_PIXELS
      err = Pointer(LibPixbuf::GError).null
      pb = if px > PREVIEW_PIXELS
             sw, sh = w >= h ? {PREVIEW_SIDE, -1} : {-1, PREVIEW_SIDE}
             LibPixbuf.new_from_file_at_scale(path, sw, sh, 1, pointerof(err))
           else
             LibPixbuf.new_from_file(path, pointerof(err))
           end
      if pb.null?
        msg = err.null? ? "unreadable" : String.new(err.value.message)
        LibPixbuf.error_free(err) unless err.null?
        raise ImageError.new("#{path}: #{msg}")
      end
      begin
        from_pixbuf(pb)
      ensure
        LibPixbuf.unref(pb.as(Void*))
      end
    end

    def self.from_pixbuf(pb : LibPixbuf::Pixbuf) : Picture
      w = LibPixbuf.width(pb)
      h = LibPixbuf.height(pb)
      n = LibPixbuf.n_channels(pb)
      stride = LibPixbuf.rowstride(pb)
      raise ImageError.new("unsupported pixel format") unless LibPixbuf.bits_per_sample(pb) == 8 && (n == 3 || n == 4)
      src = LibPixbuf.read_pixels(pb)
      res = Bytes.new(w.to_i64 * h * 4)
      h.times do |y|
        row = src + y.to_i64 * stride
        o = y.to_i64 * w * 4
        if n == 4
          (res.to_unsafe + o).copy_from(row, w * 4)
        else
          w.times do |x|
            res[o + x * 4] = row[x * 3]
            res[o + x * 4 + 1] = row[x * 3 + 1]
            res[o + x * 4 + 2] = row[x * 3 + 2]
            res[o + x * 4 + 3] = 255_u8
          end
        end
      end
      Picture.new(w, h, res)
    end

    # Writes the picture as PNG or JPEG (test fixtures and the benchmark picture).
    def save(path : String, type : String = "png") : Nil
      pb = LibPixbuf.new(0, 1, 8, @width, @height)
      raise ImageError.new("out of memory") if pb.null?
      begin
        dst = LibPixbuf.pixels(pb)
        stride = LibPixbuf.rowstride(pb)
        @height.times { |y| (dst + y.to_i64 * stride).copy_from(@rgba.to_unsafe + y.to_i64 * @width * 4, @width * 4) }
        target = pb
        if type == "jpeg"
          # JPEG has no alpha: drop it by an identity scale into an RGB copy.
          target = LibPixbuf.new(0, 0, 8, @width, @height)
          d = LibPixbuf.pixels(target)
          ts = LibPixbuf.rowstride(target)
          @height.times do |y|
            @width.times do |x|
              s = y.to_i64 * @width * 4 + x * 4
              q = d + y.to_i64 * ts + x * 3
              q[0] = @rgba[s]
              q[1] = @rgba[s + 1]
              q[2] = @rgba[s + 2]
            end
          end
        end
        err = Pointer(LibPixbuf::GError).null
        ok = type == "jpeg" ? LibPixbuf.save(target, path, type, pointerof(err), "quality".to_unsafe, "90".to_unsafe, Pointer(UInt8).null) : LibPixbuf.save(target, path, type, pointerof(err), Pointer(UInt8).null)
        LibPixbuf.unref(target.as(Void*)) unless target == pb
        if ok == 0
          msg = err.null? ? "write failed" : String.new(err.value.message)
          LibPixbuf.error_free(err) unless err.null?
          raise ImageError.new("#{path}: #{msg}")
        end
      ensure
        LibPixbuf.unref(pb.as(Void*))
      end
    end

    # Catmull-Rom (B=0, C=0.5) two-pass downscale to fit `side` x `side`, aspect kept: a 4x4
    # cubic keeps small vivid features (a sign, a flag) that a box filter would average away.
    # The arithmetic follows the usual 16-bit premultiplied resampler (horizontal pass, then
    # vertical, overshoot clamped to alpha, then un-premultiplied), so results are stable to the bit.
    def fit(side : Int32) : Picture
      if @width >= @height
        dw = side
        dh = Math.max(1, Color.round(@height.to_f * side / @width).to_i)
      else
        dh = side
        dw = Math.max(1, Color.round(@width.to_f * side / @height).to_i)
      end
      xs = Picture.distrib(dw, @width)
      ys = Picture.distrib(dh, @height)
      tmp = Slice(Float64).new(@height.to_i64 * dw * 4)
      src = @rgba
      @height.times do |y|
        row = y.to_i64 * @width * 4
        trow = y.to_i64 * dw * 4
        dw.times do |dx|
          i, j, inv = xs[0][dx]
          pr = pg = pb = pa = 0.0
          (i...j).each do |k|
            coord, wt = xs[1][k]
            p = row + coord * 4
            a = src[p + 3].to_u32 * 0x101
            pr += (src[p].to_u32 * a // 0xff).to_f * wt
            pg += (src[p + 1].to_u32 * a // 0xff).to_f * wt
            pb += (src[p + 2].to_u32 * a // 0xff).to_f * wt
            pa += a.to_f * wt
          end
          f = inv / 0xffff
          t = trow + dx * 4
          tmp[t] = pr * f
          tmp[t + 1] = pg * f
          tmp[t + 2] = pb * f
          tmp[t + 3] = pa * f
        end
      end
      res = Bytes.new(dw.to_i64 * dh * 4)
      dh.times do |dy|
        i, j, inv = ys[0][dy]
        dw.times do |dx|
          pr = pg = pb = pa = 0.0
          (i...j).each do |k|
            coord, wt = ys[1][k]
            t = (coord.to_i64 * dw + dx) * 4
            pr += tmp[t] * wt
            pg += tmp[t + 1] * wt
            pb += tmp[t + 2] * wt
            pa += tmp[t + 3] * wt
          end
          pr = pa if pr > pa
          pg = pa if pg > pa
          pb = pa if pb > pa
          r = Picture.ftou(pr * inv)
          g = Picture.ftou(pg * inv)
          b = Picture.ftou(pb * inv)
          a = Picture.ftou(pa * inv)
          o = (dy.to_i64 * dw + dx) * 4
          if a == 0xffff
            res[o], res[o + 1], res[o + 2], res[o + 3] = (r >> 8).to_u8, (g >> 8).to_u8, (b >> 8).to_u8, 255_u8
          elsif a != 0
            res[o] = ((r * 0xffff // a) >> 8).to_u8
            res[o + 1] = ((g * 0xffff // a) >> 8).to_u8
            res[o + 2] = ((b * 0xffff // a) >> 8).to_u8
            res[o + 3] = (a >> 8).to_u8
          end
        end
      end
      Picture.new(dw, dh, res)
    end

    protected def self.ftou(f : Float64) : UInt32
      i = (0xffff * f + 0.5)
      return 0_u32 unless i > 0
      return 0xffff_u32 if i > 0xffff
      i.to_i32!.to_u32
    end

    private def self.kernel(t : Float64) : Float64
      return (1.5 * t - 2.5) * t * t + 1 if t < 1
      ((-0.5 * t + 2.5) * t - 4) * t + 2
    end

    # For every destination column: the source range [i, j) into the contribution list and the
    # inverse of its total weight; the list holds (source coordinate, weight).
    protected def self.distrib(dw : Int32, sw : Int32) : {Array({Int32, Int32, Float64}), Array({Int32, Float64})}
      scale = sw.to_f / dw
      half = 2.0
      arg = 1.0
      if scale > 1
        half *= scale
        arg = 1 / scale
      end
      sources = Array({Int32, Int32, Float64}).new(dw)
      contribs = [] of {Int32, Float64}
      dw.times do |x|
        center = (x + 0.5) * scale - 0.5
        i = Math.max(0, (center - half).floor.to_i)
        j = (center + half).ceil.to_i
        j = Math.max(i, sw) if j > sw
        l = contribs.size
        total = 0.0
        (i...j).each do |coord|
          t = ((center - coord) * arg).abs
          next if t >= 2.0
          w = kernel(t)
          next if w == 0
          total += w
          contribs << {coord, w}
        end
        sources << {l, contribs.size, 1 / total}
      end
      {sources, contribs}
    end
  end
end
