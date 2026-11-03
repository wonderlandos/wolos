-- WolOS tiling shortcuts (the Lua form of tiling.conf). Not loaded directly: while System Settings >
-- Desktop & Dock > Tile windows is on, Settings binds these (with any changes from Keyboard >
-- Shortcuts) in the user's wolos/shortcuts file.
local mod = "SUPER"

hl.bind(mod .. " + ALT + left", hl.dsp.focus({ direction = "left" }))
hl.bind(mod .. " + ALT + right", hl.dsp.focus({ direction = "right" }))
hl.bind(mod .. " + ALT + up", hl.dsp.focus({ direction = "up" }))
hl.bind(mod .. " + ALT + down", hl.dsp.focus({ direction = "down" }))
hl.bind(mod .. " + SHIFT + left", hl.dsp.window.move({ direction = "left" }))
hl.bind(mod .. " + SHIFT + right", hl.dsp.window.move({ direction = "right" }))
hl.bind(mod .. " + SHIFT + up", hl.dsp.window.move({ direction = "up" }))
hl.bind(mod .. " + SHIFT + down", hl.dsp.window.move({ direction = "down" }))
hl.bind(mod .. " + CTRL + left", hl.dsp.window.resize({ x = -40, y = 0, relative = true }), { repeating = true })
hl.bind(mod .. " + CTRL + right", hl.dsp.window.resize({ x = 40, y = 0, relative = true }), { repeating = true })
hl.bind(mod .. " + CTRL + up", hl.dsp.window.resize({ x = 0, y = -40, relative = true }), { repeating = true })
hl.bind(mod .. " + CTRL + down", hl.dsp.window.resize({ x = 0, y = 40, relative = true }), { repeating = true })
-- Float one window out of the tiling (⌘F is Find in apps).
hl.bind(mod .. " + ALT + F", hl.dsp.window.float({ action = "toggle" }))
