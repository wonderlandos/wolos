# SPDX-License-Identifier: GPL-2.0-or-later
# The preview pictures as files. The window shows an image by file path, and the engine may keep what
# it read for a path, so every redraw writes new file names (a generation number in the name) and
# the files of generations that are no longer on screen are removed.
#
# Concept: Previews.write(picture, tag) returns the path of a fresh PNG. Why: a slider moves many
# times a second, each move changes a dozen pictures, and none of them may pile up on disk or show a
# stale frame. How: names are `<tag>-<generation>.png`; `finish` deletes every file of a generation
# older than the one before the current, so the frame on screen and the one being swapped in both
# exist while the swap happens.
require "file_utils"
require "./raster"

module Looks
  class Previews
    getter dir : String
    getter generation : Int32 = 0

    def initialize(@dir : String)
    end

    # A neutral picture of the size the real ones will have, for the first frame (kept across
    # generations); its path.
    def placeholder(w : Int32, h : Int32, tag : String) : String
      FileUtils.mkdir_p(@dir, 0o700)
      file = File.join(@dir, "blank-#{tag}.png")
      Raster.unreadable(w, h).save(file) unless File.file?(file)
      file
    rescue File::Error | Hue::ImageError
      ""
    end

    # Starts a new set of pictures.
    def next_generation : Int32
      @generation += 1
    end

    # Writes `pic` for `tag` in the current generation; the path, or nil when the disk refuses.
    def write(pic : Hue::Picture, tag : String) : String?
      FileUtils.mkdir_p(@dir, 0o700)
      file = File.join(@dir, "#{tag}-#{@generation}.png")
      tmp = "#{file}.tmp"
      begin
        pic.save(tmp)
        File.rename(tmp, file)
      ensure
        File.delete?(tmp)
      end
      file
    rescue File::Error | Hue::ImageError
      nil
    end

    # Removes files of generations before the previous one.
    def finish : Nil
      return unless Dir.exists?(@dir)
      Dir.children(@dir).each do |name|
        g = name.match(/\A[a-z]+-\d+-(\d+)\.png\z/).try(&.[1].to_i?) || next
        File.delete?(File.join(@dir, name)) if g < @generation - 1
      end
    rescue File::Error
      nil
    end

    # Removes everything (the window closed).
    def clear : Nil
      FileUtils.rm_rf(@dir)
    end
  end
end
