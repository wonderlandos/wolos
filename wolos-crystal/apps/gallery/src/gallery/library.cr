# SPDX-License-Identifier: BSD-2-Clause
# The library scan: every PNG and JPEG under the Pictures folder, with its camera block and its
# thumbnail. It runs on the background pool (never on the UI context) and returns one value.
#
# Concept: a bounded walk. Why: a Pictures folder can hold a backup of a whole phone, links that
# loop, and files that are not pictures at all. How: hidden folders and linked folders are skipped,
# the walk stops at `MAX_DEPTH` levels and `MAX_FILES` pictures (the result says it stopped), only
# the first `HEAD` bytes of each file are read (the camera block and its preview sit there), and a
# file whose bytes are not a picture is counted, not listed.
module Gallery
  # Why there is no library to show: one sentence for the person.
  class ScanError < Exception
  end

  record ScanResult, photos : Array(Photo), skipped : Int32, truncated : Bool, folder : String, missing : Bool = false

  class Library
    MAX_DEPTH =      8
    MAX_FILES = 20_000
    HEAD      = 192 * 1024

    getter folder : String

    # `depth`: how many folder levels below `folder` are read (0: only the folder itself).
    def initialize(@folder : String, @thumbs : Thumbs?, @location : Time::Location = Time::Location.local,
                   @depth : Int32 = MAX_DEPTH)
    end

    def scan : ScanResult
      unless Dir.exists?(@folder)
        return ScanResult.new([] of Photo, 0, false, @folder, missing: true)
      end
      photos = [] of Photo
      skipped = 0
      truncated = false
      walk(@folder, 0) do |path|
        if photos.size >= MAX_FILES
          truncated = true
          next false
        end
        if photo = read(path)
          photos << photo
        else
          skipped += 1
        end
        true
      end
      @thumbs.try { |t| t.prune(photos.map { |p| t.key(p.path, p.bytes, p.modified) }.to_set) }
      ScanResult.new(photos, skipped, truncated, @folder)
    rescue File::AccessDeniedError
      raise ScanError.new("The gallery may not read #{@folder}. Allow it in System Settings, or choose another folder.")
    end

    # One picture, or nil when the bytes are not a picture the engine shows.
    def read(path : String) : Photo?
      info = File.info(path)
      head = Bytes.new(Math.min(HEAD.to_i64, info.size).to_i)
      File.open(path, &.read_fully(head))
      return unless Picture.sniff(head)
      exif = ExifReader.read(head, @location)
      dims = Picture.dimensions(head)
      w, h = dims ? dims : {exif.width, exif.height}
      thumb = @thumbs.try &.fetch(path, info.size, info.modification_time, head, exif)
      Photo.new(path, info.size, info.modification_time, exif, w, h, thumb)
    rescue File::Error | IO::EOFError
      nil
    end

    # Yield every listed file under `dir`; the block returns false to stop the walk.
    private def walk(dir : String, depth : Int32, &block : String -> Bool) : Bool
      return true if depth > @depth
      names = Dir.children(dir).sort!
      names.each do |name|
        next if name.starts_with?('.')
        path = File.join(dir, name)
        info = File.info?(path, follow_symlinks: false)
        next unless info
        if info.directory?
          return false unless walk(path, depth + 1, &block)
        elsif info.file? && Picture.listed?(name)
          return false unless block.call(path)
        end
      end
      true
    rescue File::AccessDeniedError
      raise ScanError.new("The gallery may not read #{dir}. Allow it in System Settings, or choose another folder.") if depth == 0
      true
    end
  end
end
