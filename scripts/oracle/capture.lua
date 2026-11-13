-- Game Printer's capture script for Mesen 2 (docs/oracle.md).
--
-- `gp oracle record` runs it in Mesen's test runner mode, in the Mesen image
-- (docker/mesen/Dockerfile), with these paths mounted:
--
--   /gp/in/input.log   the input log to play (docs/formats/recording.md)
--   /gp/in/config      "key value" lines: frames, events (0 or 1)
--   /gp/out/           where the capture goes
--
-- It plays the input log from power-on and writes /gp/out/capture.txt: for
-- frame 0 (power-on) and for each frame after it, the console state at the
-- frame boundary gp-nes uses, the first instruction boundary at or after
-- scanline 241, dot 1 (the start of vertical blank). gp turns the capture
-- into a recording with its own field table; this script only reads what
-- Mesen shows and writes it down. Exit codes (Mesen's, through emu.stop):
-- 0 done, 3 the script failed (the reason is in /gp/out/error.txt).
--
-- Capture format, one record per line:
--   gp-mesen-capture 1
--   frame <n> polled <0|1>
--   s <key> <value>        a value from emu.getState()
--   m <name> <hex>         a memory region, read without side effects
--   e <kind> <scanline> <dot> <addr> <value>   an event in the frame before
--   end <frames>

local OUT = "/gp/out/"
local out = assert(io.open(OUT .. "capture.txt", "w"))

local function fail(message)
  local f = io.open(OUT .. "error.txt", "w")
  if f then
    f:write(tostring(message), "\n")
    f:close()
  end
  out:close()
  emu.stop(3)
end

local function guard(fn)
  return function(...)
    local ok, err = pcall(fn, ...)
    if not ok then
      fail(err)
    end
  end
end

local config = {}
for line in io.lines("/gp/in/config") do
  local k, v = line:match("^(%S+)%s+(%S+)$")
  if k then
    config[k] = v
  end
end
local total = tonumber(config.frames)
local with_events = config.events == "1"

-- The input log: one entry per frame, controller 1 and 2 as Mesen tables.
-- A frame line starting `P!` or `-!` (format 1) presses reset first.
local LETTERS = { "right", "left", "down", "up", "start", "select", "b", "a" }
local inputs = {}
local n = 0
for line in io.lines("/gp/in/input.log") do
  n = n + 1
  if n > 2 then
    local frame = { reset = line:sub(2, 2) == "!" }
    local port = 0
    for field in line:sub(3):gmatch("%S+") do
      local pad = {}
      for i = 1, 8 do
        pad[LETTERS[i]] = field:sub(i, i) ~= "."
      end
      frame[port] = pad
      port = port + 1
    end
    inputs[#inputs + 1] = frame
  end
end
if total == nil or total ~= #inputs then
  fail("config frames " .. tostring(config.frames) .. " but the input log has " .. #inputs)
  return
end

local NONE = { right = false, left = false, down = false, up = false,
               start = false, select = false, b = false, a = false }

-- Mesen 2.1.1's emu.setInput reads the port from its third argument (the
-- subport's place; LuaApi::SetInput reads its parameters one place off),
-- so the port goes there too. Passed only second, every call lands on
-- port 0, and port 2's buttons overwrite port 1's.
local function apply(entry)
  local frame = inputs[entry]
  for port = 0, 1 do
    emu.setInput((frame and frame[port]) or NONE, port, port)
  end
end

-- getState keys worth keeping; the rest are Mesen's bookkeeping.
local KEEP = { "cpu.", "ppu.", "apu.", "controlManager.", "memoryManager.", "frameCount", "masterClock" }
local SKIP = { "ppu.secondarySpriteRam", "ppu.openBusDecayStamp", "ppu.paletteRam", "mapper.chrMemoryOffset" }

local function keep(key)
  for _, p in ipairs(SKIP) do
    if key:sub(1, #p) == p then
      return false
    end
  end
  for _, p in ipairs(KEEP) do
    if key:sub(1, #p) == p then
      return true
    end
  end
  return false
end

local HEX = {}
for i = 0, 255 do
  HEX[i] = string.format("%02x", i)
end

local mt = emu.memType
local REGIONS = {
  { "ram", mt.nesInternalRam },
  { "oam", mt.nesSpriteRam },
  { "palette", mt.nesPaletteRam },
  { "nametables", mt.nesNametableRam },
  { "chr_ram", mt.nesChrRam },
  { "work_ram", mt.nesWorkRam },
  { "save_ram", mt.nesSaveRam },
  { "mapper_ram", mt.nesMapperRam },
}

local function dump_memory(name, memtype)
  local size = emu.getMemorySize(memtype)
  local parts = {}
  for i = 0, size - 1 do
    parts[#parts + 1] = HEX[emu.read(i, memtype)]
  end
  out:write("m ", name, " ", table.concat(parts), "\n")
end

local events = {}
local polled = false

local function capture(frame, state)
  out:write("frame ", frame, " polled ", polled and 1 or 0, "\n")
  local keys = {}
  for k in pairs(state) do
    if keep(k) then
      keys[#keys + 1] = k
    end
  end
  table.sort(keys)
  for _, k in ipairs(keys) do
    out:write("s ", k, " ", tostring(state[k]), "\n")
  end
  for _, r in ipairs(REGIONS) do
    dump_memory(r[1], r[2])
  end
  for _, e in ipairs(events) do
    out:write("e ", e, "\n")
  end
  events = {}
  polled = false
end

out:write("gp-mesen-capture 1\n")
out:write("mesen-region ", tostring(emu.getState().region), "\n")

-- ram_fill fceux: FCEUX's power-on fill of internal RAM, 00 00 00 00 FF FF
-- FF FF repeated, written before frame 0 as gp-nes powers on with it.
if config.ram_fill == "fceux" then
  for addr = 0, 0x7FF do
    emu.write(addr, (addr & 4) ~= 0 and 0xFF or 0x00, emu.memType.nesInternalRam)
  end
end

-- Frame 0: power-on, before any instruction runs.
capture(0, emu.getState())
-- A reset on the first entry is pressed before frame 1, as for any other.
if inputs[1] and inputs[1].reset then
  emu.reset()
end
apply(1)

-- At or past scanline 241, dot 1, the start of vertical blank as Mesen's
-- own PPU counts it. Mesen calls exec callbacks once per instruction, at
-- the boundary before its opcode fetch (Debugger::ProcessInstruction), so
-- the first one past that dot is where the frame ends, as in gp-nes.
local function past_vblank_start(s)
  local line, dot = s["ppu.scanline"], s["ppu.cycle"]
  return (line == 241 and dot >= 1) or (line > 241)
end

local frame = 0
local waiting = false

local on_exec
on_exec = guard(function()
  if not waiting then
    return
  end
  local s = emu.getState()
  if past_vblank_start(s) then
    waiting = false
    frame = frame + 1
    capture(frame, s)
    -- A reset at the start of the next frame. emu.reset() lands a little
    -- later than gp-nes's reset at the frame boundary (docs/open-questions.md,
    -- "Does Mesen's reset land where gp-nes's does?").
    local nxt = inputs[frame + 1]
    if nxt and nxt.reset then
      emu.reset()
    end
    if frame >= total then
      out:write("end ", frame, "\n")
      out:close()
      emu.stop(0)
    end
  end
end)

emu.addMemoryCallback(on_exec, emu.callbackType.exec, 0x0000, 0xFFFF)

emu.addEventCallback(guard(function()
  waiting = true
end), emu.eventType.endFrame)

-- Input for the next frame. Mesen polls its input devices at the start of
-- scanline 241 (nes.inputScanline), one dot before the boundary, so the
-- poll after frame N's own is where frame N+1's input goes.
emu.addEventCallback(guard(function()
  apply(frame + 2)
end), emu.eventType.inputPolled)

local function position()
  local s = emu.getState()
  return s["ppu.scanline"], s["ppu.cycle"]
end

emu.addMemoryCallback(guard(function(addr, value)
  polled = true
  if with_events then
    local line, dot = position()
    events[#events + 1] = string.format("controller-read %d %d %d %d", line, dot, addr, value)
  end
end), emu.callbackType.read, 0x4016, 0x4017)

if with_events then
  -- Mesen raises these after the interrupt sequence, with PC at the
  -- handler, as gp-nes logs them.
  for _, kind in ipairs({ "nmi", "irq" }) do
    emu.addEventCallback(guard(function()
      local s = emu.getState()
      events[#events + 1] = string.format("%s %d %d %d 0", kind, s["ppu.scanline"], s["ppu.cycle"], s["cpu.pc"])
    end), emu.eventType[kind])
  end

  -- OAM DMA's 256 writes to $2004 reach write callbacks too; gp-nes logs
  -- the $4014 write that starts it and not the copy, so skip them.
  local dma_writes = 0

  local function on_write(kind)
    return guard(function(addr, value)
      -- PPU registers repeat every 8 bytes up to $3FFF; gp-nes logs the
      -- register, folded down to $2000-$2007.
      if addr >= 0x2008 and addr <= 0x3FFF then
        addr = 0x2000 + addr % 8
      end
      if addr == 0x4014 then
        dma_writes = 256
      elseif addr == 0x2004 and dma_writes > 0 then
        dma_writes = dma_writes - 1
        return
      end
      local line, dot = position()
      events[#events + 1] = string.format("%s %d %d %d %d", kind, line, dot, addr, value)
    end)
  end
  emu.addMemoryCallback(on_write("ppu-write"), emu.callbackType.write, 0x2000, 0x3FFF)
  emu.addMemoryCallback(on_write("ppu-write"), emu.callbackType.write, 0x4014, 0x4014)
  emu.addMemoryCallback(on_write("controller-write"), emu.callbackType.write, 0x4016, 0x4016)
  emu.addMemoryCallback(on_write("apu-write"), emu.callbackType.write, 0x4000, 0x4013)
  emu.addMemoryCallback(on_write("apu-write"), emu.callbackType.write, 0x4015, 0x4015)
  emu.addMemoryCallback(on_write("apu-write"), emu.callbackType.write, 0x4017, 0x4017)
end
