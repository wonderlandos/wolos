# SPDX-License-Identifier: GPL-2.0-or-later
# The real source of the pictures to start from: the wallpaper folders and the Pictures folder,
# scanned on the Wolkit background pool so the window never waits for a thumbnail.
module Looks
  class FolderPictures < PictureSource
    def initialize(@scan : Scan = Scan.new(Dirs.wallpapers + [Dirs.pictures], Thumbs.new(Dirs.thumbnails)))
    end

    def load(&done : Array(Pic) ->) : Nil
      scan = @scan
      Wolkit::Background.run(-> { scan.run }, ->(r : Array(Pic) | Exception) do
        if r.is_a?(Exception)
          Looks.trace("pictures failed: #{r.message}")
          done.call([] of Pic)
        else
          done.call(r)
        end
        nil
      end)
    end
  end
end
