hl.monitor({ output = "", mode = "1280x800@60", position = "0x0", scale = 1 })
hl.config({ general = { gaps_in = 8, gaps_out = 8, border_size = 2 }, animations = { enabled = false }, misc = { disable_hyprland_logo = true, disable_splash_rendering = true } })
hl.window_rule({ name = "test-float", match = { class = ".*" }, float = true, size = { 1000, 650 }, center = true })
if hl.plugin and hl.plugin.hyprbars then hl.config({ plugin = { hyprbars = { enabled = true, bar_height = 32, bar_text_align = "left", bar_text_size = 15 } } }) end
