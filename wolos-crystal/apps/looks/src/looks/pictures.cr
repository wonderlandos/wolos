# SPDX-License-Identifier: GPL-2.0-or-later
# The pictures Looks offers to start from: the files of the wallpaper folders and of the Pictures
# folder, each with a small thumbnail kept in the cache, and the copy it makes of a picture the person
# hands over through the file panel (the system gives the app an open file, never a path).
#
# Concept: a Pic is a path and a thumbnail path. Why: the first step of the app is choosing a
# picture, so it must be quick and must never read the whole folder's pictures at full size twice.
# How: the folders are walked one level deep, without links; each thumbnail is a 96 x 64 PNG made by
# the engine's decoder (so any format wol-hue reads has a thumbnail), named by a hash of the path,
# size and change time, and written through a temporary file.
require "digest/sha256"
require "file_utils"
require "./engine_link"

module Looks
  record Pic, path : String, name : String, thumb : String?

  module Pictures
    EXTENSIONS = %w(.png .jpg .jpeg .webp .svg .avif)
    MAX_BYTES  = 64_i64 * 1024 * 1024
    THUMB_W    = 96
    THUMB_H    = 64

    # Picture files directly in the folders, wallpapers first, at most `limit`, no links.
    def self.list(folders : Array(String), limit : Int32 = 12) : Array(String)
      found = [] of String
      folders.each do |dir|
        next unless Dir.exists?(dir)
        Dir.children(dir).sort!.each do |name|
          next if name.starts_with?('.')
          next unless EXTENSIONS.includes?(File.extname(name).downcase)
          path = File.join(dir, name)
          info = File.info?(path, follow_symlinks: false)
          next unless info && info.file? && info.size <= MAX_BYTES
          found << path
          return found if found.size >= limit
        end
      rescue File::Error
        next
      end
      found
    end

    # A name for the person: the file name without its extension, dashes and underscores as spaces.
    def self.title(path : String) : String
      File.basename(path, File.extname(path)).gsub(/[-_]+/, " ").strip
    end

    # Writes the bytes the person handed over into the app's folder; returns the path. The name is
    # reduced to letters, digits, dot, dash and underscore, and prefixed by a hash of the content, so
    # the same picture is stored once and a name can never leave the folder.
    def self.keep(dir : String, name : String, io : IO) : String
      data = IO::Memory.new
      IO.copy(io, data, limit: MAX_BYTES + 1)
      raise Hue::ImageError.new("This picture is larger than 64 MB.") if data.size > MAX_BYTES
      clean = File.basename(name).gsub(/[^A-Za-z0-9._-]+/, "-").lstrip('.')
      clean = "picture" if clean.empty?
      clean = clean[-60..] if clean.size > 60
      path = File.join(dir, "#{Digest::SHA256.hexdigest(data.to_slice)[0, 12]}-#{clean}")
      return path if File.file?(path)
      FileUtils.mkdir_p(dir, 0o700)
      tmp = "#{path}.tmp.#{Process.pid}"
      begin
        File.write(tmp, data.to_slice, perm: 0o600)
        File.rename(tmp, path)
      ensure
        File.delete?(tmp)
      end
      path
    end
  end

  # Thumbnails in the cache folder.
  class Thumbs
    def initialize(@dir : String)
    end

    def key(path : String) : String?
      info = File.info?(path)
      return unless info
      Digest::SHA256.hexdigest("#{path}\0#{info.size}\0#{info.modification_time.to_unix}")[0, 24]
    end

    # The thumbnail's path, made when it is not there yet; nil when the picture cannot be read.
    def fetch(path : String) : String?
      k = key(path) || return
      file = File.join(@dir, "#{k}.png")
      return file if File.file?(file)
      pic = Hue::Picture.load(path, 400)
      cover = Math.max(Pictures::THUMB_W.to_f / pic.width, Pictures::THUMB_H.to_f / pic.height)
      long = Math.max((pic.width * cover).ceil.to_i, (pic.height * cover).ceil.to_i)
      small = pic.fit(Math.max(long, 1))
      out = crop(small, Pictures::THUMB_W, Pictures::THUMB_H)
      FileUtils.mkdir_p(@dir, 0o700)
      tmp = "#{file}.tmp.#{Process.pid}.png"
      begin
        out.save(tmp)
        File.rename(tmp, file)
      ensure
        File.delete?(tmp)
      end
      file
    rescue Hue::ImageError | File::Error
      nil
    end

    # The centre of the picture, `w` x `h`, or the picture on a transparent ground when it is smaller.
    def crop(src : Hue::Picture, w : Int32, h : Int32) : Hue::Picture
      res = Bytes.new(w * h * 4)
      ox = (src.width - w) // 2
      oy = (src.height - h) // 2
      h.times do |y|
        sy = y + oy
        next if sy < 0 || sy >= src.height
        w.times do |x|
          sx = x + ox
          next if sx < 0 || sx >= src.width
          res[(y * w + x) * 4, 4].copy_from(src.rgba[(sy * src.width + sx) * 4, 4])
        end
      end
      Hue::Picture.new(w, h, res)
    end

    # Removes thumbnails whose picture is gone from `keep` (their keys), so the cache stays small.
    def prune(keep : Set(String)) : Nil
      return unless Dir.exists?(@dir)
      Dir.children(@dir).each do |f|
        next unless f.ends_with?(".png")
        File.delete?(File.join(@dir, f)) unless keep.includes?(f.chomp(".png"))
      end
    rescue File::Error
      nil
    end
  end
end

module Looks
  # Where the pictures to start from come from. The app asks and gets one answer on the UI context.
  abstract class PictureSource
    abstract def load(&done : Array(Pic) ->) : Nil
  end

  # What a source does, without the thread: the wallpaper folders and the Pictures folder, each with
  # its thumbnail.
  class Scan
    def initialize(@folders : Array(String), @thumbs : Thumbs, @limit : Int32 = 12)
    end

    def run : Array(Pic)
      paths = Pictures.list(@folders, @limit)
      pics = paths.map { |p| Pic.new(p, Pictures.title(p), @thumbs.fetch(p)) }
      @thumbs.prune(paths.compact_map { |p| @thumbs.key(p) }.to_set)
      pics
    end
  end
end
