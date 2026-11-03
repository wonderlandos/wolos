# SPDX-License-Identifier: BSD-2-Clause
# Sharing text (wol.h "1.13: sharing", WOL_1.13). The desktop shows the person the title and the
# text; they copy it, save it or cancel. The app never learns where a file was saved, and this
# module never logs the text (docs/STANDARDS.md §7).
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
  end
end
