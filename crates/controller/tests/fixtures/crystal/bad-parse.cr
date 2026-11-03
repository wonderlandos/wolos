# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-024
# Scanner fixture: untyped JSON decoding of input from outside.
require "json"

module BadParse
  def self.tag(body : String) : String
    JSON.parse(body)["tag"].as_s
  end
end
