# SPDX-License-Identifier: BSD-2-Clause
# A minimal PNG writer for `Session#screenshot`: 8-bit RGBA, one IDAT, no interlace. The standard
# library has the zlib deflate and CRC32 it needs, so the harness has no image dependency.
require "compress/zlib"
require "digest/crc32"

module Wolkit
  module Test
    module PNG
      SIGNATURE = Bytes[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

      # `pixels` is RGBA, `width * height * 4` bytes.
      def self.encode(pixels : Bytes, width : Int32, height : Int32) : Bytes
        raise ArgumentError.new("a #{width}x#{height} image needs #{width * height * 4} bytes, not #{pixels.size}") if pixels.size < width * height * 4
        file = IO::Memory.new
        file.write(SIGNATURE)
        header = IO::Memory.new
        header.write_bytes(width.to_u32, IO::ByteFormat::BigEndian)
        header.write_bytes(height.to_u32, IO::ByteFormat::BigEndian)
        header.write(Bytes[8, 6, 0, 0, 0])
        chunk(file, "IHDR", header.to_slice)
        raw = IO::Memory.new
        height.times do |row|
          raw.write_byte(0_u8)
          raw.write(pixels[row * width * 4, width * 4])
        end
        packed = IO::Memory.new
        Compress::Zlib::Writer.open(packed, Compress::Deflate::BEST_SPEED) { |zlib| zlib.write(raw.to_slice) }
        chunk(file, "IDAT", packed.to_slice)
        chunk(file, "IEND", Bytes.empty)
        file.to_slice
      end

      private def self.chunk(file : IO, kind : String, data : Bytes) : Nil
        file.write_bytes(data.size.to_u32, IO::ByteFormat::BigEndian)
        file.write(kind.to_slice)
        file.write(data)
        crc = Digest::CRC32.initial
        crc = Digest::CRC32.update(kind.to_slice, crc)
        crc = Digest::CRC32.update(data, crc)
        file.write_bytes(crc ^ 0xffff_ffff_u32, IO::ByteFormat::BigEndian)
      end
    end
  end
end
