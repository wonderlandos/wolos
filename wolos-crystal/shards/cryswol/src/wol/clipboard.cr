# SPDX-License-Identifier: BSD-2-Clause
# The clipboard (wol.h "1.3: clipboard", WOL_1.3). The text is the person's: it is never logged or
# kept by this module (docs/STANDARDS.md §7).
require "./lib_wol"
require "./error"
require "./string"

module Wol
  module Clipboard
    # The text on the clipboard ("" when it holds none). Permission: clipboard-read.
    def self.text(context : Context) : ::String
      str = Wol.out(LibWol::String) { |o, err| LibWol.wol_clipboard_copy_text(context, o, err) }
      Strings.take(str) || ""
    end

    # Put `text` on the clipboard: up to 48 KiB, no NUL byte (libwol refuses more with
    # `InvalidArgument`). Permission: clipboard-write.
    def self.set_text(context : Context, text : ::String) : Nil
      Wol.call { |err| LibWol.wol_clipboard_set_text(context, Strings.ptr(text), Strings.size(text), err) }
    end
  end
end
