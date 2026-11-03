# SPDX-License-Identifier: BSD-2-Clause
# One picture of the library: where the file is, what the camera block said, and the thumbnail the
# grid shows. A plain value, so the library can be built off the UI context and handed over whole.
module Gallery
  struct Photo
    getter path : String
    getter bytes : Int64
    getter modified : Time
    getter exif : Exif
    getter width : Int32?
    getter height : Int32?
    # The file the grid and filmstrip show: a cached thumbnail, or the picture itself.
    getter thumbnail : String

    def initialize(@path, @bytes, @modified, @exif = Exif.new, @width = nil, @height = nil, thumbnail : String? = nil)
      @thumbnail = thumbnail || @path
    end

    def name : String
      File.basename(@path)
    end

    # When it was taken, else when the file last changed.
    def date : Time
      @exif.taken || @modified
    end

    def dated_by_camera? : Bool
      !@exif.taken.nil?
    end

    def place : GeoPoint?
      @exif.place
    end

    # Width and height as shown (a quarter turn swaps them).
    def shown_size : {Int32, Int32}?
      w, h = @width, @height
      return unless w && h
      @exif.quarter_turn? ? {h, w} : {w, h}
    end

    def with_thumbnail(path : String) : Photo
      Photo.new(@path, @bytes, @modified, @exif, @width, @height, path)
    end
  end
end
