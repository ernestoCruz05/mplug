local function update_brightness_notification()
    local stdout, _ = mplug.exec("brightnessctl -m")
    if stdout == "" then
        return
    end

    local percent_str = stdout:match(",(%d+)%%")
    local percent = tonumber(percent_str) or 0

    mplug.spawn("dunstify", {
        args = {
            "-t", "1500",
            "-h", "string:x-dunst-stack-tag:brightness",
            "-h", "int:value:" .. percent,
            "-a", "Brightness",
            "Brightness", percent .. "%"
        }
    })
end

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" then
        if event.name == "brightness_up" then
            mplug.exec("brightnessctl set +5%")
            update_brightness_notification()
        elseif event.name == "brightness_down" then
            mplug.exec("brightnessctl set 5%-")
            update_brightness_notification()
        end
    end
end)
