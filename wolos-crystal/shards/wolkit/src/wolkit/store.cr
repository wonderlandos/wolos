# SPDX-License-Identifier: BSD-2-Clause
# Stores for `persisted` signals. A store reads and writes strings by key. The default one keeps
# them in memory; `Wolkit::Services` installs one backed by the app's preferences (libwol
# `wol_prefs`, grant `app-preferences`). A store that is refused says so in a sentence the app can
# show; the signal keeps working in memory (plan-wolkit section 3.4).
module Wolkit
  abstract class Store
    # The stored text, or nil.
    abstract def read(key : String) : String?

    # Store `value`. Returns nil on success, or the sentence to show when the store refused.
    abstract def write(key : String, value : String) : String?

    @@current : Store = MemoryStore.new

    def self.current : Store
      @@current
    end

    def self.current=(store : Store) : Store
      @@current = store
    end
  end

  # Keeps values in the process; the default, and the store of specs.
  class MemoryStore < Store
    @data = {} of String => String

    def read(key : String) : String?
      @data[key]?
    end

    def write(key : String, value : String) : String?
      @data[key] = value
      nil
    end

    def clear : Nil
      @data.clear
    end
  end
end
