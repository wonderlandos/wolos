# SPDX-License-Identifier: GPL-2.0-or-later
# A project file that every rule wol app check runs accepts: documented public methods, private
# helpers without comments, an annotation between a doc comment and its method.
module Good
  # The greeting for `name`.
  def self.greet(name : String) : String
    "Hello, #{name}! #{helper}"
  end

  # Inlined, still documented.
  @[AlwaysInline]
  def self.answer : Int32
    42
  end

  private def self.helper : String
    "def in a string is not a method"
  end
end
