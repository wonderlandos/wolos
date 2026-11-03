# SPDX-License-Identifier: BSD-2-Clause
# The program: `wol-gallery` opens the library of the Pictures folder; `wol-gallery PICTURE` opens
# that picture in the viewer, with the other pictures of its folder in the filmstrip. Runs until the
# window is closed. The specs require src/gallery.cr instead, so they never start the real app.
require "./gallery"

class GalleryMain < Gallery::App
  def initialize
    arg = ARGV.find { |a| !a.starts_with?('-') }
    path = arg.try { |a| File.expand_path(a.starts_with?("file://") ? URI.decode(a.lchop("file://")) : a) }
    if path && File.file?(path)
      super(Gallery::FolderSource.new(File.dirname(path), depth: 0), open_path: path)
    else
      super()
    end
  end
end

Wolkit.run GalleryMain, app_id: Gallery::APP_ID
