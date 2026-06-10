-- screenshot.lua
--
-- Triggers a screenshot selection via slurp, captures it via grim, saves it
-- to Pictures/Screenshots, copies it to the clipboard, and plays the
-- photographer cat in a Dunst popup: frames 1-3 loop while the region is
-- being selected, frame 4 is the shutter payoff once the capture lands.
--
-- Trigger: screenshot
-- Frames:  assets/screenshot/base1..base4.png (80x80); the dunstrc
--          [screenshot] rule pins max_icon_size = 80 for them.

local FRAME_MS = 600 -- cycle speed of the waiting cat (frames 1-3)
local DONE_MS = 2500 -- how long the final shutter frame stays up

local anim_timer = nil
local capturing = false

local function show_frame(frame, title, body, timeout)
    local icon = mplug.plugin_dir .. "/personal/assets/screenshot/base" .. frame .. ".png"
    mplug.spawn("dunstify", {
        args = {
            "-t", tostring(timeout),
            "-h", "string:x-dunst-stack-tag:screenshot",
            "-a", "Screenshot",
            "-i", icon,
            title,
            body
        }
    })
end

local function stop_animation()
    if anim_timer then
        anim_timer:cancel()
        anim_timer = nil
    end
end

local function start_selection_animation()
    stop_animation()
    local frame = 1
    show_frame(frame, "Screenshot", "Select a region…", FRAME_MS + 500)
    anim_timer = mplug.every(FRAME_MS, function()
        frame = frame % 3 + 1
        show_frame(frame, "Screenshot", "Select a region…", FRAME_MS + 500)
    end)
end

local capture_script = [[
DIR="$HOME/Pictures/Screenshots"
FILE="$DIR/Screenshot_$(date +'%Y-%m-%d_%H-%M-%S').png"
mkdir -p "$DIR"
GEOM=$(slurp) || exit 1
grim -g "$GEOM" - | tee "$FILE" | wl-copy
echo "$FILE"
]]

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" and event.name == "screenshot" then
        if capturing then
            return
        end
        capturing = true
        local saved_file = ""

        start_selection_animation()

        -- Spawned, not exec'd: slurp blocks until a region is picked, and
        -- that must not freeze the engine (or the cat above).
        mplug.spawn("sh", {
            args = { "-c", capture_script },
            on_stdout = function(line)
                saved_file = line
            end,
            on_exit = function(code)
                capturing = false
                stop_animation()
                if code == 0 then
                    local name = saved_file:match("([^/]+)$") or ""
                    local body = "Saved to Pictures/Screenshots\nand copied to clipboard."
                    if name ~= "" then
                        body = name .. "\nSaved and copied to clipboard."
                    end
                    show_frame(4, "Screenshot Captured", body, DONE_MS)
                else
                    -- Selection cancelled: replace the waiting popup with a
                    -- blink-and-it's-gone frame so it clears immediately.
                    show_frame(1, "Screenshot", "Cancelled", 400)
                end
            end
        })
    end
end)
