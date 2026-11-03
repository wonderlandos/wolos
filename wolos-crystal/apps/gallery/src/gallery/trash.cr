# SPDX-License-Identifier: GPL-2.0-or-later
# Moving a picture to the Trash, and back. The Trash is the shared folder of the session (a `files`
# folder with the items and an `info` folder with one note per item saying where it came from and
# when), so the file manager's Trash shows the picture and can put it back.
#
# Concept: delete is a move, never an unlink, and it can be undone. Why: the HIG prefers undo over a
# confirmation, and a wrong key press must not lose a photo. How: the note is written first, then the
# file is renamed into the Trash; a rename that fails (another drive) removes the note again and says
# so in one sentence. Restoring renames it back if nothing took its place.
require "uri"

module Gallery
  class TrashError < Exception
  end

  record Trashed, original : String, name : String

  class Trash
    getter root : String

    def initialize(@root : String)
    end

    def files : String
      File.join(@root, "files")
    end

    def info : String
      File.join(@root, "info")
    end

    # Move `path` to the Trash. Raises TrashError with the sentence to show.
    def move(path : String, now : Time = Time.local) : Trashed
      raise TrashError.new("This picture is gone already.") unless File.exists?(path)
      Dir.mkdir_p(files, 0o700)
      Dir.mkdir_p(info, 0o700)
      name = free_name(File.basename(path))
      note = File.join(info, "#{name}.trashinfo")
      File.write(note, note_text(File.expand_path(path), now), perm: 0o600)
      begin
        File.rename(path, File.join(files, name))
      rescue File::Error
        File.delete?(note)
        raise TrashError.new("This picture is on another drive or read-only, so it cannot go to the Trash.")
      end
      Trashed.new(File.expand_path(path), name)
    rescue File::AccessDeniedError
      raise TrashError.new("The gallery may not change the Trash folder.")
    end

    # Put a trashed picture back where it was.
    def restore(item : Trashed) : Nil
      raise TrashError.new("Another file now has this picture’s name, so it stays in the Trash.") if File.exists?(item.original)
      Dir.mkdir_p(File.dirname(item.original))
      File.rename(File.join(files, item.name), item.original)
      File.delete?(File.join(info, "#{item.name}.trashinfo"))
    rescue File::Error
      raise TrashError.new("The picture could not be put back; it is still in the Trash.")
    end

    # The note: where the item came from (percent-encoded) and when it was deleted (local time).
    def note_text(path : String, now : Time) : String
      "[Trash Info]\nPath=#{URI.encode_path(path)}\nDeletionDate=#{now.to_s("%Y-%m-%dT%H:%M:%S")}\n"
    end

    # `name`, or `name 2.ext`, `name 3.ext`... whichever has neither an item nor a note yet.
    def free_name(name : String) : String
      ext = File.extname(name)
      stem = name[0, name.size - ext.size]
      n = 1
      candidate = name
      while File.exists?(File.join(files, candidate)) || File.exists?(File.join(info, "#{candidate}.trashinfo"))
        n += 1
        candidate = "#{stem} #{n}#{ext}"
      end
      candidate
    end
  end
end
