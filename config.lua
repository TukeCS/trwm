-- trwm default config
--
-- Copy it to ~/.config/trwm/config.lua and change whatever you like:
--   mkdir -p ~/.config/trwm && trwm default-config > ~/.config/trwm/config.lua
--
-- The config reloads automatically when you save it. If it has an error,
-- trwm keeps the old config and shows the error in the bar.
--
-- Key bindings are plain Lua functions. `a.<name>(args)` is shorthand for
-- `function() trwm.<name>(args) end`, so you can also write your own:
--   { "Mod+x", function() trwm.view(2); trwm.spawn("firefox") end },

local a = trwm.action

local term     = "kitty"
local launcher = { "rofi", "-show", "drun" }

-- keys ------------------------------------------------------------------------

local keys = {
  { "Mod+p",            a.spawn(launcher) },
  { "Mod+Return",       a.spawn(term) },
  { "Mod+Shift+l",      a.lock() },

  -- windows
  { "Mod+j",            a.focusstack(1) },
  { "Mod+k",            a.focusstack(-1) },
  { "Mod+i",            a.incnmaster(1) },
  { "Mod+d",            a.incnmaster(-1) },
  { "Mod+h",            a.setmfact(-0.05) },
  { "Mod+l",            a.setmfact(0.05) },
  { "Mod+Shift+Return", a.zoom() },
  { "Mod+Shift+c",      a.killclient() },
  { "Mod+Shift+space",  a.togglefloating() },
  { "Mod+Shift+f",      a.togglefullscreen() },
  { "Mod+b",            a.togglebar() },

  -- layouts
  { "Mod+t",            a.setlayout("tile") },
  { "Mod+f",            a.setlayout("float") },
  { "Mod+m",            a.setlayout("monocle") },
  { "Mod+space",        a.setlayout() },           -- back to the previous layout

  -- gaps: Mod+= / Mod+- bigger/smaller, Mod+g on/off, Mod+Shift+g default size
  { "Mod+equal",        a.incgaps(1) },
  { "Mod+minus",        a.incgaps(-1) },
  { "Mod+g",            a.togglegaps() },
  { "Mod+Shift+g",      a.defaultgaps() },

  -- tags and monitors
  { "Mod+Tab",          a.view() },                -- previous tags
  { "Mod+0",            a.view(0) },               -- 0 = all tags
  { "Mod+Shift+0",      a.tag(0) },
  { "Mod+comma",        a.focusmon(-1) },
  { "Mod+period",       a.focusmon(1) },
  { "Mod+Shift+comma",  a.tagmon(-1) },
  { "Mod+Shift+period", a.tagmon(1) },

  -- trwm itself
  { "Mod+Ctrl+r",       a.reload() },              -- reload config (also happens on save)
  { "Mod+Shift+r",      a.restart() },             -- restart in place, keeps all windows
  { "Mod+Shift+q",      a.quit() },
}

-- Mod+N view, Mod+Ctrl+N toggleview, Mod+Shift+N tag, Mod+Ctrl+Shift+N toggletag
for i = 1, 9 do
  table.insert(keys, { "Mod+" .. i,            a.view(i) })
  table.insert(keys, { "Mod+Ctrl+" .. i,       a.toggleview(i) })
  table.insert(keys, { "Mod+Shift+" .. i,      a.tag(i) })
  table.insert(keys, { "Mod+Ctrl+Shift+" .. i, a.toggletag(i) })
end

-- the config ------------------------------------------------------------------

return {
  modkey   = "Super",                 -- what "Mod" means: "Super" or "Alt"
  fonts    = { "monospace:size=10" },
  -- dpi   = 144,                     -- screen DPI for HiDPI screens (like `xrandr --dpi`
                                      -- + Xft.dpi). Unset = leave it alone (usually 96)

  borderpx = 1,
  snap     = 32,
  gaps     = { inner = 10, outer = 10, smart = true },  -- smart: no outer gap with one window
  showbar  = true,
  topbar   = true,

  -- dwm's default colours
  colors = {
    norm = { fg = "#bbbbbb", bg = "#222222", border = "#444444" },
    sel  = { fg = "#eeeeee", bg = "#005577", border = "#005577" },
  },

  tags = { "1", "2", "3", "4", "5", "6", "7", "8", "9" },

  -- class / instance / title are substring matches (run `xprop` to find them)
  -- tag = N puts the window on tag N, floating = true makes it float
  rules = {
    { class = "Gimp", floating = true },
  },

  mfact          = 0.55,
  nmaster        = 1,
  resizehints    = false,
  lockfullscreen = true,

  -- first entry is the default; also available: bstack grid deck
  layouts = {
    { "[]=", "tile" },
    { "><>", "float" },
    { "[M]", "monocle" },
  },

  keys = keys,

  -- where: tagbar layout status title client root
  -- tag-bar actions without arguments receive the clicked tag number
  buttons = {
    { "layout", "Button1",     a.setlayout() },
    { "layout", "Button3",     a.setlayout("monocle") },
    { "title",  "Button2",     a.zoom() },
    { "status", "Button2",     a.spawn(term) },
    { "client", "Mod+Button1", a.movemouse() },
    { "client", "Mod+Button2", a.togglefloating() },
    { "client", "Mod+Button3", a.resizemouse() },
    { "tagbar", "Button1",     a.view() },
    { "tagbar", "Button3",     a.toggleview() },
    { "tagbar", "Mod+Button1", a.tag() },
    { "tagbar", "Mod+Button3", a.toggletag() },
  },

  -- status bar. Each entry: { source, format, argument }
  --   %s is replaced by the value, %% is a literal %
  --   more sources: battery_perc, temp, disk_perc, netspeed_rx, uptime, ...
  --   (see the README), "run_command" for any shell command, or a Lua function
  status = {
    { "cpu_perc", "cpu %s%% | " },
    { "ram_perc", "ram %s%% | " },
    { "datetime", "%s", "%a %d %b %H:%M" },
  },

  -- screen locker (Mod+Shift+l or `trwm lock`): slock's colours
  lock = {
    colors = { init = "black", input = "#005577", failed = "#CC3333" },
    fail_on_clear = true,
    clock         = false,     -- true shows the time and date
    monitor_off   = 0,         -- turn the screen off after N seconds (0 = never)
    idle          = 0,         -- lock after N idle seconds (0 = off)
  },

  -- wallpaper, set with feh (off by default). A file, or a folder to pick a
  -- random image from; mode = fill | center | max | scale | tile
  -- wallpaper = { path = "~/Pictures/wall.png", mode = "fill" },

  -- commands to run once when trwm starts
  autostart = {},
}
