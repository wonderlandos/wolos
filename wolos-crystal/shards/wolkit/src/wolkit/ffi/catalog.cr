# SPDX-License-Identifier: BSD-2-Clause
# FFI::Catalog: a unit's compiled text tables (wolkit.h "text catalogs", ADR 0045) and
# `I18n::Native`, the `Wolkit::I18n::Source` over them. Lookups are constant time and allocate
# nothing in libwolkit; the String made for a label is kept by `Wolkit::I18n`, once per language.
require "./call"
require "../i18n"

module Wolkit
  module FFI
    class Catalog < Wol::Handle(LibWolkit::Catalog)
      # Build a catalog from tables held in memory: the English one and any translations.
      def self.load(tables : Array(Bytes)) : Catalog
        pointers = tables.map(&.to_unsafe)
        lens = tables.map { |t| LibC::SizeT.new(t.size) }
        ptr = FFI.out(LibWolkit::Catalog) do |slot, err|
          LibWolkit.wolkit_catalog_load(pointers.to_unsafe, lens.to_unsafe, lens.size, slot, err)
        end
        new(ptr)
      end

      # Read `<dir>/<unit>.<lang>.wtx` for every language that has a file. Raises when the English
      # table is missing.
      def self.load_dir(dir : ::String, unit : ::String) : Catalog
        ptr = FFI.out(LibWolkit::Catalog) do |slot, err|
          LibWolkit.wolkit_catalog_load_dir(dir.to_unsafe, dir.bytesize, unit.to_unsafe, unit.bytesize, slot, err)
        end
        new(ptr)
      end

      protected def release(ptr : Pointer(LibWolkit::Catalog)) : Nil
        LibWolkit.wolkit_catalog_release(ptr)
      end

      def count : Int32
        LibWolkit.wolkit_catalog_count(self).to_i
      end

      # The key named `name`, or nil.
      def find(name : ::String) : UInt32?
        key = LibWolkit.wolkit_catalog_find(self, name.to_unsafe, name.bytesize)
        key == LibWolkit::WOLKIT_NO_KEY ? nil : key
      end

      # The text of `key` in language id `lang` (WOLKIT_LANG_*), English when the language lacks it.
      def text(key : UInt32, lang : Int32) : ::String
        ptr = Pointer(UInt8).null
        len = LibC::SizeT.new(0)
        LibWolkit.wolkit_catalog_text(self, key, lang, pointerof(ptr), pointerof(len))
        ::String.new(ptr, len)
      end

      def plural(key : UInt32, n : UInt64, lang : Int32) : ::String
        ptr = Pointer(UInt8).null
        len = LibC::SizeT.new(0)
        LibWolkit.wolkit_catalog_plural(self, key, n, lang, pointerof(ptr), pointerof(len))
        ::String.new(ptr, len)
      end
    end
  end

  module FFI
    # The language of this process as libwolkit keeps it: what `WOLKIT_LANG_CURRENT` lookups read.
    # `Wolkit::I18n` passes its own language id on every lookup, so an app needs this only when C
    # code or a plugin in the same process reads the same catalog.
    module Language
      # The id for a tag (`"pt-BR"`) or POSIX locale (`"pt_BR.UTF-8"`), or nil (show English).
      def self.parse(name : ::String) : Int32?
        id = LibWolkit.wolkit_lang_parse(name.to_unsafe, name.bytesize)
        id < 0 ? nil : id
      end

      # The BCP 47 tag of language id `id`, or nil for an unknown id.
      def self.tag(id : Int32) : ::String?
        ptr = LibWolkit.wolkit_lang_tag(id)
        ptr.null? ? nil : ::String.new(ptr)
      end

      def self.current : Int32
        LibWolkit.wolkit_lang_get
      end

      def self.current=(id : Int32) : Int32
        FFI.check(LibWolkit.wolkit_lang_set(id), Pointer(LibWolkit::Error).null)
        id
      end
    end
  end

  module I18n
    # The catalog of libwolkit as the source `t` reads.
    class Native < Source
      def initialize(@catalog : FFI::Catalog)
      end

      def find(name : ::String) : UInt32?
        @catalog.find(name)
      end

      def count : Int32
        @catalog.count
      end

      def text(key : UInt32, lang : Int32) : ::String
        @catalog.text(key, lang)
      end

      def plural(key : UInt32, n : UInt64, lang : Int32) : ::String
        @catalog.plural(key, n, lang)
      end
    end

    # Use the tables in `dir` (`<unit>.<lang>.wtx`) for every `t` from now on.
    def self.load(dir : ::String, unit : ::String) : Nil
      use Native.new(FFI::Catalog.load_dir(dir, unit))
    end

    # Use tables held in memory (an app that embeds them with `{{ read_file }}`).
    def self.load(tables : Array(Bytes)) : Nil
      use Native.new(FFI::Catalog.load(tables))
    end
  end
end
