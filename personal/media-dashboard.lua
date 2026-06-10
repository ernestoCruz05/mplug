local anim_timer = nil
local current_frame = 0
local ticks_remaining = 0
local last_title = ""
local last_artist = ""
local last_status = ""
local last_art_url = nil

local function show_notification(title, artist, status, frame, timeout)
    timeout = timeout or 4000
    local title_str = (title ~= "" and title) or "Unknown Title"
    local artist_str = (artist ~= "" and artist) or "Unknown Artist"
    local status_icon = (status == "Playing") and "▶" or "⏸"
    local icon_path = "/tmp/mplug_media_" .. frame .. ".png"

    mplug.spawn("dunstify", {
        args = {
            "-h", "string:x-dunst-stack-tag:media",
            "-a", "Playerctl",
            "-t", tostring(timeout),
            "-i", icon_path,
            status_icon .. " Now Playing",
            string.format("%s\n<span foreground='#AAAAAA'>%s</span>", title_str, artist_str)
        }
    })
end

local function start_animation_loop(title, artist, status)
    if anim_timer then
        anim_timer:cancel()
        anim_timer = nil
    end

    if status ~= "Playing" then
        show_notification(title, artist, status, 0)
        return
    end

    current_frame = 0
    show_notification(title, artist, status, current_frame)

    ticks_remaining = 16
    anim_timer = mplug.every(250, function()
        ticks_remaining = ticks_remaining - 1
        current_frame = (current_frame + 1) % 12
        if ticks_remaining <= 0 then
            anim_timer:cancel()
            anim_timer = nil
            show_notification(title, artist, status, current_frame, 300)
            return
        end
        show_notification(title, artist, status, current_frame)
    end)
end

local function show_media_notification(force)
    local status, _ = mplug.exec("playerctl --player=spotify status")
    if status == "" then
        mplug.spawn("dunstify", {
            args = {
                "-h", "string:x-dunst-stack-tag:media",
                "-t", "2000",
                "Media", "Spotify not running"
            }
        })
        return
    end

    status = status:gsub("%s+", "")

    local title, _ = mplug.exec("playerctl --player=spotify metadata title")
    local artist, _ = mplug.exec("playerctl --player=spotify metadata artist")
    local art_url, _ = mplug.exec("playerctl --player=spotify metadata mpris:artUrl")

    title = title:gsub("^%s*(.-)%s*$", "%1")
    artist = artist:gsub("^%s*(.-)%s*$", "%1")
    art_url = art_url:gsub("^%s*(.-)%s*$", "%1")

    if not force and title == last_title and artist == last_artist and status == last_status then
        return
    end

    last_title = title
    last_artist = artist
    last_status = status

    if art_url == last_art_url then
        start_animation_loop(title, artist, status)
        return
    end

    local assets_dir = mplug.plugin_dir .. "/personal/assets/volume-controller"
    mplug.spawn("python3", {
        args = {
            mplug.plugin_dir .. "/personal/media-renderer.py",
            art_url,
            "/tmp/mplug_media",
            assets_dir
        },
        on_exit = function(code)
            last_art_url = art_url
            start_animation_loop(title, artist, status)
        end
    })
end

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" then
        if event.name == "media_dashboard" then
            show_media_notification(true)
        elseif event.name == "media_playpause" then
            mplug.spawn("playerctl", { args = { "--player=spotify", "play-pause" } })
        elseif event.name == "media_next" then
            mplug.spawn("playerctl", { args = { "--player=spotify", "next" } })
        elseif event.name == "media_prev" then
            mplug.spawn("playerctl", { args = { "--player=spotify", "previous" } })
        end
    end
end)

mplug.spawn("stdbuf", {
    args = { "-oL", "playerctl", "--player=spotify", "metadata", "--format", "{{title}} - {{artist}} - {{status}}", "--follow" },
    on_stdout = function(line)
        show_media_notification()
    end
})
