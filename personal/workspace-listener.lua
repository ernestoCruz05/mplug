local last_active_tags = {}

local layout_map = {
    T = "Tile",
    S = "Scroller",
    G = "Grid",
    M = "Monocle",
    K = "Deck",
    CT = "Center Tile",
    RT = "Right Tile",
    VS = "Vertical Scroller",
    VT = "Vertical Tile",
    VG = "Vertical Grid",
    VK = "Vertical Deck",
    CV = "Canvas",
    DW = "Dwindle",
    F = "Fair",
    VF = "Vertical Fair"
}

mplug.add_listener(function(event, state)
    if event.type == "IpcMonitors" then
        for _, mon in ipairs(event.data.monitors) do
            local mon_name = mon.name
            local current_tag = mon.active_tags[1]
            
            if current_tag and current_tag ~= last_active_tags[mon_name] then
                if last_active_tags[mon_name] ~= nil then
                    local layout_symbol = nil
                    if state.ipc_tags and state.ipc_tags.all_tags then
                        for _, mon_tags in ipairs(state.ipc_tags.all_tags) do
                            if mon_tags.monitor == mon_name then
                                for _, tag_info in ipairs(mon_tags.tags) do
                                    if tag_info.index == current_tag then
                                        layout_symbol = tag_info.layout
                                        break
                                    end
                                end
                            end
                        end
                    end

                    local layout_name = layout_symbol and layout_map[layout_symbol] or layout_symbol
                    local message
                    if layout_name then
                        message = string.format("[ Tag %d on %s ]\nLayout: %s", current_tag, mon_name, layout_name)
                    else
                        message = string.format("[ Tag %d on %s ]", current_tag, mon_name)
                    end

                    mplug.spawn("dunstify", {
                        args = {
                            "-h", "string:x-dunst-stack-tag:workspace-" .. mon_name,
                            "-t", "1000",
                            "-a", "MangoWM",
                            message
                        }
                    })
                end
                last_active_tags[mon_name] = current_tag
            end
        end
    end
end)
