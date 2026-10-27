-- WolOS loader hook (loader.lua runs try_include("local") before the menu).
-- The Kernel carousel shows the product name a kernel build wrote to /boot/<dir>/ident
-- (kernel/build.sh: "WolKernel <size of kernel>"). A kernel without that file, or one whose size
-- no longer matches (installed over it later), keeps its directory name.
-- Directories keep their names: pkg, kldxref and freebsd-update expect /boot/kernel.
local menu = require("menu")

-- Quiet boot (live and installed): no menu, no countdown, straight to the WolOS splash.
-- A key held while the machine starts (Space, Esc, any key) keeps the menu up, with no
-- countdown: recovery (boot environments, single user, `set wolos.install=1`).
-- menu.run boots at once, before drawing anything, when autoboot_delay is -1.
if loader.getenv("wolos_boot_menu") ~= "YES" then
	local held = false
	for _ = 1, 4 do
		if io.ischar() then
			held = true
			break
		end
		loader.delay(50000)
	end
	if held then
		while io.ischar() do
			io.getchar()
		end
		loader.setenv("autoboot_delay", "NO")
	else
		loader.setenv("autoboot_delay", "-1")
	end
end
local entry = menu.welcome.all_entries.kernel_options
local dirname = entry.name
local names = {}

local function read_ident(dir)
	local f = io.open("/boot/" .. dir .. "/ident")
	if f == nil then
		return nil
	end
	local text = io.read(f)
	io.close(f)
	local name, size = (text or ""):match("^([%w%-_]+) (%d+)")
	if name and lfs.attributes("/boot/" .. dir .. "/kernel", "size") == tonumber(size) then
		return name
	end
	return nil
end

-- The menu redraws on every key: read each directory once (false = no product name).
local function ident(dir)
	if names[dir] == nil then
		names[dir] = read_ident(dir) or false
	end
	return names[dir] or nil
end

entry.name = function(idx, choice, all_choices)
	local name = choice and ident(choice)
	if name then
		-- kernel -> WolKernel, kernel.old -> WolKernel old
		local suffix = choice:match("^kernel%.(.+)$")
		choice = suffix and (name .. " " .. suffix) or name
	end
	return dirname(idx, choice, all_choices)
end
