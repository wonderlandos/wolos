# SPDX-License-Identifier: BSD-2-Clause
# Shared spec support for the pure parts of Wolkit (no engine, no display).
require "spec"
require "file_utils"
require "../src/wolkit/core"

# Compare `actual` with spec/golden/<name>. `WOLKIT_BLESS=1 make spec` rewrites the file; review the
# diff before committing it, a golden is a promise about what the DSL emits.
def golden(name : String, actual : String) : Nil
  path = File.join(__DIR__, "golden", name)
  if ENV["WOLKIT_BLESS"]?
    Dir.mkdir_p(File.dirname(path))
    File.write(path, actual)
  else
    File.read(path).should eq(actual), "#{path} differs; run WOLKIT_BLESS=1 make spec and review the diff"
  end
end
