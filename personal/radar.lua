mplug.add_listener(function(event, state)
    if event.type == "UserCommand" and event.name == "radar" then
        local icons = {}
        local _, code_discord = mplug.exec("pgrep -x discord")
        local _, code_vesktop = mplug.exec("pgrep -x vesktop")
        if code_discord == 0 or code_vesktop == 0 then
            table.insert(icons, "")
        end
        local _, code_steam = mplug.exec("pgrep -x steam")
        if code_steam == 0 then
            table.insert(icons, "")
        end
        local _, code_spotify = mplug.exec("pgrep -x spotify")
        if code_spotify == 0 then
            table.insert(icons, "")
        end
        local _, code_easyeffects = mplug.exec("pgrep -x easyeffects")
        if code_easyeffects == 0 then
            table.insert(icons, "")
        end

        if #icons > 0 then
            local output = table.concat(icons, "   ")
            mplug.spawn("dunstify", {
                args = {
                    "-a", "Radar",
                    "-t", "2500",
                    "-h", "string:x-dunst-stack-tag:radar",
                    "Radar",
                    "<span foreground='#FFFFFF' font='12'>" .. output .. "</span>"
                }
            })
        else
            mplug.spawn("dunstify", {
                args = {
                    "-a", "Radar",
                    "-t", "2000",
                    "-h", "string:x-dunst-stack-tag:radar",
                    "Radar",
                    "<span foreground='#FFFFFF'>System Clear</span>"
                }
            })
        end
    end
end)
