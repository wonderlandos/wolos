# SPDX-License-Identifier: BSD-2-Clause
# What kind of picture a file is and how big, from its first bytes. The engine shows PNG and JPEG
# (libwolkit refuses other encoded formats), so those are the two the library lists.
module Gallery
  module Picture
    enum Format
      Jpeg
      Png
    end

    PNG_SIGNATURE = Bytes[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    EXTENSIONS    = {".jpg" => Format::Jpeg, ".jpeg" => Format::Jpeg, ".png" => Format::Png}

    def self.png?(b : Bytes) : Bool
      b.size >= 8 && b[0, 8] == PNG_SIGNATURE
    end

    def self.jpeg?(b : Bytes) : Bool
      b.size >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF
    end

    # The format the bytes are in, or nil.
    def self.sniff(b : Bytes) : Format?
      if jpeg?(b)
        Format::Jpeg
      elsif png?(b)
        Format::Png
      end
    end

    # Whether a file name looks like a picture the library lists (the bytes decide later).
    def self.listed?(name : String) : Bool
      !name.starts_with?('.') && EXTENSIONS.has_key?(File.extname(name).downcase)
    end

    # Width and height in pixels as stored (before any turn), or nil.
    def self.dimensions(b : Bytes) : {Int32, Int32}?
      if png?(b)
        return unless b.size >= 24 && b[12, 4] == "IHDR".to_slice
        w = be32(b, 16)
        h = be32(b, 20)
        return w > 0 && h > 0 ? {w, h} : nil
      end
      return unless jpeg?(b)
      at = 2
      while at + 9 <= b.size
        return unless b[at] == 0xFF
        marker = b[at + 1]
        if marker == 0xFF
          at += 1
          next
        end
        return if marker == 0xD9 || marker == 0xDA
        len = (b[at + 2].to_i << 8) | b[at + 3].to_i
        return if len < 2
        # Start of frame: every 0xC0..0xCF except the huffman (C4), arithmetic (CC) and JPG (C8) markers.
        if 0xC0 <= marker <= 0xCF && marker != 0xC4 && marker != 0xC8 && marker != 0xCC
          h = (b[at + 5].to_i << 8) | b[at + 6].to_i
          w = (b[at + 7].to_i << 8) | b[at + 8].to_i
          return w > 0 && h > 0 ? {w, h} : nil
        end
        at += 2 + len
      end
      nil
    end

    private def self.be32(b : Bytes, at : Int32) : Int32
      ((b[at].to_u32 << 24) | (b[at + 1].to_u32 << 16) | (b[at + 2].to_u32 << 8) | b[at + 3].to_u32).to_i32!
    end
  end
end
