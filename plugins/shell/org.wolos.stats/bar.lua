-- SPDX-License-Identifier: GPL-2.0-or-later
-- Menu bar item: "CPU 12%  MEM 41%", refreshed every 5 s. Clicking it opens a popover with
-- details and a button that opens btop. Data comes from `wol sys stats --json` through wol.run
-- (permission "run"): the plugin never reads /proc itself, so it works the same on every box.
local s = { cpu = 0, mem = 0, used = 0, total = 0, uptime = nil, err = nil }

local function read()
  local ok, out = wol.run { "wol", "sys", "stats", "--json", timeout = 5 }
  if not ok then s.err = out; return end
  local v, e = wol.json.decode(out)
  if not v then s.err = e; return end
  s.err = nil
  s.cpu = math.floor((v.cpu_percent or 0) + 0.5)
  s.total = (v.memory and v.memory.total) or 0
  s.used = (v.memory and v.memory.used) or 0
  s.mem = s.total > 0 and math.floor(s.used * 100 / s.total + 0.5) or 0
  s.uptime = v.uptime_secs
end

local function gib(bytes) return string.format("%.1f GiB", bytes / 1073741824) end

local function uptime()
  if not s.uptime then return "Uptime unknown" end
  local h = math.floor(s.uptime / 3600)
  local m = math.floor((s.uptime % 3600) / 60)
  return string.format("Up %d h %02d min", h, m)
end

return wol.menubar {
  refresh = 5,
  render = function()
    read()
    if s.err then return wol.label { "Stats: -" } end
    return wol.row { spacing = 8,
      wol.label { string.format("CPU %d%%", s.cpu) },
      wol.label { string.format("MEM %d%%", s.mem) },
    }
  end,
  popover = function()
    if s.err then
      return wol.stack { spacing = 8,
        wol.label { "Stats", role = "title" },
        wol.label { s.err, role = "secondary" },
      }
    end
    return wol.stack { spacing = 8,
      wol.list {
        wol.item { title = string.format("CPU %d%%", s.cpu), subtitle = uptime(),
                   icon = "utilities-system-monitor-symbolic" },
        wol.item { title = string.format("Memory %d%%", s.mem), subtitle = gib(s.used) .. " of " .. gib(s.total),
                   icon = "drive-harddisk-symbolic" },
      },
      wol.button { label = "Open Top", on_click = function() wol.run { "foot", "-e", "btop" } end },
    }
  end,
}
