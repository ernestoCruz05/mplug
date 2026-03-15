-- powersave.lua
--
-- Turns the display off when the session goes idle and turns it back
-- on when activity resumes. Optionally launches a screen locker
-- before blanking.
--
-- The idle timeout is set by the compositor (MangoWM's idletime config
-- key, in milliseconds). This plugin only decides what to do when that
-- threshold fires.
--
-- Configuration:

-- Set to a locker command string to lock before blanking, or nil to
-- just blank without locking.
--   examples: "swaylock", "swaylock -f -c 000000", "waylock"
local LOCK_CMD = "swaylock -f"

-- If true, the display is turned off after the locker is launched.
-- If false, the locker runs but the display stays on (useful if your
-- locker already handles blanking itself).
local BLANK_DISPLAY = true

local locked = false

mplug.add_listener(function(event, state)
	if event.type == "Idled" and not locked then
		locked = true

		if LOCK_CMD then
			mplug.exec(LOCK_CMD .. " &")
		end

		if BLANK_DISPLAY then
			mplug.set_output_power(false)
		end
	elseif event.type == "IdleResumed" then
		locked = false
		mplug.set_output_power(true)
	end
end)
