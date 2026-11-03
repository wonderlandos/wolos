# SPDX-License-Identifier: BSD-2-Clause
# The thumbnail cache, under the app's data folder. A thumbnail is the preview the camera stored in
# the picture (a small JPEG), copied out once so the grid never reads the full file again.
#
# Concept: one file per picture version, named by a SHA-256 of the path, size and change time, so a
# changed or moved picture gets a new entry and a stale one is never shown. Why: decoding and
# scaling full pictures is the engine's job (it lands with its image decoder); until then the stored
# preview gives most camera pictures a real thumbnail at the cost of a copy. How: written to a
# temporary file in the same folder and renamed, mode 0600; `prune` drops entries no picture uses.
require "digest/sha256"

module Gallery
  class Thumbs
    # A stored preview larger than this is not a preview.
    MAX_PREVIEW = 512 * 1024

    getter dir : String

    def initialize(@dir : String)
    end

    def key(path : String, bytes : Int64, modified : Time) : String
      Digest::SHA256.hexdigest("#{path}\0#{bytes}\0#{modified.to_unix_ns}")
    end

    def path_for(key : String) : String
      File.join(@dir, "#{key}.jpg")
    end

    # The cached thumbnail of this version of the picture, writing it from `head` (the picture's
    # first bytes) when the camera stored a preview there. Nil when there is none.
    def fetch(path : String, bytes : Int64, modified : Time, head : Bytes, exif : Exif) : String?
      target = path_for(key(path, bytes, modified))
      return target if File.file?(target)
      range = exif.preview
      return unless range && range.size <= MAX_PREVIEW && range.end <= head.size
      store(target, head[range.begin, range.size])
    end

    # Remove every cached file whose key is not in `keep`. Returns how many went.
    def prune(keep : Set(String)) : Int32
      return 0 unless Dir.exists?(@dir)
      gone = 0
      Dir.each_child(@dir) do |name|
        next unless name.ends_with?(".jpg") && name.size == 68
        next if keep.includes?(name[0, 64])
        File.delete(File.join(@dir, name))
        gone += 1
      rescue File::Error
        next
      end
      gone
    end

    private def store(target : String, data : Bytes) : String?
      Dir.mkdir_p(@dir, 0o700)
      tmp = File.tempfile("thumb", ".part", dir: @dir) { |f| f.write(data) }
      File.chmod(tmp.path, 0o600)
      File.rename(tmp.path, target)
      target
    rescue File::Error
      tmp.try { |t| File.delete?(t.path) }
      nil
    end
  end
end
