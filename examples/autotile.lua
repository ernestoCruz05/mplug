-- autotile.lua
--
-- Automatically picks a layout based on how many windows are on the
-- active tag:
--
--   1 window  → monocle   (full-screen, nothing wasted)
--   2 windows → tile      (master on the left, one slave on the right)
--   3+windows → scroller  (horizontal strip you can scroll through)
--
-- Configuration:
-- The indices below correspond to the position of each layout name in
-- MangoWM's internal layout list. The default ordering is:
--
--   0  tile            6  right_tile
--   1  scroller        7  vertical_scroller
--   2  monocle         8  vertical_grid
--   3  grid            9  vertical_deck
--   4  deck           10  tgmix
--   5  center_tile
--

local LAYOUT_MONOCLE = 2
local LAYOUT_TILE = 0
local LAYOUT_SCROLLER = 1

local last_count = -1

mplug.add_listener(function(event, state)
	local relevant = event.type == "OutputTag" -- tag state / client count changed
		or event.type == "ToplevelUpdated" -- window opened or properties changed
		or event.type == "ToplevelClosed" -- window closed

	if not relevant then
		return
	end

	local total = 0
	for _, tag_num in ipairs(state.active_tags) do
		local info = state.tags[tag_num]
		if info then
			total = total + info.clients
		end
	end

	if total == last_count then
		return
	end
	last_count = total

	if total <= 1 then
		mplug.dispatch("set_layout " .. LAYOUT_MONOCLE)
	elseif total == 2 then
		mplug.dispatch("set_layout " .. LAYOUT_TILE)
	else
		mplug.dispatch("set_layout " .. LAYOUT_SCROLLER)
	end
end)
