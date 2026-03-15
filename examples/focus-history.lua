-- focus-history.lua
--
-- Maintains an ordered history of focused windows. When the focused
-- window is closed, the previously focused window is automatically
-- focused in its place.
--
-- The history is a stack capped at MAX_HISTORY entries. Closing a
-- non-focused window (e.g. closing something in the background) does
-- not affect focus at all — only the focused window being closed
-- triggers a focus transfer.
--
local MAX_HISTORY = 32 -- maximum number of window IDs to remember

local history = {}

local function remove_id(id)
	for i = #history, 1, -1 do
		if history[i] == id then
			table.remove(history, i)
		end
	end
end

local function push_id(id)
	remove_id(id)
	table.insert(history, id)
	if #history > MAX_HISTORY then
		table.remove(history, 1)
	end
end

mplug.add_listener(function(event, state)
	if event.type == "ToplevelUpdated" and event.activated then
		push_id(event.id)
		return
	end

	if event.type == "ToplevelClosed" then
		local closed_id = event.id

		if #history > 0 and history[#history] == closed_id then
			remove_id(closed_id)

			while #history > 0 do
				local candidate = history[#history]
				if state.toplevels[candidate] then
					mplug.focus_window(candidate)
					break
				else
					table.remove(history)
				end
			end
		else
			remove_id(closed_id)
		end
	end
end)
