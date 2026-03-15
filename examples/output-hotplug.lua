-- output-hotplug.lua
--
-- Automatically configures a monitor the moment it is connected,
-- without restarting the compositor or editing config files.
--
-- Default behaviour for a newly connected monitor:
--   1. Enable it.
--   2. Position it immediately to the right of the rightmost
--      currently active output.
--   3. Apply the configured default scale.
--
-- Per-monitor overrides can be added to the OVERRIDES table keyed by
-- the connector name (e.g. "HDMI-A-1", "DP-2"). Each entry can
-- specify any subset of the supported fields; missing fields fall back
-- to the defaults.
--

local DEFAULT_SCALE = 1.0

-- Optional per-monitor overrides keyed by connector name.
-- Supported fields: scale (number), position_x (integer),
--                   position_y (integer), enabled (boolean).
local OVERRIDES = {
	-- ["HDMI-A-1"] = { scale = 1.0 },
	-- ["eDP-1"]    = { scale = 2.0 },
	-- ["DP-2"]     = { scale = 1.0, position_x = 3840, position_y = 0 },
}

local known_heads = {}

local function rightmost_x(outputs)
	local max_x = 0
	for _, out in ipairs(outputs) do
		if out.enabled then
			local right_edge = out.x + out.width_px
			if right_edge > max_x then
				max_x = right_edge
			end
		end
	end
	return max_x
end

mplug.add_listener(function(event, state)
	if event.type == "OutputHeadUpdated" then
		if known_heads[event.id] then
			return
		end
		known_heads[event.id] = true

		if event.enabled then
			return
		end

		local override = OVERRIDES[event.name] or {}

		local scale = override.scale or DEFAULT_SCALE

		local pos_x = override.position_x
		local pos_y = override.position_y or 0

		if pos_x == nil then
			pos_x = rightmost_x(state.outputs)
		end

		mplug.set_output_enabled(event.name, override.enabled ~= false)
		mplug.set_output_scale(event.name, scale)
		mplug.set_output_position(event.name, pos_x, pos_y)
	elseif event.type == "OutputHeadRemoved" then
		known_heads[event.id] = nil
	end
end)
