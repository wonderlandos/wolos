# SPDX-License-Identifier: GPL-2.0-or-later
# The words and numbers the gallery shows about a picture: dates, sizes, the camera line. Pure, so the
# info panel and the accessible names read the same in every test.
module Gallery
  module Format
    # "Saturday 31 October 2026 at 18:04".
    def self.long_date(t : Time) : String
      "#{Sections::WEEKDAYS[t.day_of_week.value % 7]} #{t.day} #{Sections::MONTHS[t.month - 1]} #{t.year} at #{t.to_s("%H:%M")}"
    end

    # "31 October 2026".
    def self.short_date(t : Time) : String
      "#{t.day} #{Sections::MONTHS[t.month - 1]} #{t.year}"
    end

    # "4032 × 3024".
    def self.pixels(size : {Int32, Int32}?) : String
      size ? "#{size[0]} × #{size[1]}" : "Unknown"
    end

    # "820 bytes", "14 KB", "3.2 MB" (powers of 1000, as the file manager counts).
    def self.bytes(n : Int64) : String
      return "#{n} bytes" if n < 1000
      return "#{(n / 1000.0).round.to_i} KB" if n < 1_000_000
      return "#{(n / 1_000_000.0).round(1)} MB" if n < 1_000_000_000
      "#{(n / 1_000_000_000.0).round(1)} GB"
    end

    # "1/120 s, f/1.8, ISO 100, 4.2 mm" from what the camera block has, or nil.
    def self.exposure(e : Exif) : String?
      parts = [e.exposure, e.aperture, e.iso.try { |i| "ISO #{i}" }, e.focal_length].compact
      parts.empty? ? nil : parts.join(", ")
    end

    # The accessible name of a tile: "IMG_0412.jpg, 31 October 2026".
    def self.tile_name(p : Photo) : String
      "#{p.name}, #{short_date(p.date)}"
    end

    # "3 of 12".
    def self.position(index : Int32, size : Int32) : String
      size <= 0 ? "" : "#{index + 1} of #{size}"
    end
  end
end
