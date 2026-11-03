# SPDX-License-Identifier: BSD-2-Clause
# Pictures written from code for the specs (no binary fixtures in git): JPEG and PNG files with a
# camera block of chosen fields, in either byte order, with or without a stored preview.
module Fixtures
  # One directory entry: tag, type, and the values already encoded (in the file's byte order).
  record Field, tag : UInt16, type : UInt16, count : UInt32, data : Bytes

  class TiffWriter
    def initialize(@little : Bool = true)
    end

    def u16(v : Int) : Bytes
      b = Bytes.new(2)
      v = v.to_u16
      @little ? (b[0] = (v & 0xff).to_u8; b[1] = (v >> 8).to_u8) : (b[0] = (v >> 8).to_u8; b[1] = (v & 0xff).to_u8)
      b
    end

    def u32(v : Int) : Bytes
      b = Bytes.new(4)
      v = v.to_u32
      4.times do |i|
        shift = @little ? 8 * i : 8 * (3 - i)
        b[i] = ((v >> shift) & 0xff).to_u8
      end
      b
    end

    def ascii(tag : Int, text : String) : Field
      data = Fixtures.cat(text.to_slice, Bytes[0])
      Field.new(tag.to_u16, 2_u16, data.size.to_u32, data)
    end

    def short(tag : Int, v : Int) : Field
      Field.new(tag.to_u16, 3_u16, 1_u32, u16(v))
    end

    def long(tag : Int, v : Int) : Field
      Field.new(tag.to_u16, 4_u16, 1_u32, u32(v))
    end

    def rationals(tag : Int, values : Array({Int32, Int32})) : Field
      io = IO::Memory.new
      values.each { |(n, d)| io.write(u32(n)); io.write(u32(d)) }
      Field.new(tag.to_u16, 5_u16, values.size.to_u32, io.to_slice)
    end

    # A TIFF block: IFD0 (with pointers to the Exif and GPS directories when given) and IFD1 with
    # the preview when given.
    def build(zero : Array(Field), exif : Array(Field) = [] of Field, gps : Array(Field) = [] of Field, preview : Bytes? = nil) : Bytes
      # Layout: header 8, then the directories one after the other, then all out-of-line data.
      ifd0 = zero.dup
      ifd0 << long(0x8769, 0) unless exif.empty?
      ifd0 << long(0x8825, 0) unless gps.empty?
      ifd1 = preview ? [long(0x0201, 0), long(0x0202, preview.size)] : [] of Field
      sizes = [ifd0, exif, gps, ifd1].map { |d| d.empty? ? 0 : 2 + 12 * d.size + 4 }
      o0 = 8
      oe = o0 + sizes[0]
      og = oe + sizes[1]
      o1 = og + sizes[2]
      data_at = o1 + sizes[3]
      # Fix the pointers.
      ifd0 = ifd0.map do |f|
        case f.tag
        when 0x8769 then long(0x8769, oe)
        when 0x8825 then long(0x8825, og)
        else             f
        end
      end
      blobs = IO::Memory.new
      dirs = IO::Memory.new
      write_dir = ->(fields : Array(Field), nxt : Int32) do
        return if fields.empty?
        dirs.write(u16(fields.size))
        fields.sort_by(&.tag).each do |f|
          dirs.write(u16(f.tag))
          dirs.write(u16(f.type))
          dirs.write(u32(f.count))
          if f.data.size <= 4
            pad = Bytes.new(4)
            f.data.copy_to(pad)
            dirs.write(pad)
          else
            dirs.write(u32(data_at + blobs.size))
            blobs.write(f.data)
            blobs.write_byte(0_u8) if f.data.size.odd?
          end
        end
        dirs.write(u32(nxt))
      end
      write_dir.call(ifd0, preview ? o1 : 0)
      write_dir.call(exif, 0)
      write_dir.call(gps, 0)
      if preview
        start = data_at + blobs.size
        ifd1 = [long(0x0201, start), long(0x0202, preview.size)]
        blobs.write(preview)
        write_dir.call(ifd1, 0)
      end
      io = IO::Memory.new
      io.write(@little ? "II".to_slice : "MM".to_slice)
      io.write(u16(42))
      io.write(u32(8))
      io.write(dirs.to_slice)
      io.write(blobs.to_slice)
      io.to_slice
    end
  end

  # A small stand-in for a camera preview: a JPEG start, some bytes, a JPEG end.
  PREVIEW = Bytes[0xFF, 0xD8, 0xFF, 0xDB, 1, 2, 3, 4, 5, 6, 7, 8, 0xFF, 0xD9]

  # A camera block like a phone writes: date, place, camera, exposure, a preview.
  def self.camera_tiff(little = true, date = "2026:10:31 18:04:05", lat = {41, 23, 2400}, lon = {2, 10, 1200},
                       south = false, west = false, preview : Bytes? = PREVIEW, orientation = 1) : Bytes
    t = TiffWriter.new(little)
    zero = [t.ascii(0x010F, "Wolcam"), t.ascii(0x0110, "Wolcam One"), t.short(0x0112, orientation), t.ascii(0x0132, "2026:11:01 09:00:00")]
    exif = [
      t.ascii(0x9003, date),
      t.rationals(0x829A, [{1, 120}]),
      t.rationals(0x829D, [{18, 10}]),
      t.short(0x8827, 100),
      t.rationals(0x920A, [{42, 10}]),
      t.long(0xA002, 4032),
      t.long(0xA003, 3024),
    ]
    gps = lat ? [
      t.ascii(1, south ? "S" : "N"),
      t.rationals(2, [{lat[0], 1}, {lat[1], 1}, {lat[2], 100}]),
      t.ascii(3, west ? "W" : "E"),
      t.rationals(4, [{lon[0], 1}, {lon[1], 1}, {lon[2], 100}]),
    ] : [] of Field
    t.build(zero, exif, gps, preview)
  end

  # A JPEG: start, the camera block in APP1 when given, a frame header of width x height, the scan.
  def self.jpeg(width = 640, height = 480, tiff : Bytes? = nil) : Bytes
    io = IO::Memory.new
    io.write(Bytes[0xFF, 0xD8])
    io.write(cat(Bytes[0xFF, 0xE0, 0x00, 0x10], "JFIF\0".to_slice, Bytes[1, 1, 0, 0, 1, 0, 1, 0, 0]))
    if tiff
      body = cat("Exif\0\0".to_slice, tiff)
      len = body.size + 2
      io.write(Bytes[0xFF, 0xE1, (len >> 8).to_u8, (len & 0xff).to_u8])
      io.write(body)
    end
    io.write(Bytes[0xFF, 0xC0, 0x00, 0x11, 0x08, (height >> 8).to_u8, (height & 0xff).to_u8, (width >> 8).to_u8, (width & 0xff).to_u8, 3,
      1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1])
    io.write(Bytes[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0, 0x12, 0x34])
    io.write(Bytes[0xFF, 0xD9])
    io.to_slice
  end

  # A PNG of width x height (header only, with an eXIf chunk when given).
  def self.png(width = 32, height = 16, tiff : Bytes? = nil) : Bytes
    io = IO::Memory.new
    io.write(Bytes[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    chunk(io, "IHDR", cat(be32(width), be32(height), Bytes[8, 6, 0, 0, 0]))
    chunk(io, "eXIf", tiff) if tiff
    chunk(io, "IDAT", Bytes[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01])
    chunk(io, "IEND", Bytes.new(0))
    io.to_slice
  end

  def self.cat(*parts : Bytes) : Bytes
    io = IO::Memory.new
    parts.each { |p| io.write(p) }
    io.to_slice
  end

  private def self.chunk(io : IO, kind : String, data : Bytes) : Nil
    io.write(be32(data.size))
    io.write(kind.to_slice)
    io.write(data)
    io.write(Bytes[0, 0, 0, 0]) # the checksum is not read
  end

  private def self.be32(v : Int) : Bytes
    Bytes[((v >> 24) & 0xff).to_u8, ((v >> 16) & 0xff).to_u8, ((v >> 8) & 0xff).to_u8, (v & 0xff).to_u8]
  end
end
