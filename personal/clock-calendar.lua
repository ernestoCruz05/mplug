local function highlight_calendar(cal_text)
	local day = tonumber(os.date("%d"))
	if not day then
		return cal_text
	end
	local lines = {}
	for line in cal_text:gmatch("[^\r\n]+") do
		table.insert(lines, line)
	end

	local day_str = tostring(day)
	for i = 3, #lines do
		local line = lines[i]
		local pattern
		if day < 10 then
			pattern = "%s(" .. day_str .. ")%f[%D]"
		else
			pattern = "%f[%d](" .. day_str .. ")%f[%D]"
		end

		local new_line, count = line:gsub(pattern, function(matched)
			return "<span background='#5E81AC' foreground='#ECEFF4'><b>" .. matched .. "</b></span>"
		end)

		if count > 0 then
			lines[i] = new_line
			break
		end
	end

	return table.concat(lines, "\n")
end

local function show_clock()
	local time_str, _ = mplug.exec("date +'%H:%M  //  %A, %b %d'")
	mplug.spawn("dunstify", {
		args = {
			"-t",
			"3000",
			"-h",
			"string:x-dunst-stack-tag:clock-calendar",
			"-a",
			"Clock",
			"Clock",
			time_str,
		},
	})
end

local function show_calendar()
	local cal_raw, _ = mplug.exec("cal")
	local cal_highlighted = highlight_calendar(cal_raw)
	mplug.spawn("dunstify", {
		args = {
			"-t",
			"5000",
			"-h",
			"string:x-dunst-stack-tag:clock-calendar",
			"-a",
			"Calendar",
			"Calendar",
			"<span font='monospace'>" .. cal_highlighted .. "</span>",
		},
	})
end

local pending_timer = nil

mplug.add_listener(function(event, state)
	if event.type == "UserCommand" then
		if event.name == "clock" then
			show_clock()
		elseif event.name == "calendar" then
			show_calendar()
		elseif event.name == "clock_calendar" then
			if pending_timer then
				pending_timer:cancel()
				pending_timer = nil
				show_calendar()
			else
				show_clock()
				pending_timer = mplug.after(3000, function()
					pending_timer = nil
				end)
			end
		end
	end
end)
