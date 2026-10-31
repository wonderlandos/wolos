# SPDX-License-Identifier: BSD-2-Clause
module Wopkg
  class UsageError < Exception
  end

  # Parsed command line. Plain words (`install`, `search`) and pacman's short forms (`-S`, `-Ss`,
  # `-Syu`, `-R`, `-Qi`) give the same commands.
  class Args
    property verb : String = "help"
    property targets = [] of String
    property json = false
    property yes = false
    property check = false
    property local = false
    property from : String? = nil
    property to : String? = nil
    property only : Array(String)? = nil

    VERBS = {
      "install" => "install", "add" => "install", "i" => "install",
      "remove" => "remove", "rm" => "remove", "uninstall" => "remove",
      "update" => "update", "upgrade" => "update", "up" => "update",
      "search" => "search", "s" => "search", "find" => "search",
      "info" => "info", "show" => "info",
      "list" => "list", "ls" => "list",
      "sync" => "sync", "refresh" => "sync",
      "sources" => "sources", "history" => "history",
      "rollback" => "rollback", "undo" => "rollback",
      "channel" => "channel", "pause" => "pause", "resume" => "resume",
      "appimage" => "appimage", "aur" => "aur", "recipe" => "recipe",
      "completions" => "completions", "__complete" => "__complete", "__fetch" => "__fetch",
      "version" => "version", "help" => "help",
    }

    # pacman operation + modifiers -> verb (and implied flags).
    def self.pacman(op : String, a : Args) : String
      letters = op.lchop('-').chars
      main = letters.first? || raise UsageError.new("Unknown option #{op}.")
      mods = letters[1..]? || [] of Char
      case main
      when 'S'
        return "search" if mods.includes?('s')
        return "info" if mods.includes?('i')
        return "update" if mods.includes?('u')
        return "sync" if mods.includes?('y')
        "install"
      when 'R'
        "remove"
      when 'Q'
        return "info" if mods.includes?('i')
        if mods.includes?('u')
          a.check = true
          return "update"
        end
        "list"
      when 'U'
        "install"
      else
        raise UsageError.new("Unknown option #{op}. Try `wopkg help`.")
      end
    end

    def self.parse(argv : Array(String)) : Args
      a = Args.new
      verb : String? = nil
      i = 0
      while i < argv.size
        w = argv[i]
        case w
        when "--json"           then a.json = true
        when "-y", "--yes", "--noconfirm" then a.yes = true
        when "--check", "-c"    then a.check = true
        when "--local"          then a.local = true
        when "--from", "--to", "--only"
          v = argv[i + 1]? || raise UsageError.new("#{w} needs a value.")
          i += 1
          case w
          when "--from" then a.from = v
          when "--to"   then a.to = v
          else               a.only = v.split(',').map(&.strip).reject(&.empty?)
          end
        when "-h", "--help" then verb ||= "help"
        when "-V", "--version" then verb ||= "version"
        when "--"
          a.targets.concat(argv[(i + 1)..])
          break
        else
          if verb.nil? && w.starts_with?('-') && w.size >= 2 && w[1].uppercase?
            verb = pacman(w, a)
          elsif w.starts_with?("--")
            raise UsageError.new("Unknown option #{w}. Try `wopkg help`.")
          elsif verb.nil?
            verb = VERBS[w]? || raise UsageError.new("“#{w}” is not a wopkg command. Try `wopkg help`.")
          else
            a.targets << w
          end
        end
        i += 1
      end
      a.verb = verb || "help"
      if a.to && a.verb != "update"
        raise UsageError.new("--to goes with `wopkg update`.")
      end
      if %w(install remove info search rollback).includes?(a.verb) && a.targets.empty?
        raise UsageError.new("`wopkg #{a.verb}` needs #{a.verb == "search" ? "something to search for" : "a name"}.")
      end
      a
    end
  end
end
