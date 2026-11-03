# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-022
# Scanner fixture: the class-method HTTP call has no connect or read timeout.
require "http/client"

module BadHttp
  def self.fetch(url : String) : String
    HTTP::Client.get(url).body
  end
end
