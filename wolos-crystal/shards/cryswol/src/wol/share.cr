# SPDX-License-Identifier: BSD-2-Clause
# Sharing text (wol.h "1.13: sharing", WOL_1.13) and files (wol.h "1.15", WOL_1.15). The desktop
# shows the person what is offered; they copy it, save it, open it with another app or cancel.
# The app never learns where a copy went or which app opened it, and this module never logs the
# text (docs/STANDARDS.md §7).
require "./lib_wol"
require "./error"
require "./string"

module Wol
  module Share
    # Offer up to 24 KiB of `text`. Returns what the person chose: "copied", "saved" or
    # "cancelled". Permission: share.
    # Blocks: until the person chooses (run it in `Wol.background`).
    def self.text(context : Context, title : ::String, text : ::String) : ::String
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_share_text(context, Strings.ptr(title), Strings.size(title),
          Strings.ptr(text), Strings.size(text), o, err)
      end
      Strings.take(str) || ""
    end

    # Offer the file at `path` (full path, a file, not a folder) on the share sheet. Returns what
    # the person chose: "copied" (a link to the file on the clipboard), "saved" (a copy),
    # "opened" (with an app they picked) or "cancelled". Permission: share.
    # Blocks: until the person chooses (run it in `Wol.background`).
    def self.file(context : Context, path : ::String) : ::String
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_share_file(context, Strings.ptr(path), Strings.size(path), o, err)
      end
      Strings.take(str) || ""
    end

    # Let the person choose the app that opens the file at `path`. Returns "opened" or
    # "cancelled"; raises `Wol::Unavailable` when no app opens that kind of file.
    # Permission: share.
    # Blocks: until the person chooses (run it in `Wol.background`).
    def self.open_with(context : Context, path : ::String) : ::String
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_open_with(context, Strings.ptr(path), Strings.size(path), o, err)
      end
      Strings.take(str) || ""
    end
  end
end
