local notified_low = false
local last_net_state = nil

local function check_metrics()
    local capacity_str, _ = mplug.exec("cat /sys/class/power_supply/BAT*/capacity 2>/dev/null | head -n 1")
    local status_str, _ = mplug.exec("cat /sys/class/power_supply/BAT*/status 2>/dev/null | head -n 1")
    
    if capacity_str ~= "" and status_str ~= "" then
        local capacity = tonumber((capacity_str:gsub("%s+", ""))) or 0
        local status = status_str:gsub("%s+", "")
        
        if status == "Discharging" and capacity <= 15 then
            if not notified_low then
                notified_low = true
                mplug.spawn("dunstify", {
                    args = {
                        "-u", "critical",
                        "-t", "0",
                        "-h", "string:x-dunst-stack-tag:battery",
                        "-a", "Battery",
                        "Battery Low",
                        capacity .. "% remaining!"
                    }
                })
            end
        else
            notified_low = false
        end
    end

    local _, exit_code = mplug.exec("ping -c 1 -W 1 1.1.1.1 2>/dev/null")
    local current_net_state = (exit_code == 0) and "up" or "down"

    if last_net_state ~= nil and current_net_state ~= last_net_state then
        if current_net_state == "up" then
            mplug.spawn("dunstify", {
                args = {
                    "-t", "3000",
                    "-h", "string:x-dunst-stack-tag:network",
                    "-a", "Network",
                    "Network", "Connection Restored"
                }
            })
        else
            mplug.spawn("dunstify", {
                args = {
                    "-u", "critical",
                    "-t", "3000",
                    "-h", "string:x-dunst-stack-tag:network",
                    "-a", "Network",
                    "Network", "Connection Lost"
                }
            })
        end
    end
    last_net_state = current_net_state
end

mplug.every(10000, check_metrics)

mplug.after(2000, function()
    local _, exit_code = mplug.exec("ping -c 1 -W 1 1.1.1.1 2>/dev/null")
    last_net_state = (exit_code == 0) and "up" or "down"
end)

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" then
        if event.name == "battery" then
            local capacity_str, _ = mplug.exec("cat /sys/class/power_supply/BAT*/capacity 2>/dev/null | head -n 1")
            local status_str, _ = mplug.exec("cat /sys/class/power_supply/BAT*/status 2>/dev/null | head -n 1")
            
            if capacity_str ~= "" then
                local capacity = capacity_str:gsub("%s+", "")
                local status = status_str:gsub("%s+", "")
                mplug.spawn("dunstify", {
                    args = {
                        "-t", "2000",
                        "-h", "string:x-dunst-stack-tag:battery",
                        "-a", "Battery",
                        "Battery",
                        capacity .. "% (" .. status .. ")"
                    }
                })
            end
        elseif event.name == "network" then
            local ssid, _ = mplug.exec("nmcli -t -f active,ssid dev wifi 2>/dev/null | grep '^yes' | cut -d':' -f2")
            if ssid ~= "" then
                mplug.spawn("dunstify", {
                    args = {
                        "-t", "2000",
                        "-h", "string:x-dunst-stack-tag:network",
                        "-a", "Network",
                        "Network",
                        "Connected: " .. ssid
                    }
                })
            else
                local _, ping_exit = mplug.exec("ping -c 1 -W 1 1.1.1.1 2>/dev/null")
                local msg = (ping_exit == 0) and "Connected (Ethernet/Unknown)" or "Disconnected"
                mplug.spawn("dunstify", {
                    args = {
                        "-t", "2000",
                        "-h", "string:x-dunst-stack-tag:network",
                        "-a", "Network",
                        "Network",
                        msg
                    }
                })
            end
        end
    end
end)
