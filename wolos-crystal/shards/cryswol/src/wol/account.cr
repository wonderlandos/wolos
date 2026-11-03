# SPDX-License-Identifier: BSD-2-Clause
# The account running this app, and no other (wol.h "1.10: accounts", WOL_1.10).
require "./lib_wol"
require "./error"
require "./info"

module Wol
  module Account
    # Keys: name, full_name (the name when the account has no other). Permission: account-info.
    def self.info(context : Context) : Info
      Info.copy { |slot, err| LibWol.wol_account_copy_info(context, slot, err) }
    end
  end
end
