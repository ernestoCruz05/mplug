local anim_timer = nil
local current_frame = 1
local last_volume = 0
local last_muted = false
local ticks_remaining = 0

local function show_notification(volume, muted, frame, timeout)
    timeout = timeout or 1500
    local state_name
    if muted or volume == 0 then
        state_name = "mute"
    elseif volume <= 30 then
        state_name = "low"
    elseif volume <= 70 then
        state_name = "med"
    else
        state_name = "high"
    end

    local assets_dir = mplug.plugin_dir .. "/personal/assets/volume-controller/"
    local icon = string.format("%scat_%s_%d.png", assets_dir, state_name, frame)

    if muted then
        mplug.spawn("dunstify", {
            args = {
                "-t", tostring(timeout),
                "-h", "string:x-dunst-stack-tag:volume",
                "-i", icon,
                "-a", "Volume",
                "Volume", "Muted"
            }
        })
    else
        mplug.spawn("dunstify", {
            args = {
                "-t", tostring(timeout),
                "-h", "string:x-dunst-stack-tag:volume",
                "-h", "int:value:" .. volume,
                "-i", icon,
                "-a", "Volume",
                "Volume", volume .. "%"
            }
        })
    end
end

local function start_animation_loop()
    if anim_timer then
        return
    end

    anim_timer = mplug.every(250, function()
        ticks_remaining = ticks_remaining - 1
        current_frame = current_frame == 1 and 2 or 1
        if ticks_remaining <= 0 then
            anim_timer:cancel()
            anim_timer = nil
            show_notification(last_volume, last_muted, current_frame, 300)
            return
        end

        show_notification(last_volume, last_muted, current_frame)
    end)
end

local function update_volume_notification()
    local status, _ = mplug.exec("wpctl get-volume @DEFAULT_AUDIO_SINK@")
    if status == "" then
        return
    end

    local volume_raw = status:match("Volume:%s+([%d%.]+)")
    local muted = status:find("%[MUTED%]") ~= nil
    
    last_volume = math.floor((tonumber(volume_raw) or 0) * 100 + 0.5)
    last_muted = muted
    
    current_frame = 1
    show_notification(last_volume, last_muted, current_frame)
    
    ticks_remaining = 14
    start_animation_loop()
end

mplug.add_listener(function(event, state)
    if event.type == "UserCommand" then
        if event.name == "volume_up" then
            mplug.exec("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+")
            update_volume_notification()
        elseif event.name == "volume_down" then
            mplug.exec("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-")
            update_volume_notification()
        elseif event.name == "volume_mute" then
            mplug.exec("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle")
            update_volume_notification()
        end
    end
end)
