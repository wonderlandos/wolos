-- SPDX-License-Identifier: GPL-2.0-or-later
-- Desktop card: CPU as the big figure, memory under it. Same data source as bar.lua.
local s = { cpu = 0, mem = 0, used = 0, total = 0, err = nil }

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
end

local function gib(bytes) return string.format("%.1f", bytes / 1073741824) end

return wol.widget {
  refresh = 10,
  render = function()
    read()
    if s.err then
      return wol.stack { spacing = 4,
        wol.label { "Stats", role = "title" },
        wol.label { s.err, role = "secondary" },
      }
    end
    return wol.stack { spacing = 4,
      wol.label { "CPU", role = "caption" },
      wol.label { string.format("%d%%", s.cpu), role = "display" },
      wol.spacer(),
      wol.label { string.format("Memory %d%% (%s of %s GiB)", s.mem, gib(s.used), gib(s.total)), role = "secondary" },
    }
  end,
}
