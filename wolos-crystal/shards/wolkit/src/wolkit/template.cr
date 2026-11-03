# SPDX-License-Identifier: BSD-2-Clause
# Template: turns wolos-crystal/templates/app into a new project (`wol app new todos --template app`).
# Files ending in `.tmpl` are copied without the suffix, with `{{name}}`, `{{class_name}}` and
# `{{app_id}}` replaced; other files are copied as they are. The templates carry the `.tmpl` suffix so
# the repository's Crystal checks (which read every `.cr` file) do not see `require "wolkit"` as an
# unknown dependency; the shard's template spec builds a generated project to keep them honest.
module Wolkit
  module Template
    # The name must be a lower-case word with digits, `_` or `-`, so it is safe as a directory,
    # a target and a class (RCR-190).
    NAME = /\A[a-z][a-z0-9_-]{0,31}\z/

    # `todos` -> `Todos`, `my-notes` -> `MyNotes`.
    def self.class_name(name : String) : String
      name.split(/[-_]/).map(&.capitalize).join
    end

    # Where the templates are: `$WOLKIT_TEMPLATES`, else next to the shard in a checkout.
    def self.directory : String
      ENV["WOLKIT_TEMPLATES"]? || File.expand_path("../../../../templates", __DIR__)
    end

    # Write the project `name` into `target` (which must not exist).
    def self.render(name : String, target : String, template : String = "app") : Array(String)
      raise ArgumentError.new("A project name is lower-case letters, digits, '_' or '-', and starts with a letter; #{name.inspect} is not.") unless NAME.matches?(name)
      raise ArgumentError.new("#{target} exists already; choose another name.") if File.exists?(target)
      source = File.join(directory, template)
      raise ArgumentError.new("There is no template #{template.inspect} in #{directory}.") unless Dir.exists?(source)
      values = {"name" => name, "class_name" => class_name(name), "app_id" => "org.wolos.#{name.delete("_-")}"}
      written = [] of String
      each_file(source) do |relative|
        text = File.read(File.join(source, relative))
        destination = relative.ends_with?(".tmpl") ? relative.rchop(".tmpl") : relative
        path = File.join(target, destination)
        Dir.mkdir_p(File.dirname(path))
        File.write(path, relative.ends_with?(".tmpl") ? fill(text, values) : text)
        written << destination
      end
      written.sort
    end

    private def self.fill(text : String, values : Hash(String, String)) : String
      text.gsub(/\{\{(\w+)\}\}/) { |match| values[$1]? || match }
    end

    private def self.each_file(root : String, prefix : String = "", &block : String ->) : Nil
      Dir.each_child(File.join(root, prefix)) do |entry|
        relative = prefix.empty? ? entry : File.join(prefix, entry)
        if File.directory?(File.join(root, relative))
          each_file(root, relative, &block)
        else
          block.call(relative)
        end
      end
    end
  end
end
