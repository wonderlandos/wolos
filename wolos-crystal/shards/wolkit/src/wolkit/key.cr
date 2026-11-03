# SPDX-License-Identifier: BSD-2-Clause
# Row keys: a stable 64-bit number for the value `each` was given as `key:`. Strings, symbols and
# integers hash the same on every run (FNV-1a), so a patch stream is reproducible and goldens do not
# change between processes; anything else uses its own `hash`.
module Wolkit
  module Key
    OFFSET = 0xcbf29ce484222325_u64
    PRIME  =      0x100000001b3_u64

    def self.of(value : String) : UInt64
      hash = OFFSET
      value.to_slice.each { |byte| hash = (hash ^ byte) &* PRIME }
      hash
    end

    def self.of(value : Symbol) : UInt64
      of(value.to_s)
    end

    def self.of(value : Int) : UInt64
      value.to_u64!
    end

    def self.of(value) : UInt64
      value.hash.to_u64!
    end
  end
end
