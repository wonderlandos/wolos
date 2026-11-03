# SPDX-License-Identifier: BSD-2-Clause
# The smart sections of the library: pictures grouped by day (the last week) and month (before), or
# by place. Pure functions of the photos and of "now", so the specs pin every heading.
#
# Concept: a section is a heading, a quiet line under it and the photos, newest first. Why: people
# look for "the ones from last weekend" or "the ones from Lisbon", not for file names. How: dates are
# compared as calendar days in the zone of `now`; places are cells of a tenth of a degree (about
# 11 km north to south), named by the mean position of their pictures, since naming a town would
# need a map the app does not carry and would leak where the person was to whoever served it.
module Gallery
  record Section, id : String, title : String, detail : String, photos : Array(Photo)

  enum Grouping
    Date
    Place
  end

  module Sections
    MONTHS   = %w(January February March April May June July August September October November December)
    WEEKDAYS = %w(Sunday Monday Tuesday Wednesday Thursday Friday Saturday)
    # A place cell is this many degrees on a side.
    CELL = 0.1

    def self.build(photos : Array(Photo), grouping : Grouping, now : Time) : Array(Section)
      grouping.date? ? by_date(photos, now) : by_place(photos)
    end

    # Newest first: one section per day for the last seven days, then one per month.
    def self.by_date(photos : Array(Photo), now : Time) : Array(Section)
      today = day(now)
      groups = {} of String => {String, Array(Photo)}
      order = [] of String
      sorted(photos).each do |p|
        id, title = date_heading(p.date.in(now.location), today)
        unless groups.has_key?(id)
          groups[id] = {title, [] of Photo}
          order << id
        end
        groups[id][1] << p
      end
      order.map do |id|
        title, list = groups[id]
        Section.new(id, title, count(list.size), list)
      end
    end

    # One section per place cell, the one with the newest photo first; pictures with no place last.
    def self.by_place(photos : Array(Photo)) : Array(Section)
      cells = {} of {Int32, Int32} => Array(Photo)
      nowhere = [] of Photo
      sorted(photos).each do |p|
        if pt = p.place
          (cells[cell(pt)] ||= [] of Photo) << p
        else
          nowhere << p
        end
      end
      result = cells.map do |key, list|
        points = list.compact_map(&.place)
        mean = GeoPoint.new(points.sum(&.latitude) / points.size, points.sum(&.longitude) / points.size)
        Section.new("place-#{key[0]}-#{key[1]}", place_name(mean), "#{count(list.size)}, #{span(list)}", list)
      end
      result << Section.new("place-none", "No place", count(nowhere.size), nowhere) unless nowhere.empty?
      result
    end

    # "41.4° N, 2.2° E".
    def self.place_name(p : GeoPoint) : String
      lat = "#{p.latitude.abs.round(1)}° #{p.latitude >= 0 ? 'N' : 'S'}"
      lon = "#{p.longitude.abs.round(1)}° #{p.longitude >= 0 ? 'E' : 'W'}"
      "#{lat}, #{lon}"
    end

    def self.cell(p : GeoPoint) : {Int32, Int32}
      {(p.latitude / CELL).floor.to_i, (p.longitude / CELL).floor.to_i}
    end

    # "1 photo", "12 photos".
    def self.count(n : Int) : String
      n == 1 ? "1 photo" : "#{n} photos"
    end

    # "October 2026", or "September to November 2026", or "December 2025 to January 2026".
    def self.span(list : Array(Photo)) : String
      dates = list.map(&.date)
      a, b = dates.min, dates.max
      first = "#{MONTHS[a.month - 1]} #{a.year}"
      return first if a.year == b.year && a.month == b.month
      return "#{MONTHS[a.month - 1]} to #{MONTHS[b.month - 1]} #{b.year}" if a.year == b.year
      "#{first} to #{MONTHS[b.month - 1]} #{b.year}"
    end

    # The heading of a date: "Today", "Yesterday", a weekday within the week, else "Month Year".
    def self.date_heading(t : Time, today : Time) : {String, String}
      d = day(t)
      ago = (today - d).total_days.round.to_i
      if ago == 0
        {"day-#{d.to_s("%F")}", "Today"}
      elsif ago == 1
        {"day-#{d.to_s("%F")}", "Yesterday"}
      elsif 2 <= ago < 7
        {"day-#{d.to_s("%F")}", WEEKDAYS[d.day_of_week.value % 7]}
      else
        {"month-#{d.to_s("%Y-%m")}", "#{MONTHS[d.month - 1]} #{d.year}"}
      end
    end

    # Newest first; the path breaks ties so the order is the same on every run.
    def self.sorted(photos : Array(Photo)) : Array(Photo)
      photos.sort do |a, b|
        c = b.date <=> a.date
        c == 0 ? a.path <=> b.path : c
      end
    end

    private def self.day(t : Time) : Time
      Time.local(t.year, t.month, t.day, location: t.location)
    end
  end
end
