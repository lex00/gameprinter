-- The MAME oracle's capture script (docs/oracle-mame.md). MAME runs it with
-- -autoboot_script; it plays an input log from power-on and writes, for
-- every video frame, the main board's RAM, the sound board's RAM, the input
-- ports as the game could read them, a hash of the picture and the sound
-- CPU's writes to its AY-8910s, plus a PNG of the picture every Nth frame.
--
-- Settings come from the environment (`just mame-record` sets them):
--   GP_ORACLE_LOG         the input log (format in docs/oracle-mame.md)
--   GP_ORACLE_OUT         the output folder; it must exist and be empty
--   GP_ORACLE_SHOT_EVERY  write a PNG every N frames; 0 writes none
--
-- What a frame is. MAME ends a video frame when the primary screen's VBLANK
-- starts: video_manager::frame_update finishes the screen's picture and then
-- calls the machine's frame notifiers. Tapper's screen runs at 30 Hz, so
-- that's 30 frames a second. Frame 0 is power-on: the autoboot script runs
-- at emulated time zero, after the reset and before any instruction. Frame
-- n is the state at the n-th frame notifier call, after emulating from the
-- end of frame n-1. Everything for frame n is read in that notifier.
--
-- How inputs line up. MAME's input system runs its own frame notifier first,
-- and only that copies the values a script sets into the ports the game
-- reads. So a value set at the end of frame k is first seen by the game in
-- frame k+2. The script therefore sets the log's frame n+2 at the end of
-- frame n, and the log's frame 1 must hold nothing (the ports at power-on
-- hold their defaults). Each record also carries the ports as the game read
-- them during that frame, taken from MAME, so a mismatch can't hide.

-- MAME runs the autoboot script again after every soft reset (the watchdog's,
-- for one), in the same Lua state. The capture set up the first time goes
-- on across a reset, so a second run does nothing.
if GP_ORACLE_STARTED then
	return
end
GP_ORACLE_STARTED = true

local OUT = assert(os.getenv("GP_ORACLE_OUT"), "GP_ORACLE_OUT is not set")
local LOG = assert(os.getenv("GP_ORACLE_LOG"), "GP_ORACLE_LOG is not set")
local SHOT_EVERY = tonumber(os.getenv("GP_ORACLE_SHOT_EVERY") or "0")
assert(SHOT_EVERY and SHOT_EVERY >= 0 and SHOT_EVERY == math.floor(SHOT_EVERY),
	"GP_ORACLE_SHOT_EVERY must be a whole number")

local machine = manager.machine

local function fail(message)
	-- An error inside a notifier is only logged by MAME, so write it where
	-- the recipe looks and stop the machine.
	local f = io.open(OUT .. "/error.txt", "w")
	if f then
		f:write(message, "\n")
		f:close()
	end
	print("mame-capture: " .. message)
	machine:exit()
	error(message)
end

-- The controls the log can name, per system. Each is a port tag and a bit
-- mask from mcr.cpp's INPUT_PORTS_START( tapper ); all are active low.
local TAPPER = {
	columns = "CSRLDUT",
	-- Columns a log may add after those, in this order.
	optional = "X",
	-- The joystick's columns, which `joystick 4way` sends through MAME's
	-- 4-way filter.
	joystick = "RLDU",
	controls = {
		C = { ":ssio:IP0", 0x01 }, -- coin 1
		S = { ":ssio:IP0", 0x04 }, -- start 1
		R = { ":ssio:IP1", 0x01 }, -- joystick right
		L = { ":ssio:IP1", 0x02 }, -- joystick left
		D = { ":ssio:IP1", 0x04 }, -- joystick down
		U = { ":ssio:IP1", 0x08 }, -- joystick up
		T = { ":ssio:IP1", 0x10 }, -- tap (button 1)
		X = { ":ssio:IP0", 0x20 }, -- tilt
	},
	-- Every port the game reads through the SSIO board, recorded per frame.
	ports = { ":ssio:IP0", ":ssio:IP1", ":ssio:IP2", ":ssio:IP3", ":ssio:IP4" },
	-- The DIP switch port. Its fields must sit at mcr.cpp's defaults.
	dips = ":ssio:IP3",
	-- What goes in ram.bin for each frame, in this order. Main board RAM
	-- from mcr.cpp's cpu_91490_map, then the SSIO board's RAM from
	-- midway_sound.cpp's ssio_map. The palette RAM is write-only on the bus,
	-- so it's read from its memory share instead.
	ram = {
		{ "main e000-e7ff work RAM (nvram)", "space", ":maincpu", 0xe000, 0xe7ff },
		{ "main e800-e9ff sprite RAM", "space", ":maincpu", 0xe800, 0xe9ff },
		{ "main f000-f7ff video RAM", "space", ":maincpu", 0xf000, 0xf7ff },
		{ "main f800-f87f palette RAM", "share", ":paletteram", 0x00, 0x7f },
		{ "ssio 8000-83ff sound CPU RAM", "space", ":ssio:cpu", 0x8000, 0x83ff },
	},
}
local SYSTEMS = { tapper = TAPPER, tapperg = TAPPER, tappera = TAPPER,
	tapperb = TAPPER, sutapper = TAPPER, rbtapper = TAPPER }

