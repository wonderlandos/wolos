-- SPDX-License-Identifier: BSD-2-Clause
-- Clock: the time, big, and the date under it. Copy to ~/.config/wolos/widgets/ to change it;
-- the desktop reloads it when the file changes.
local days = { "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday" }
local months = { "January", "February", "March", "April", "May", "June", "July", "August",
  "September", "October", "November", "December" }

return wol.widget {
  name = "Clock",
  refresh = 60,
  render = function()
    local t = wol.now()
    return wol.stack {
      spacing = 4,
      wol.label { string.format("%02d:%02d", t.hour, t.minute), role = "display" },
      wol.label { string.format("%s, %d %s", days[t.weekday], t.day, months[t.month]), role = "secondary" },
    }
  end,
}
