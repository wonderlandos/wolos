# SPDX-License-Identifier: GPL-2.0-or-later
# A small reader for the camera block of a picture: when it was taken, where, which way up, and the
# small preview the camera stored next to it. Pure Crystal over a byte slice: no library, no pointer.
#
# Concept: a JPEG keeps this block in its APP1 segment and a PNG in an `eXIf` chunk; both hold the
# same tagged directory (a TIFF header, then directories of 12-byte entries). Why: the library groups
# pictures by date and place, and the stored preview is the cheapest thumbnail there is. How: every
# offset is checked against the slice before it is read, a directory is visited once, and anything
# malformed reads as "not there" (nil), never as an exception, because pictures come from anywhere.
module Gallery
  # A point on Earth in decimal degrees (north and east are positive).
  record GeoPoint, latitude : Float64, longitude : Float64 do
    def valid? : Bool
      latitude.finite? && longitude.finite? && latitude.abs <= 90.0 && longitude.abs <= 180.0 &&
        !(latitude == 0.0 && longitude == 0.0)
    end
  end

  # What the camera block says. Every field is optional.
  struct Exif
    getter taken : Time?
    getter place : GeoPoint?
    getter orientation : Int32
    getter camera : String?
    getter width : Int32?
    getter height : Int32?
    getter exposure : String?
    getter aperture : String?
    getter iso : Int32?
    getter focal_length : String?
    # Where the stored preview (a JPEG) sits in the slice the block was read from.
    getter preview : Range(Int32, Int32)?

    def initialize(@taken = nil, @place = nil, @orientation = 1, @camera = nil, @width = nil, @height = nil,
                   @exposure = nil, @aperture = nil, @iso = nil, @focal_length = nil, @preview = nil)
    end

    # Whether the picture is turned a quarter (orientations 5 to 8), so width and height swap on screen.
    def quarter_turn? : Bool
      (5..8).includes?(@orientation)
    end
  end

  # Reads an `Exif` from the bytes of a JPEG or PNG file (at least its first part).
  module ExifReader
    # Directories hold at most this many entries; a larger count is a damaged file.
    MAX_ENTRIES = 512

    TAG_ORIENTATION   = 0x0112_u16
    TAG_MAKE          = 0x010F_u16
    TAG_MODEL         = 0x0110_u16
    TAG_DATE_TIME     = 0x0132_u16
    TAG_EXIF_IFD      = 0x8769_u16
    TAG_GPS_IFD       = 0x8825_u16
    TAG_EXPOSURE      = 0x829A_u16
    TAG_F_NUMBER      = 0x829D_u16
    TAG_ISO           = 0x8827_u16
    TAG_ORIGINAL      = 0x9003_u16
    TAG_DIGITIZED     = 0x9004_u16
    TAG_FOCAL_LENGTH  = 0x920A_u16
    TAG_PIXEL_X       = 0xA002_u16
    TAG_PIXEL_Y       = 0xA003_u16
    TAG_PREVIEW_START = 0x0201_u16
    TAG_PREVIEW_SIZE  = 0x0202_u16
    GPS_LAT_REF       = 0x0001_u16
    GPS_LAT           = 0x0002_u16
    GPS_LON_REF       = 0x0003_u16
    GPS_LON           = 0x0004_u16

    # The camera block of `file` (JPEG or PNG bytes), or an empty `Exif`. `location` is the zone the
    # camera's clock is taken to be in (the block rarely says).
    def self.read(file : Bytes, location : Time::Location = Time::Location.local) : Exif
      if tiff = tiff_range(file)
        parse(file, tiff, location)
      else
        Exif.new
      end
    end

    # Where the TIFF-structured block starts and ends inside a JPEG (APP1 "Exif\0\0") or a PNG (eXIf).
    def self.tiff_range(file : Bytes) : Range(Int32, Int32)?
      if file.size >= 4 && file[0] == 0xFF && file[1] == 0xD8
        jpeg_tiff(file)
      elsif Picture.png?(file)
        png_tiff(file)
      end
    end

    private def self.jpeg_tiff(file : Bytes) : Range(Int32, Int32)?
      at = 2
      while at + 4 <= file.size
        return unless file[at] == 0xFF
        marker = file[at + 1]
        if marker == 0xFF # fill byte
          at += 1
          next
        end
        return if marker == 0xDA || marker == 0xD9 # image data or end: no block before it
        len = (file[at + 2].to_i << 8) | file[at + 3].to_i
        return if len < 2 || at + 2 + len > file.size
        body = at + 4
        if marker == 0xE1 && len >= 8 && file[body, 6] == "Exif\0\0".to_slice
          return (body + 6)...(at + 2 + len)
        end
        at += 2 + len
      end
      nil
    end

    private def self.png_tiff(file : Bytes) : Range(Int32, Int32)?
      at = 8
      while at + 12 <= file.size
        len = be32(file, at)
        return if len > file.size
        kind = file[at + 4, 4]
        body = at + 8
        return if body.to_i64 + len > file.size
        return body...(body + len.to_i) if kind == "eXIf".to_slice
        return if kind == "IDAT".to_slice || kind == "IEND".to_slice
        at = body + len.to_i + 4 # data, then the checksum
      end
      nil
    end

    # Parse the tagged directories in `file[tiff]`.
    def self.parse(file : Bytes, tiff : Range(Int32, Int32), location : Time::Location) : Exif
      t = Tiff.new(file[tiff.begin, tiff.end - tiff.begin])
      return Exif.new unless t.valid?
      ifd0 = t.u32(4)
      return Exif.new unless ifd0
      zero = t.entries(Tiff.off(ifd0))
      exif = zero[TAG_EXIF_IFD]?.try { |e| t.entries(Tiff.off(t.long(e))) } || {} of UInt16 => Tiff::Entry
      gps = zero[TAG_GPS_IFD]?.try { |e| t.entries(Tiff.off(t.long(e))) } || {} of UInt16 => Tiff::Entry
      first = t.next_ifd(Tiff.off(ifd0)).try { |n| t.entries(n) } || {} of UInt16 => Tiff::Entry

      # The first date that reads: when it was taken, when it was stored, when the file was written.
      dates = [exif[TAG_ORIGINAL]?, exif[TAG_DIGITIZED]?, zero[TAG_DATE_TIME]?].compact
      taken = nil
      dates.each do |e|
        taken = parse_date(t.ascii(e), location)
        break if taken
      end
      camera = [zero[TAG_MAKE]?, zero[TAG_MODEL]?].compact.map { |e| t.ascii(e).strip }.reject(&.empty?)
      camera_name = camera.size == 2 && camera[1].starts_with?(camera[0]) ? camera[1] : camera.join(" ")
      orientation = zero[TAG_ORIENTATION]?.try { |e| t.short(e) } || 1
      orientation = 1 unless (1..8).includes?(orientation)

      preview = nil
      if (s = first[TAG_PREVIEW_START]?) && (n = first[TAG_PREVIEW_SIZE]?)
        start = t.long(s).to_i64
        size = t.long(n).to_i64
        if size >= 2 && start + size <= t.size
          b = tiff.begin + start.to_i
          preview = b...(b + size.to_i) if file[b] == 0xFF && file[b + 1] == 0xD8
        end
      end

      Exif.new(
        taken: taken,
        place: place(t, gps),
        orientation: orientation,
        camera: camera_name.empty? ? nil : camera_name,
        width: exif[TAG_PIXEL_X]?.try { |e| dimension(t.long(e)) },
        height: exif[TAG_PIXEL_Y]?.try { |e| dimension(t.long(e)) },
        exposure: exif[TAG_EXPOSURE]?.try { |e| exposure(t.rational(e, 0)) },
        aperture: exif[TAG_F_NUMBER]?.try { |e| t.rational(e, 0).try { |f| "f/#{trim(f)}" } },
        iso: exif[TAG_ISO]?.try { |e| t.short(e) },
        focal_length: exif[TAG_FOCAL_LENGTH]?.try { |e| t.rational(e, 0).try { |f| "#{trim(f)} mm" } },
        preview: preview,
      )
    end

    # "2026:10:31 18:04:05" in `location`, or nil.
    def self.parse_date(text : String, location : Time::Location) : Time?
      m = text.match(/\A(\d{4}):(\d{2}):(\d{2})[ T](\d{2}):(\d{2}):(\d{2})/)
      return unless m
      y, mo, d, h, mi, s = (1..6).map { |i| m[i].to_i }
      return unless y >= 1900 && (1..12).includes?(mo) && (1..31).includes?(d) && h < 24 && mi < 60 && s < 61
      return if d > Time.days_in_month(y, mo)
      Time.local(y, mo, d, h, mi, Math.min(s, 59), location: location)
    end

    private def self.place(t : Tiff, gps : Hash(UInt16, Tiff::Entry)) : GeoPoint?
      lat = gps[GPS_LAT]?.try { |e| degrees(t, e) }
      lon = gps[GPS_LON]?.try { |e| degrees(t, e) }
      return unless lat && lon
      lat = -lat if gps[GPS_LAT_REF]?.try { |e| t.ascii(e).starts_with?('S') }
      lon = -lon if gps[GPS_LON_REF]?.try { |e| t.ascii(e).starts_with?('W') }
      point = GeoPoint.new(lat, lon)
      point.valid? ? point : nil
    end

    private def self.degrees(t : Tiff, e : Tiff::Entry) : Float64?
      d = t.rational(e, 0)
      m = t.rational(e, 1) || 0.0
      s = t.rational(e, 2) || 0.0
      d.try { |deg| deg + m / 60.0 + s / 3600.0 }
    end

    # A side in pixels, or nil when it is none or more than any picture has.
    private def self.dimension(v : UInt32) : Int32?
      v > 0 && v <= 1_000_000 ? v.to_i : nil
    end

    private def self.exposure(v : Float64?) : String?
      return unless v && v > 1e-6 && v < 1e6
      v >= 1.0 ? "#{trim(v)} s" : "1/#{(1.0 / v).round.to_i} s"
    end

    # One decimal at most: "1.8", "4", "42.5".
    private def self.trim(v : Float64) : String
      text = "%.1f" % v
      text.ends_with?(".0") ? text[0, text.size - 2] : text
    end

    private def self.be32(b : Bytes, at : Int32) : UInt32
      (b[at].to_u32 << 24) | (b[at + 1].to_u32 << 16) | (b[at + 2].to_u32 << 8) | b[at + 3].to_u32
    end

    # The TIFF structure: byte order, bounded reads, directories.
    struct Tiff
      record Entry, tag : UInt16, type : UInt16, count : UInt32, at : Int32

      # Bytes per value, by type (1 byte, 2 ascii, 3 short, 4 long, 5 rational, 7 undefined, 9 slong,
      # 10 srational).
      SIZES = {1 => 1, 2 => 1, 3 => 2, 4 => 4, 5 => 8, 7 => 1, 9 => 4, 10 => 8}

      getter size : Int32

      # An offset as an index, or -1 (never in range) when it is past what an index holds.
      def self.off(v : UInt32) : Int32
        v > Int32::MAX ? -1 : v.to_i
      end

      def initialize(@b : Bytes)
        @size = @b.size
        @little = @b.size >= 2 && @b[0] == 0x49 && @b[1] == 0x49
      end

      def valid? : Bool
        return false if @size < 8
        order = (@b[0] == 0x49 && @b[1] == 0x49) || (@b[0] == 0x4D && @b[1] == 0x4D)
        order && u16(2) == 42
      end

      def u16(at : Int) : UInt16?
        return if at < 0 || at + 2 > @size
        a, b = @b[at].to_u16, @b[at + 1].to_u16
        @little ? a | (b << 8) : (a << 8) | b
      end

      def u32(at : Int) : UInt32?
        return if at < 0 || at + 4 > @size
        w = (0..3).map { |i| @b[at + i].to_u32 }
        @little ? w[0] | (w[1] << 8) | (w[2] << 16) | (w[3] << 24) : (w[0] << 24) | (w[1] << 16) | (w[2] << 8) | w[3]
      end

      # The entries of the directory at `at`, by tag (the first of a repeated tag wins).
      def entries(at : Int32) : Hash(UInt16, Entry)
        found = {} of UInt16 => Entry
        n = u16(at)
        return found unless n && n <= MAX_ENTRIES
        n.times do |i|
          e = at + 2 + i * 12
          tag, type, count = u16(e), u16(e + 2), u32(e + 4)
          break unless tag && type && count
          next unless w = SIZES[type.to_i]?
          total = w.to_i64 * count
          next if total > @size
          data = total <= 4 ? e + 8 : (u32(e + 8) || next).to_i64
          next if data + total > @size
          found[tag] = Entry.new(tag, type, count, data.to_i) unless found.has_key?(tag)
        end
        found
      end

      # The offset of the directory after the one at `at`, or nil.
      def next_ifd(at : Int32) : Int32?
        n = u16(at)
        return unless n && n <= MAX_ENTRIES
        nxt = u32(at + 2 + n.to_i * 12)
        return unless nxt && nxt > 0 && nxt != at && nxt < @size
        nxt.to_i
      end

      def short(e : Entry) : Int32?
        case e.type
        when 3 then u16(e.at).try(&.to_i)
        when 4 then u32(e.at).try { |v| v > Int32::MAX ? nil : v.to_i }
        end
      end

      def long(e : Entry) : UInt32
        case e.type
        when 3 then u16(e.at).try(&.to_u32) || 0_u32
        when 4 then u32(e.at) || 0_u32
        else        0_u32
        end
      end

      def rational(e : Entry, index : Int32) : Float64?
        return unless (e.type == 5 || e.type == 10) && index < e.count
        num, den = u32(e.at + index * 8), u32(e.at + index * 8 + 4)
        return unless num && den && den != 0
        if e.type == 10
          num.to_i32!.to_f64 / den.to_i32!.to_f64
        else
          num.to_f64 / den.to_f64
        end
      end

      def ascii(e : Entry) : String
        return "" unless e.type == 2 || e.type == 7
        raw = @b[e.at, e.count.to_i]
        n = raw.index(0_u8) || raw.size
        String.new(raw[0, n]).scrub
      end
    end
  end
end
