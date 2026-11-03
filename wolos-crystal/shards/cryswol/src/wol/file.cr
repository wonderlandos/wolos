# SPDX-License-Identifier: BSD-2-Clause
# Files the person picks (wol.h "1.5: files and panels", WOL_1.5). The app gets the one file
# chosen, already open, and never a path. The descriptor belongs to libwol's `wol_file`, so
# `OpenFile#to_io` hands out a close-on-exec duplicate that outlives `close` (plan-cr1 §3 rule 8).
# Named `OpenFile`, not `File`, so it never shadows `::File` inside `module Wol`.
require "./lib_wol"
require "./error"
require "./handle"
require "./string"
require "./context"

module Wol
  # An open file the system handed over: from a panel (`Panel.open`, `Panel.save`) or a device
  # (`Device.open`).
  class OpenFile < Handle(LibWol::File)
    # Wrap an owned `wol_file` (from a verb). NULL gives nil: the person cancelled.
    def self.take(ptr : LibWol::File*) : OpenFile?
      ptr.null? ? nil : new(ptr)
    end

    # The file's name, without its folder (a device's name for a device).
    def name : ::String
      Strings.borrow(LibWol.wol_file_get_name(self)) || ""
    end

    # The descriptor, owned by this object: valid until `close`. Use `to_io` to keep it longer.
    def fd : Int32
      LibWol.wol_file_get_fd(self)
    end

    # A Crystal IO on a duplicate of the descriptor (close-on-exec). It stays open after `close`;
    # closing it is the caller's.
    def to_io : IO::FileDescriptor
      Wol.dup_fd(fd)
    end

    # The whole file as text, read from the start through a duplicate (the descriptor's own
    # offset is shared with it, so this reads from wherever the file stands).
    def read : ::String
      io = to_io
      begin
        io.gets_to_end
      ensure
        io.close
      end
    end

    protected def release(ptr : LibWol::File*) : Nil
      LibWol.wol_file_release(ptr)
    end
  end

  # The open and save panels: the person picks, the app gets that file. Permission: file-panels.
  module Panel
    # Ask the person for a file to read; `title` may be empty. Nil when they cancel.
    # Blocks: up to ten minutes, while the person chooses (run it in `Wol.background`).
    def self.open(context : Context, title : ::String = "") : OpenFile?
      ptr = Wol.out(LibWol::File) do |slot, err|
        LibWol.wol_panel_open(context, Strings.ptr(title), Strings.size(title), slot, err)
      end
      OpenFile.take(ptr)
    end

    # Ask the person where to save; the file comes open for writing and emptied. `name` is the
    # suggested file name (no '/'). Nil when they cancel.
    # Blocks: up to ten minutes, while the person chooses (run it in `Wol.background`).
    def self.save(context : Context, name : ::String, title : ::String = "") : OpenFile?
      ptr = Wol.out(LibWol::File) do |slot, err|
        LibWol.wol_panel_save(context, Strings.ptr(title), Strings.size(title),
          Strings.ptr(name), Strings.size(name), slot, err)
      end
      OpenFile.take(ptr)
    end
  end
end
