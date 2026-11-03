# SPDX-License-Identifier: BSD-2-Clause
# Wolkit::I18n: the words an app shows, in the person's language (ADR 0045). `t("key")` returns the
# text of a key of the app's catalog; the catalog is the compiled tables of `locale/<unit>/`
# (`wol-i18n compile`), read through libwolkit, the same tables the shell uses.
#
# Concept: `t("sound.louder")` reads the language signal, so a `text t("sound.louder")` in a view
# is an effect and relabels itself when `Wolkit::I18n.language = "de"` runs. Why: the person
# changes language in Settings with the app open.
# How: a `Source` answers `find(name)` (constant time) and `text(key, lang)`; this file keeps one
# String per (key, language) after the first use, so a label read again allocates nothing. A key the
# catalog lacks shows its own name (and `Wolkit::I18n.missing` counts it), never an exception: a
# missing key_name must not stop an app from opening.
require "./reactive/signal"

module Wolkit
  module I18n
    # The language ids of `wolkit.h` (WOLKIT_LANG_*), in id order.
    TAGS = %w(en pt-BR es fr de ja ko zh-Hans id tl xx)

    # One unit's text. `FFI::Catalog` is the real one (libwolkit); specs use `Table`.
    abstract class Source
      # The key named `name`, or nil.
      abstract def find(name : ::String) : UInt32?
      # How many keys there are.
      abstract def count : Int32
      # The text of `key` in language id `lang`, falling back to English.
      abstract def text(key : UInt32, lang : Int32) : ::String
      # The plural form of `key` for `n` in language id `lang`.
      abstract def plural(key : UInt32, n : UInt64, lang : Int32) : ::String
    end

    # A source held in memory as `{ language tag => { key name => text } }`, for specs and tools.
    # English defines the keys; a plural key has a `name#one`, `name#other` text per form and the
    # count picks `one` for 1 and `other` otherwise (the real tables follow each language's rules).
    class Table < Source
      @names = [] of ::String

      def initialize(@texts : Hash(::String, Hash(::String, ::String)))
        @texts["en"].each_key do |k|
          name = k.partition('#')[0]
          @names << name unless @names.includes?(name)
        end
      end

      def find(name : ::String) : UInt32?
        @names.index(name).try(&.to_u32)
      end

      def count : Int32
        @names.size
      end

      def text(key : UInt32, lang : Int32) : ::String
        pick(@names[key], lang, "") || pick(@names[key], 0, "") || pick(@names[key], 0, "#other") || ""
      end

      def plural(key : UInt32, n : UInt64, lang : Int32) : ::String
        form = n == 1 ? "#one" : "#other"
        pick(@names[key], lang, form) || pick(@names[key], 0, form) || text(key, lang)
      end

      private def pick(name : ::String, lang : Int32, form : ::String) : ::String?
        @texts[TAGS[lang]]?.try(&.[name + form]?)
      end
    end

    @@source : Source? = nil
    @@language = Signal(Int32).new(0)
    @@cache = [] of ::String?
    @@missing = 0

    # Use `source` for every `t` from now on.
    def self.use(source : Source) : Nil
      @@source = source
      @@cache = Array(::String?).new(source.count * TAGS.size, nil)
    end

    # How many lookups named a key the catalog lacks (a test or a dev tool asserts it is 0).
    def self.missing : Int32
      @@missing
    end

    # The language id in use. Inside an effect this records a dependency.
    def self.language_id : Int32
      @@language.value
    end

    # The BCP 47 tag of the language in use (`"pt-BR"`).
    def self.language : ::String
      TAGS[language_id]
    end

    # Show text in the language `tag` names (`"de"`, `"pt-BR"`) or a POSIX locale (`"pt_BR.UTF-8"`);
    # returns false and keeps the language for one WolOS has no translation of.
    def self.language=(tag : ::String) : ::String
      id = parse(tag)
      @@language.value = id if id
      tag
    end

    # The language id for a tag (`"pt-BR"`, `"xx"`) or POSIX locale (`"pt_BR.UTF-8"`, `"zh_CN"`), or
    # nil for a language without a translation (show English). The same rules as `wolkit_lang_parse`.
    def self.parse(name : ::String) : Int32?
      if (i = TAGS.index(name))
        return i
      end
      base = name.split('.').first.split('@').first.tr("-", "_")
      return 0 if base == "C" || base == "POSIX"
      code, _, region = base.partition('_')
      case code
      when "zh"  then region.in?("", "CN", "SG") ? TAGS.index("zh-Hans") : nil
      when "pt"  then TAGS.index("pt-BR")
      when "fil" then TAGS.index("tl")
      else
        i = TAGS.index(code)
        i == TAGS.size - 1 ? nil : i
      end
    end

    # The text of `key_name` in the language in use. A key the catalog lacks reads as its own key_name.
    def self.t(key_name : ::String) : ::String
      cached(key_name)
    end

    private def self.cached(key_name : ::String) : ::String
      lang = @@language.value
      source = @@source
      return key_name unless source
      key = source.find(key_name)
      unless key
        @@missing += 1
        return key_name
      end
      slot = key.to_i * TAGS.size + lang
      @@cache[slot] ||= source.text(key, lang)
    end

    # The plural form of `key_name` for `n`, `{n}` filled in.
    def self.t(key_name : ::String, n : Int, **args) : ::String
      source = @@source
      lang = @@language.value
      return key_name unless source
      key = source.find(key_name)
      unless key
        @@missing += 1
        return key_name
      end
      fill(source.plural(key, n.to_u64, lang), args.merge({n: n}))
    end

    # `key_name` with each `{placeholder}` replaced by its value (`t("join", key_name: "Rabbit Hole")`).
    def self.t(key_name : ::String, **args) : ::String
      fill(cached(key_name), args)
    end

    # `template` with each `{name}` replaced by its value in `args`; `{{` and `}}` are braces and a
    # name not in `args` stays as written.
    def self.fill(template : ::String, args : NamedTuple | Hash) : ::String
      return template if args.empty? && !template.includes?('{') && !template.includes?('}')
      ::String.build do |io|
        i = 0
        while i < template.size
          c = template[i]
          if (c == '{' || c == '}') && template[i + 1]? == c
            io << c
            i += 2
          elsif c == '{' && (close = template.index('}', i))
            name = template[i + 1...close]
            found = false
            args.each do |k, v|
              if k.to_s == name
                io << v
                found = true
                break
              end
            end
            io << template[i..close] unless found
            i = close + 1
          else
            io << c
            i += 1
          end
        end
      end
    end
  end

  module DSL
    # The text of a key of the app's catalog in the person's language: `text t("sound.louder")`.
    # Reads the language signal, so the element relabels itself when the language changes.
    def t(key_name : ::String, **args) : ::String
      I18n.t(key_name, **args)
    end

    # The plural form for `n`: `t("files.count", n: 3)` is "3 files".
    def t(key_name : ::String, n : Int, **args) : ::String
      I18n.t(key_name, n, **args)
    end
  end
end
