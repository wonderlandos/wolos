# SPDX-License-Identifier: BSD-2-Clause
# The pure specs: the gallery's core (no engine) and the picture fixtures.
require "spec"
require "../src/gallery/core"
require "./support/fixtures"

UTC = Time::Location::UTC

# A temporary folder for one example, removed after it.
def with_tmpdir(& : String ->) : Nil
  dir = File.join(Dir.tempdir, "gallery-spec-#{Random::Secure.hex(6)}")
  Dir.mkdir_p(dir, 0o700)
  begin
    yield dir
  ensure
    FileUtils.rm_rf(dir)
  end
end

require "file_utils"
