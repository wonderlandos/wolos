# SPDX-License-Identifier: GPL-2.0-or-later
# Looks' door to the file panels. The system hands the app one open file (never a path) after the
# person chooses it in the panel, and an open file to write to; this interface gives the app the bytes
# or a sentence, and the specs give it a fake. Everything a panel does is off the UI thread.
#
# Concept: each verb answers once, on the UI context: the handed file, nothing (they cancelled) or a
# sentence (the system said no, with what to do). Why: a refusal is an everyday outcome of the
# permission system, and the HIG wants it said where it happened. How: `Panels` calls
# `Wolkit::Services.open_file` and `save_file`, which run on `Wol.background`.
module Looks
  # A file the person handed over: its name and what is in it (at most 64 MB).
  record Handed, name : String, bytes : Bytes

  abstract class System
    MAX_BYTES = 64 * 1024 * 1024

    # Ask for a file to read. The block gets the file, or a sentence, or neither when they cancelled.
    abstract def open(title : String, &done : Handed?, String? ->) : Nil

    # Ask where to save `text` as `name`. The block gets nil when it was written, or a sentence;
    # cancelling is the sentence "Nothing was saved."
    abstract def save(name : String, text : String, &done : String? ->) : Nil
  end

  CANCELLED = "Nothing was saved."

  class Panels < System
    def open(title : String, &done : Handed?, String? ->) : Nil
      Wolkit::Services.open_file(title) do |outcome|
        if file = outcome.value
          begin
            io = file.to_io
            data = IO::Memory.new
            begin
              IO.copy(io, data, limit: MAX_BYTES + 1)
            ensure
              io.close
            end
            if data.size > MAX_BYTES
              done.call(nil, "That file is larger than 64 MB.")
            else
              done.call(Handed.new(file.name, data.to_slice.dup), nil)
            end
          rescue ex : IO::Error
            done.call(nil, "That file could not be read: #{ex.message}")
          ensure
            file.close
          end
        else
          done.call(nil, outcome.notice)
        end
      end
    end

    def save(name : String, text : String, &done : String? ->) : Nil
      Wolkit::Services.save_file(name, "Export look") do |outcome|
        if file = outcome.value
          begin
            io = file.to_io
            begin
              io << text
              io.flush
            ensure
              io.close
            end
            done.call(nil)
          rescue ex : IO::Error
            done.call("The look could not be written: #{ex.message}")
          ensure
            file.close
          end
        else
          done.call(outcome.notice || CANCELLED)
        end
      end
    end
  end
end
