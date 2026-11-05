# SPDX-License-Identifier: BSD-2-Clause
# Things this app asks the desktop to do as the person (wol.h "1.16", WOL_1.16): add a recent file,
# let the person pick a colour on the screen, print a file. The app learns only what was done.
require "./lib_wol"
require "./error"
require "./string"

module Wol
  module Recents
    # Add the file or folder at `path` (a full path) to the person's recent files, as this app
    # opened it: Files shows it under Recent. Permission: recents. The desktop adds it only if it is
    # in the person's home folder or on a mounted drive with nothing hidden in its path (links
    # followed); otherwise, or if it is not there, `Wol::Unavailable` with one sentence.
    def self.add(context : Context, path : ::String) : Nil
      Wol.call { |err| LibWol.wol_recent_add(context, Strings.ptr(path), Strings.size(path), err) }
    end
  end

  module Colors
    # Let the person click a colour anywhere on the screen. Returns it as 0xRRGGBB, or nil when
    # they cancelled. Permission: color-picker. Raises `Unavailable` when this computer cannot
    # pick a colour.
    # Blocks: until the person clicks or cancels (run it in `Wol.background`).
    def self.pick(context : Context) : UInt32?
      rgb, picked = Wol.scalars(UInt32, Int32) { |r, p, err| LibWol.wol_color_pick(context, r, p, err) }
      picked == 0 ? nil : rgb
    end
  end

  module Print
    # Open the print dialog for the file at `path` (a full path; PDF, PostScript, plain text, PNG
    # or JPEG) in the same places as `Recents.add`. Returns "printed" or "cancelled". Permission:
    # print.
    # Blocks: until the person chooses (run it in `Wol.background`).
    def self.file(context : Context, path : ::String) : ::String
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_print_file(context, Strings.ptr(path), Strings.size(path), o, err)
      end
      Strings.take(str) || ""
    end
  end
end
