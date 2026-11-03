# SPDX-License-Identifier: BSD-2-Clause
# Preferences of this app, for this account (wol.h "1.4: answers, preferences", WOL_1.4): a small
# string map kept by the system, which no other app can reach. Permission: app-preferences. A key is
# 1 to 64 letters, digits, '.', '_' or '-'; a value up to 4 KiB with no NUL; 256 keys per app.
# libwol checks all of it and refuses with `InvalidArgument`.
require "./lib_wol"
require "./error"
require "./string"
require "./info"

module Wol
  # The app's preferences, read and written through the system service on every call (nothing is
  # cached, so two processes of the app see the same values).
  #
  # ```
  # prefs = Wol::Prefs.new(ctx)
  # prefs["width"] = "640"
  # prefs["width"]? # => "640"
  # ```
  struct Prefs
    def initialize(@context : Context)
    end

    # The value of `key`, or nil when it is not set.
    def []?(key : ::String) : ::String?
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_prefs_copy_string(@context, Strings.ptr(key), Strings.size(key), o, err)
      end
      Strings.take(str)
    end

    # The value of `key`. Raises `KeyError` when it is not set.
    def [](key : ::String) : ::String
      self[key]? || raise KeyError.new("No preference #{key.inspect}")
    end

    # Set `key` to `value`.
    def []=(key : ::String, value : ::String) : ::String
      Wol.call do |err|
        LibWol.wol_prefs_set_string(@context, Strings.ptr(key), Strings.size(key),
          Strings.ptr(value), Strings.size(value), err)
      end
      value
    end

    # Remove `key`. Removing a key that is not set does nothing.
    def delete(key : ::String) : Nil
      Wol.call { |err| LibWol.wol_prefs_remove(@context, Strings.ptr(key), Strings.size(key), err) }
    end

    # Every key that is set.
    def keys : Array(::String)
      info = Info.copy { |o, err| LibWol.wol_prefs_copy_keys(@context, o, err) }
      begin
        info.compact_map(&.as_s?)
      ensure
        info.close
      end
    end
  end
end
