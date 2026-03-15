-- urgent-follow.lua
--
-- Automatically switches the view to any tag that has an urgent window
-- (one that is requesting your attention — typically a chat notification
-- or a dialog waiting for input).
--
-- To avoid being disruptive on busy desktops, the jump only fires when
-- the current tag has no active windows of its own. If you are in the
-- middle of work, the notification waits silently until you have an
-- idle moment. Set ALWAYS_JUMP = true to override this and jump
-- unconditionally.
--
-- Configuration:

-- If true, jump to the urgent tag even when the current tag has windows.
-- If false (default), only jump when the current tag is empty.
local ALWAYS_JUMP = false

local COOLDOWN_SECONDS = 3

local last_jump_time = 0

local function now()
	local t, _ = mplug.exec("date +%s")
	return tonumber(t) or 0
end

local function current_tag_has_windows(state)
	for _, tag_num in ipairs(state.active_tags) do
		local info = state.tags[tag_num]
		if info and info.clients > 0 then
			return true
		end
	end
	return false
end

mplug.add_listener(function(event, state)
	if event.type ~= "OutputTag" then
		return
	end
	if event.state ~= 2 then
		return
	end

	for _, active in ipairs(state.active_tags) do
		if active == event.tag then
			return
		end
	end

	if not ALWAYS_JUMP and current_tag_has_windows(state) then
		return
	end

	local t = now()
	if t - last_jump_time < COOLDOWN_SECONDS then
		return
	end
	last_jump_time = t

	local mask = 1 << (event.tag - 1)
	mplug.dispatch("set_tags " .. mask)
end)
