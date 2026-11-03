# SPDX-License-Identifier: GPL-2.0-or-later
# Where the library comes from: the Pictures folder, scanned on the background pool. The app asks a
# Source and gets one answer on the UI context; the specs give it a fixed one.
module Gallery
  alias ScanOutcome = ScanResult | Exception

  abstract class Source
    # The folder the library shows, for the sentences of the empty states.
    abstract def folder : String

    # Read the library; `done` runs once, on the UI context, with the result or what went wrong.
    abstract def load(&done : ScanOutcome ->) : Nil
  end

  # The person's Pictures folder, with thumbnails cached in the app's data folder.
  class FolderSource < Source
    getter folder : String

    # `depth`: 0 reads only the folder itself (the app was opened on a file in it).
    def initialize(@folder : String = Dirs.pictures, @thumbs : String = Dirs.thumbnails, @depth : Int32 = Library::MAX_DEPTH)
    end

    def load(&done : ScanOutcome ->) : Nil
      library = Library.new(@folder, Thumbs.new(@thumbs), depth: @depth)
      Wolkit::Background.run(-> { library.scan }, ->(r : ScanResult | Exception) { done.call(r); nil })
    end
  end
end
