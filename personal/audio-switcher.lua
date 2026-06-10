local function get_audio_sinks()
    local stdout, _ = mplug.exec("wpctl status")
    if stdout == "" then return {} end
    
    local sinks = {}
    local in_sinks_section = false
    
    for line in stdout:gmatch("[^\r\n]+") do
        if line:find("Sinks:") then
            in_sinks_section = true
        elseif in_sinks_section then
            if line:find("Sources:") or line:find("Filters:") or line:find("Streams:") or line:find("^[A-Z]") then
                in_sinks_section = false
            else
                local is_active = line:find("%*") ~= nil
                local id, name = line:match("(%d+)%.%s*(.-)%s*%[")
                if not id then
                    id, name = line:match("(%d+)%.%s*(.*)")
                end
                if id then
                    id = tonumber(id)
                    name = name:gsub("%s+$", "")
                    table.insert(sinks, {
                        id = id,
                        name = name,
                        is_active = is_active
                    })
                end
            end
        end
    end
    return sinks
end

local function show_audio_switcher()
    local sinks = get_audio_sinks()
    if #sinks == 0 then
        mplug.spawn("dunstify", {
            args = {
                "-h", "string:x-dunst-stack-tag:audio-switcher",
                "-t", "3000",
                "-a", "AudioSwitcher",
                "Audio Switcher", "No audio output devices found."
            }
        })
        return
    end
    
    local args = {
        "-h", "string:x-dunst-stack-tag:audio-switcher",
        "-t", "8000",
        "-a", "AudioSwitcher",
    }
    
    for _, sink in ipairs(sinks) do
        local prefix = sink.is_active and "● " or "○ "
        table.insert(args, "-A")
        table.insert(args, string.format("%d,%s%s", sink.id, prefix, sink.name))
    end
    
    table.insert(args, "Audio Output Switcher")
    table.insert(args, "Select default audio sink:")
    
    mplug.spawn("dunstify", {
        args = args,
        on_stdout = function(id, line)
            local sink_id = tonumber(line)
            if sink_id then
                mplug.exec("wpctl set-default " .. sink_id)
                mplug.after(100, show_audio_switcher)
            end
        end
    })
end

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" and event.name == "audio_switcher" then
        show_audio_switcher()
    end
end)
