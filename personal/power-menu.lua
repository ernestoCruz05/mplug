local function create_button(tag, icon, text, command)
    mplug.spawn("dunstify", {
        args = {
            "-a", "PowerMenu",
            "-t", "5000",
            "-h", "string:x-dunst-stack-tag:" .. tag,
            "-A", "run,Execute",
            " ",
            string.format("<span font='10'>%s</span>   <span font='10'>%s</span>", icon, text)
        },
        on_stdout = function(line)
            local action = line:gsub("%s+", "")
            if action == "run" then
                mplug.exec("dunstctl close-all")
                mplug.exec(command)
            end
        end
    })
end

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" and event.name == "power_menu" then
        mplug.exec("dunstctl close-all")
        create_button("menu_shutdown", "", "Shutdown", "systemctl poweroff")
        mplug.after(20, function()
            create_button("menu_reboot", "󰜉", "Reboot", "systemctl reboot")
        end)
        mplug.after(40, function()
            create_button("menu_lock", "", "Lock Screen", "hyprlock")
        end)
        mplug.after(60, function()
            local session_id = os.getenv("XDG_SESSION_ID") or ""
            create_button("menu_logout", "󰗽", "Log Out", "loginctl terminate-session " .. session_id)
        end)
    end
end)
