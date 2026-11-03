# SPDX-License-Identifier: BSD-2-Clause
# Search (wol.h "1.9: search", WOL_1.9): ask Search for apps and Settings pages, or add rows of
# this app's own. A provider's block runs inside dispatch for every query; the reply it fills is
# valid only during that call and takes at most 32 rows (plan-cr1 §3 rule 4). Dispatch often: the
# rows of a slow app are left out.
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./info"

module Wol
  # The rows one query gets back; lent to the provider's block only.
  class SearchReply
    include Scoped

    def initialize(@ptr : LibWol::SearchReply*)
    end

    # Add a row. `key` (up to 128 bytes) comes back to the provider's `on_open` when the person
    # opens the row; `title` and `detail` are one line each. Search shows the app's name with it.
    def add(key : ::String, title : ::String, detail : ::String? = nil) : Nil
      check_valid!
      Wol.call do |err|
        LibWol.wol_search_reply_add(@ptr, Strings.ptr(key), Strings.size(key),
          Strings.ptr(title), Strings.size(title), Strings.ptr(detail), Strings.size(detail), err)
      end
    end
  end

  # Rows of this app in Search, from `Search.provide`. One per context; it lasts until `close`
  # or the context closes.
  class SearchProvider < Handle(LibWol::SearchProvider)
    # The trampolines libwol calls; they capture nothing (see `Callbacks`).
    ON_QUERY = ->(_provider : LibWol::SearchProvider*, reply : LibWol::SearchReply*, text : LibWol::String*, data : Void*) do
      Callbacks.invoke(SearchProvider, data, &.query(reply, text))
      nil
    end
    ON_OPEN = ->(_provider : LibWol::SearchProvider*, key : LibWol::String*, data : Void*) do
      Callbacks.invoke(SearchProvider, data, &.open(key))
      nil
    end

    @on_query : SearchReply, ::String ->
    @on_open : (::String ->)?

    # Use `Search.provide`.
    def initialize(context : Context, @on_query : SearchReply, ::String ->, @on_open : (::String ->)?)
      @context = context
      adopt(Wol.out(LibWol::SearchProvider) do |slot, err|
        LibWol.wol_search_provide(context, ON_QUERY, ON_OPEN, Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    protected def query(raw : LibWol::SearchReply*, text : LibWol::String*) : Nil
      return if closed?
      words = Strings.borrow(text) || ""
      Callbacks.scoped(SearchReply.new(raw)) { |reply| @on_query.call(reply, words) }
    end

    # Always registered (simpler than a NULL function pointer); without `on_open` an opened row
    # does nothing.
    protected def open(key : LibWol::String*) : Nil
      return if closed?
      @on_open.try &.call(Strings.borrow(key) || "")
    end

    protected def release(ptr : LibWol::SearchProvider*) : Nil
      LibWol.wol_search_provider_release(ptr)
    end
  end

  module Search
    # Search the apps and Settings pages for `text`. Key: results (list of { title, kind }).
    # Permission: search.
    def self.query(context : Context, text : ::String) : Info
      Info.copy do |slot, err|
        LibWol.wol_search_query(context, Strings.ptr(text), Strings.size(text), slot, err)
      end
    end

    # Add rows of this app to Search: the block gets the reply and what the person typed, and adds
    # rows. `on_open` gets the key of a row the person opened. Permission: search-provider (the
    # person is told the app sees what they type in Search).
    #
    # ```
    # Wol::Search.provide(ctx, on_open: ->(key : String) { show(key) }) do |reply, text|
    #   notes.matching(text).first(5).each { |n| reply.add(n.id, n.title) }
    # end
    # ```
    def self.provide(context : Context, on_open : (::String ->)? = nil, &on_query : SearchReply, ::String ->) : SearchProvider
      SearchProvider.new(context, on_query, on_open)
    end
  end
end
