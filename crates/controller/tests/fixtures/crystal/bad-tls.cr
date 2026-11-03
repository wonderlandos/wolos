# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-023
# Scanner fixture: certificate checks switched off.
require "openssl"

module BadTls
  def self.context : OpenSSL::SSL::Context::Client
    ctx = OpenSSL::SSL::Context::Client.new
    ctx.verify_mode = OpenSSL::SSL::VerifyMode::NONE
    ctx
  end
end
