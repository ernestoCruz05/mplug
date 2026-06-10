-- notifications.lua
--
-- A plugin that listens to MangoWM events and sends custom desktop notifications
-- using Dunst (via dunstify). It uses Dunst's x-dunst-stack-tag feature to
-- ensure new notifications replace old ones of the same type without cluttering the screen.

-- Track the previous state to avoid duplicate notifications on startup/re-entry
local last_keymode = nil
local last_layout = nil
local last_wm_layout = nil

mplug.add_listener(function(event, state)
    -- 1. Notify on Keyboard Mode changes (Vim modes in MangoWM)
    if event.type == "IpcKeyMode" then
        if event.keymode ~= last_keymode then
            last_keymode = event.keymode
            
            -- Format the keymode name nicely (e.g. capitalize normal/insert)
            local mode_name = event.keymode:gsub("^%l", string.upper)
            
            mplug.spawn("dunstify", {
                args = {
                    "-h", "string:x-dunst-stack-tag:mango-keymode",
                    "-a", "MangoWM",
                    "-i", "keyboard",
                    "Keyboard Mode Changed",
                    "Vim Mode: " .. mode_name
                }
            })
        end
    end

    -- 2. Notify on XKB Keyboard Layout changes
    if event.type == "IpcKeyboardLayout" then
        if event.layout ~= last_layout then
            last_layout = event.layout
            
            mplug.spawn("dunstify", {
                args = {
                    "-h", "string:x-dunst-stack-tag:mango-layout",
                    "-a", "MangoWM",
                    "-i", "input-keyboard",
                    "Keyboard Layout Changed",
                    "Active Layout: " .. event.layout:upper()
                }
            })
        end
    end

    -- 3. Notify when the Window Manager Layout changes
    if event.type == "LayoutName" then
        if event.name ~= last_wm_layout then
            last_wm_layout = event.name
            
            mplug.spawn("dunstify", {
                args = {
                    "-h", "string:x-dunst-stack-tag:mango-wmlayout",
                    "-a", "MangoWM",
                    "-i", "window-new",
                    "Compositor Layout Changed",
                    "Active Layout: " .. event.name
                }
            })
        end
    end
end)
