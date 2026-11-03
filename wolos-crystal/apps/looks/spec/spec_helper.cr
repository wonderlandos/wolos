# SPDX-License-Identifier: GPL-2.0-or-later
# Helpers for the pure specs: pictures drawn from code (the scenes of hue/spec), a temporary folder
# per example, a fake desktop that records what was applied.
require "spec"
require "file_utils"
require "../src/looks/core"
require "../../../../hue/spec/support/scenes"
require "./support/fakes"

# A temporary folder for one example, removed after it.
def with_tmpdir(& : String ->) : Nil
  dir = File.join(Dir.tempdir, "looks-spec-#{Random::Secure.hex(6)}")
  Dir.mkdir_p(dir, 0o700)
  begin
    yield dir
  ensure
    FileUtils.rm_rf(dir)
  end
end

# A picture file for a named scene of hue/spec/support/scenes.cr.
def scene_file(dir : String, name : String) : String
  path = File.join(dir, "#{name}.png")
  Scenes.scene(name).save(path) unless File.file?(path)
  path
end

# A sentence that must be there (the nil of "it worked" fails the example).
def said(s : String?) : String
  s.should_not be_nil
  s || ""
end