local system_name = emu.romname()
local SYSTEM = SYSTEMS[system_name] or fail("no control table for system " .. system_name)

-- The input log.

-- The log's columns: the system's, perhaps with its optional ones.
local COLUMNS
-- Whether the joystick's columns go through MAME's 4-way filter.
local FOUR_WAY = false

local function read_log(path)
	local f = io.open(path, "r") or fail("can't open the input log " .. path)
	local frames = {}
	local n = 0
	local header = {}
	for line in f:lines() do
		n = n + 1
		local text = line:gsub("#.*$", ""):gsub("%s+$", "")
		if n == 1 then
			if text ~= "gp-arcade-input 0" then
				fail(("input log line 1: expected `gp-arcade-input 0`, found `%s`"):format(line))
			end
		elseif text == "" then
			-- blank or comment
		elseif text:match("^system ") then
			header.system = text:match("^system (%S+)$")
		elseif text:match("^joystick ") then
			local mode = text:match("^joystick (%S+)$")
			if mode ~= "4way" and mode ~= "direct" then
				fail(("input log line %d: joystick %s; expected `4way` or `direct`"):format(n, tostring(mode)))
			end
			FOUR_WAY = mode == "4way"
		elseif text:match("^columns ") then
			header.columns = text:match("^columns (%S+)$")
			if header.columns ~= SYSTEM.columns and header.columns ~= SYSTEM.columns .. SYSTEM.optional then
				fail(("the input log's columns are %s, %s's are %s, or %s with the optional ones"):format(
					tostring(header.columns), system_name, SYSTEM.columns, SYSTEM.columns .. SYSTEM.optional))
			end
			COLUMNS = header.columns
		else
			local count, buttons = text:match("^(%d+)%s+(%S+)$")
			if not count then
				fail(("input log line %d: expected `<frames> <buttons>`"):format(n))
			end
			if not COLUMNS then
				fail(("input log line %d: frames before the `columns` line"):format(n))
			end
			if #buttons ~= #COLUMNS then
				fail(("input log line %d: %d buttons, the columns are %s"):format(n, #buttons, COLUMNS))
			end
			for i = 1, #buttons do
				local c = buttons:sub(i, i)
				if c ~= "." and c ~= COLUMNS:sub(i, i) then
					fail(("input log line %d: column %d is `%s`, expected `.` or `%s`"):format(
						n, i, c, COLUMNS:sub(i, i)))
				end
			end
			for _ = 1, tonumber(count) do
				frames[#frames + 1] = buttons
			end
		end
	end
	f:close()
	if header.system ~= system_name then
		fail(("the input log is for system %s, MAME is running %s"):format(
			tostring(header.system), system_name))
	end
	if COLUMNS == nil then
		fail("the input log has no `columns` line")
	end
	if #frames == 0 then
		fail("the input log has no frames")
	end
	if frames[1] ~= string.rep(".", #COLUMNS) then
		fail("the input log's frame 1 must hold nothing: MAME can't apply input before frame 2")
	end
	return frames
end

local frames = read_log(LOG)
local last_frame = #frames

-- Ports and fields.

local ports = machine.ioport.ports
local fields = {}
for column, control in pairs(SYSTEM.controls) do
	local port = ports[control[1]] or fail("no port " .. control[1])
	local found
	for _, field in pairs(port.fields) do
		if field.mask == control[2] then
			found = field
		end
	end
	fields[column] = found or fail(("no field with mask 0x%02x in %s"):format(control[2], control[1]))
end

-- A value set with set_value skips MAME's joystick filter, which only acts
-- on what the field's input sequence says is pressed (ioport_field::
-- frame_update, digital_joystick::frame_update). So with `joystick 4way` a
-- direction is pressed through its sequence instead, as a player's key
-- would be: "NOT KEYCODE_A" reads as pressed (no keyboard is attached), an
-- empty sequence as released.
local input = machine.input
local PRESSED = input:seq_from_tokens("NOT KEYCODE_A")
local RELEASED = input:seq_from_tokens("")

local function apply(buttons)
	for i = 1, #COLUMNS do
		local column = COLUMNS:sub(i, i)
		local on = buttons:sub(i, i) == column
		if FOUR_WAY and SYSTEM.joystick:find(column, 1, true) then
			fields[column]:set_input_seq("standard", on and PRESSED or RELEASED)
		else
			fields[column]:set_value(on and 1 or 0)
		end
	end
end

local function read_ports()
	local values = {}
	for i, tag in ipairs(SYSTEM.ports) do
		values[i] = ports[tag]:read()
	end
	return values
end

-- What the log asked for, read back from the ports: active low, so a
-- pressed control reads as a clear bit.
local function ports_to_buttons(values)
	local out = {}
	for i = 1, #COLUMNS do
		local column = COLUMNS:sub(i, i)
		local control = SYSTEM.controls[column]
		local value
		for j, tag in ipairs(SYSTEM.ports) do
			if tag == control[1] then
				value = values[j]
			end
		end
		out[i] = (value & control[2]) == 0 and column or "."
	end
	return table.concat(out)
end

-- The DIP switches sit at the driver's defaults: the run starts with an
-- empty cfg folder and no ini files, so nothing else can set them. Check it.
-- (type_class raises an error for unused bits, hence the pcall.)
local dips_checked = 0
for _, field in pairs(ports[SYSTEM.dips].fields) do
	local ok, class = pcall(function() return field.type_class end)
	if ok and class == "dipswitch" then
		dips_checked = dips_checked + 1
	end
	if ok and class == "dipswitch" and field.user_value ~= field.defvalue then
		fail(("DIP switch %s is 0x%x, its default is 0x%x"):format(field.name, field.user_value, field.defvalue))
	end
end
if dips_checked == 0 then
	fail("found no DIP switches in " .. SYSTEM.dips)
end

-- Memory.

local readers = {}
local ram_layout = {}
local ram_offset = 0
for _, r in ipairs(SYSTEM.ram) do
	local name, kind, tag, first, last = r[1], r[2], r[3], r[4], r[5]
	local reader
	if kind == "space" then
		local cpu = machine.devices[tag] or fail("no device " .. tag)
		local space = cpu.spaces["program"]
		reader = function() return space:read_range(first, last, 8) end
	else
		local share = machine.memory.shares[tag] or fail("no memory share " .. tag)
		reader = function()
			local bytes = {}
			for i = first, last do
				bytes[#bytes + 1] = string.char(share:read_u8(i))
			end
			return table.concat(bytes)
		end
	end
	readers[#readers + 1] = reader
	local size = last - first + 1
	ram_layout[#ram_layout + 1] = ("%5d %5d %s"):format(ram_offset, size, name)
	ram_offset = ram_offset + size
end
local RAM_RECORD = ram_offset

local function read_ram()
	local parts = {}
	for i, reader in ipairs(readers) do
		parts[i] = reader()
	end
	local ram = table.concat(parts)
	if #ram ~= RAM_RECORD then
		fail(("read %d bytes of RAM, expected %d"):format(#ram, RAM_RECORD))
	end
	return ram
end

-- The picture. video:snapshot_pixels() draws the frame MAME just finished,
-- as -snapview native and -nosnapbilinear set it up: one 32-bit pixel per
-- screen pixel, 0xAARRGGBB, rows top to bottom. The hash is FNV-1a over
-- 64 bits, fed one pixel at a time as the 24-bit number 0xRRGGBB (alpha
-- left out), not byte by byte: h starts at 0xcbf29ce484222325, and for each
-- pixel h = (h XOR rgb) * 0x100000001b3, modulo 2^64.

local video = machine.video
local screen = machine.screens[":screen"] or fail("no screen :screen")

local FNV_OFFSET = 0xcbf29ce484222325
local FNV_PRIME = 0x100000001b3
local unpack = string.unpack
local SIXTEEN = "<" .. string.rep("I4", 16)

local function hash_pixels(s)
	local h = FNV_OFFSET
	local n = #s
	local pos = 1
	local M = 0xffffff
	local P = FNV_PRIME
	while pos + 63 <= n do
		local a, b, c, d, e, f, g, i, j, k, l, m, o, p, q, r = unpack(SIXTEEN, s, pos)
		h = (h ~ (a & M)) * P; h = (h ~ (b & M)) * P; h = (h ~ (c & M)) * P; h = (h ~ (d & M)) * P
		h = (h ~ (e & M)) * P; h = (h ~ (f & M)) * P; h = (h ~ (g & M)) * P; h = (h ~ (i & M)) * P
		h = (h ~ (j & M)) * P; h = (h ~ (k & M)) * P; h = (h ~ (l & M)) * P; h = (h ~ (m & M)) * P
		h = (h ~ (o & M)) * P; h = (h ~ (p & M)) * P; h = (h ~ (q & M)) * P; h = (h ~ (r & M)) * P
		pos = pos + 64
	end
	while pos + 3 <= n do
		local a = unpack("<I4", s, pos)
		h = (h ~ (a & M)) * P
		pos = pos + 4
	end
	return h
end

local width, height = video:snapshot_size()

-- Output files.

local function open(name, mode)
	return io.open(OUT .. "/" .. name, mode) or fail("can't write " .. OUT .. "/" .. name)
end

local ram_file = open("ram.bin", "wb")
local frames_file = open("frames.tsv", "w")
frames_file:write("frame\ttime\tbuttons\tip0\tip1\tip2\tip3\tip4\tpicture\tmain_pc\tssio_pc\tshot\n")
local ay_file = open("ay.tsv", "w")
ay_file:write("frame\twrites\n")

local maincpu = machine.devices[":maincpu"]
local ssiocpu = machine.devices[":ssio:cpu"]

-- The sound CPU's writes to its two AY-8910s, in order, a frame at a
-- time: midway.cpp's ssio_map puts chip 0 at a000 and chip 1 at b000, each
-- with its address latch at offset 0 and its data latch at offset 2,
-- mirrored over 0ffc. Each is written as the chip, `a` or `d`, and the
-- value in hex: `0a07 0d38` selects chip 0's register 7 and writes 38.
local ay_writes = {}
AY_TAP = ssiocpu.spaces["program"]:install_write_tap(0xa000, 0xbfff, "gp-ay", function(offset, data, mask)
	local latch = offset & 3
	if latch == 0 or latch == 2 then
		ay_writes[#ay_writes + 1] = ("%d%s%02x"):format(offset >= 0xb000 and 1 or 0,
			latch == 0 and "a" or "d", data & 0xff)
	end
end)

local function pc(cpu)
	return ("%04x"):format(cpu.state["PC"].value)
end

local function shot_name(n)
	return ("frame-%06d.png"):format(n)
end

-- The ports the game reads during the next frame: read at the end of each
-- frame, after MAME's input notifier has run.
local live_ports
local mismatches = 0

local function record(n)
	local t = machine.time
	local pixels = video:snapshot_pixels()
	local shot = "-"
	if SHOT_EVERY > 0 and n % SHOT_EVERY == 0 then
		shot = "shots/" .. shot_name(n)
		local err = screen:snapshot(shot_name(n))
		if err then
			fail("couldn't write " .. shot .. ": " .. tostring(err))
		end
	end
	ram_file:write(read_ram())
	ay_file:write(n, "\t", #ay_writes > 0 and table.concat(ay_writes, " ") or "-", "\n")
	ay_writes = {}
	local buttons = n == 0 and "-" or ports_to_buttons(live_ports)
	if n > 0 and buttons ~= frames[n] then
		mismatches = mismatches + 1
	end
	frames_file:write(("%d\t%d.%018d\t%s\t%02x\t%02x\t%02x\t%02x\t%02x\t%016x\t%s\t%s\t%s\n"):format(
		n, t.seconds, t.attoseconds, buttons,
		live_ports[1], live_ports[2], live_ports[3], live_ports[4], live_ports[5],
		hash_pixels(pixels), pc(maincpu), pc(ssiocpu), shot))
end

local function finish()
	ram_file:close()
	frames_file:close()
	ay_file:close()
	local meta = open("capture.txt", "w")
	meta:write("gp-mame-capture 0\n")
	meta:write("mame ", emu.app_version(), "\n")
	meta:write("system ", system_name, "\n")
	meta:write("frames ", last_frame + 1, "\n")
	meta:write("refresh_hz ", ("%.6f"):format(screen.refresh), "\n")
	meta:write("picture ", width, "x", height, " fnv1a64-rgb24\n")
	meta:write("shot_every ", SHOT_EVERY, "\n")
	meta:write("columns ", COLUMNS, "\n")
	meta:write("joystick ", FOUR_WAY and "4way" or "direct", "\n")
	meta:write("dip_switches defaults ", dips_checked, "\n")
	meta:write("input_mismatches ", mismatches, "\n")
	meta:write("ram_record ", RAM_RECORD, "\n")
	for _, line in ipairs(ram_layout) do
		meta:write("ram ", line, "\n")
	end
	meta:close()
	machine:exit()
end

-- Frame 0: power-on. The ports the game reads in frame 1 are their
-- defaults; the log's frame 2 is set now, so MAME's input notifier copies
-- it into the ports at the end of frame 1.
live_ports = read_ports()
record(0)
if last_frame >= 2 then
	apply(frames[2])
end

local frame = 0
local done = false
FRAME_SUBSCRIPTION = emu.add_machine_frame_notifier(function()
	if done then
		return
	end
	frame = frame + 1
	record(frame)
	if frame >= last_frame then
		done = true
		finish()
		return
	end
	live_ports = read_ports()
	if frames[frame + 2] then
		apply(frames[frame + 2])
	end
end)
