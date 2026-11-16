-- tuke's trwm config
--
-- A full personal setup to get ideas from: Gentoo purple theme, Nerd Font
-- icons for tags and status, vanity gaps, HiDPI (144 dpi) and cursor size,
-- Hyprland-style spiral/dwindle layouts (Mod+r flips between them, Mod+a
-- re-tiles, dragging swaps windows), keyboard / mouse / touchpad / monitor /
-- screen blanking settings, window rules, media/brightness keys,
-- screenshots, a feh wallpaper and autostart.
--
-- To try it:   cp examples/tuke.lua ~/.config/trwm/config.lua
--
-- It expects: kitty, rofi, yazi, neovim, firefox, maim + xclip, wpctl
-- (PipeWire), brightnessctl, dunst, udiskie, lxqt-policykit-agent, picom,
-- feh and "Symbols Nerd Font Mono". The supergfxctl and platform_profile
-- status entries are for ASUS laptops; remove them on other machines, and
-- change the wallpaper path to one of your own images. The monitor name
-- ("eDP") and the mouse/touchpad values are for this laptop: run
-- `trwm monitors` and `trwm input` to see yours.

local a = trwm.action

-- programs ------------------------------------------------------------------

local term     = "kitty"
local launcher = { "rofi", "-show", "drun" }
local files    = { "kitty", "--class", "yazi", "-e", "yazi" }
local code     = { "kitty", "--class", "nvim", "-e", "nvim" }
local browser  = "firefox"

local shotdir  = "mkdir -p ~/Pictures/Screenshots && f=~/Pictures/Screenshots/$(date +%F_%H-%M-%S).png && "
local toclip   = ' "$f" && xclip -selection clipboard -t image/png -i "$f"'

-- `trwm msg refresh` makes the status bar update right away
local volup    = "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+; trwm msg refresh"
local voldown  = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-; trwm msg refresh"
local volmute  = "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle; trwm msg refresh"
local micmute  = "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"
local brightup = "brightnessctl set 5%+; trwm msg refresh"
local brightdn = "brightnessctl -n set 5%-; trwm msg refresh"

-- Gentoo purple palette (matches Neovim + kitty) ------------------------------

local col = {
  bg        = "#17141f", -- bar background
  border    = "#2e2740", -- unfocused window border
  fg        = "#c4bfdb", -- bar text / status
  selfg     = "#dddfff", -- selected tag/title text
  selbg     = "#54487a", -- selected tag/title bg
  selborder = "#6e56af", -- focused window border
  red       = "#a33a4a",
}

-- keys ------------------------------------------------------------------------

local keys = {
  -- apps
  { "Mod+p",            a.spawn(launcher) },
  { "Mod+Return",       a.spawn(term) },
  { "Mod+e",            a.spawn(files) },
  { "Mod+n",            a.spawn(code) },
  { "Mod+w",            a.spawn(browser) },
  { "Mod+Shift+l",      a.lock() },               -- built-in locker (slock)

  -- gaps: Super+g on/off, Super+= / Super+- bigger/smaller, Super+Shift+g default
  { "Mod+g",            a.togglegaps() },
  { "Mod+Shift+g",      a.defaultgaps() },
  { "Mod+equal",        a.incgaps(1) },
  { "Mod+minus",        a.incgaps(-1) },

  -- screenshots: Print = full screen, Super+Print = select area
  { "Print",            a.spawn(shotdir .. "maim" .. toclip) },
  { "Mod+Print",        a.spawn(shotdir .. "maim -s -b 2 -c 0.431,0.337,0.686,1" .. toclip) },

  -- media keys
  { "XF86AudioRaiseVolume",  a.spawn(volup) },
  { "XF86AudioLowerVolume",  a.spawn(voldown) },
  { "XF86AudioMute",         a.spawn(volmute) },
  { "XF86AudioMicMute",      a.spawn(micmute) },
  { "XF86MonBrightnessUp",   a.spawn(brightup) },
  { "XF86MonBrightnessDown", a.spawn(brightdn) },

  -- windows and layouts
  { "Mod+b",            a.togglebar() },
  { "Mod+j",            a.focusstack(1) },
  { "Mod+k",            a.focusstack(-1) },
  { "Mod+i",            a.incnmaster(1) },
  { "Mod+d",            a.incnmaster(-1) },
  { "Mod+h",            a.setmfact(-0.05) },
  { "Mod+l",            a.setmfact(0.05) },
  { "Mod+Shift+Return", a.zoom() },
  { "Mod+Tab",          a.view() },                -- previous tags
  { "Mod+Shift+c",      a.killclient() },
  { "Mod+t",            a.setlayout("tile") },
  { "Mod+f",            a.setlayout("float") },
  { "Mod+m",            a.setlayout("monocle") },
  { "Mod+r",            a.togglelayout("spiral", "dwindle") },  -- switch spiral <-> dwindle
  { "Mod+space",        a.setlayout() },           -- back to the previous layout
  { "Mod+a",            a.retile() },              -- arrange all: put every floated window back into the tiling
  { "Mod+Shift+space",  a.togglefloating() },
  { "Mod+Shift+f",      a.togglefullscreen() },

  -- tags and monitors
  { "Mod+0",            a.view(0) },               -- 0 = all tags
  { "Mod+Shift+0",      a.tag(0) },
  { "Mod+comma",        a.focusmon(-1) },
  { "Mod+period",       a.focusmon(1) },
  { "Mod+Shift+comma",  a.tagmon(-1) },
  { "Mod+Shift+period", a.tagmon(1) },

  -- trwm itself
  { "Mod+Shift+r",      a.restart() },             -- re-exec (keeps windows) after rebuilding
  { "Mod+Ctrl+r",       a.reload() },              -- reload config (also happens on save)
  { "Mod+Shift+q",      a.quit() },
}

-- Mod+N view, Mod+Ctrl+N toggleview, Mod+Shift+N tag, Mod+Ctrl+Shift+N toggletag
for i = 1, 9 do
  table.insert(keys, { "Mod+" .. i,            a.view(i) })
  table.insert(keys, { "Mod+Ctrl+" .. i,       a.toggleview(i) })
  table.insert(keys, { "Mod+Shift+" .. i,      a.tag(i) })
  table.insert(keys, { "Mod+Ctrl+Shift+" .. i, a.toggletag(i) })
end

-- the config -------------------------------------------------------------------

---@type trwm.Config
return {
  modkey   = "Super",                 -- what "Mod" means in keys/buttons
  fonts    = { "monospace:size=12", "Symbols Nerd Font Mono:size=13" },
  dpi      = 144,                 -- screen DPI (replaces `xrandr --dpi 144` in .xinitrc)
  cursor_size = 36,               -- mouse cursor size (replaces Xcursor.size in ~/.Xresources)
  -- cursor_theme = "Adwaita",
  dark_mode   = true,             -- dark GTK/Qt apps, Firefox, Chromium, ...

  -- hardware: what setxkbmap, xset, xinput and xrandr would do. Every field
  -- is optional; `trwm keyboard`, `trwm input` and `trwm monitors` show the
  -- current settings and the names to use here.
  keyboard = {
    layout       = "us",                -- or { "us", "fr" } for several
    -- variant   = { "", "azerty" },    -- one per layout
    -- options   = { "caps:escape", "grp:alt_shift_toggle" },
    repeat_delay = 660,                 -- ms before a held key repeats
    repeat_rate  = 25,                  -- repeats per second
    numlock      = false,
  },
  mouse = {                             -- the Logitech PRO X 2
    speed  = 0.0,                       -- -1.0 .. 1.0
    accel  = "flat",                    -- "flat" = no acceleration
    natural_scroll = false,
  },
  touchpad = {
    tap                  = false,       -- tap to click
    natural_scroll       = false,
    disable_while_typing = true,
  },
  monitors = {
    { output = { "eDP", "eDP-1" }, mode = "2560x1600", rate = 240, primary = true },
    -- { output = { "HDMI-A-0", "HDMI-A-1-0" }, mode = "1920x1080", rate = 144, right_of = "eDP" },
  },
  screen = { blank = false, off = false },  -- never blank (was `xset s off -dpms`)
  -- cursor_theme = "Adwaita",

  borderpx = 3,
  snap     = 32,
  gaps     = { inner = 10, outer = 10, smart = true },  -- or inner_h/inner_v/outer_h/outer_v
  showbar  = true,
  topbar   = true,
  hide_empty_tags = false,

  colors = {
    norm = { fg = col.fg,    bg = col.bg,    border = col.border },
    sel  = { fg = col.selfg, bg = col.selbg, border = col.selborder },
  },

  -- wallpaper (set with feh). A file, or a directory to pick a random image
  -- from. mode = fill | center | max | scale | tile.
  -- interval > 0 picks a new random one every N seconds (directories only).
  -- You can also set it at runtime:  trwm msg 'trwm.wallpaper("~/Pictures/x.png")'
  wallpaper = { path = "~/Pictures/Wallpapers/bat.png", mode = "fill" },
  -- wallpaper = { path = "~/Pictures/Wallpapers", mode = "fill", interval = 1800 },  -- random, rotates

  -- Nerd Font icons: home, terminal, web, code, files, chat, video, VMs, games
  tags = { "\u{f015}", "\u{f120}", "\u{f269}", "\u{f121}", "\u{f07b}",
           "\u{f086}", "\u{f03d}", "\u{f1b2}", "\u{f11b}" },

  -- class / instance / title are substring matches (like dwm)
  -- tag = N (1-based) or tags = { N, M }; floating; monitor
  rules = {
    -- float on whatever tag is open
    { class = "rog-control-center", floating = true },
    { class = "Gimp",               floating = true },

    { class = "firefox",  tag = 3 },
    { class = "nvim",     tag = 4 },        -- Neovim started with Super+n
    { class = "yazi",     tag = 5 },        -- yazi started with Super+e
    { class = "discord",  tag = 6 },
    { class = "obs",      tag = 7 },
    { instance = "kdenlive", tag = 7 },
    { instance = "qemu",     tag = 8 },

    -- games: Steam, CS2 and every Proton game; Steam's small dialogs float
    { instance = "steamwebhelper", tag = 9 },
    { instance = "steamwebhelper", title = "Friends List",   tag = 9, floating = true },
    { instance = "steamwebhelper", title = "Steam Settings", tag = 9, floating = true },
    { instance = "steamwebhelper", title = "Properties",     tag = 9, floating = true },
    { class = "cs2",        tag = 9 },
    { class = "steam_app_", tag = 9 },
  },

  mfact          = 0.55,
  nmaster        = 1,
  resizehints    = false,
  lockfullscreen = true,
  refresh_rate   = 240,   -- Hz, for moving/resizing with the mouse
  drag_tiled     = "swap",  -- Mod+drag a tiled window: swap it where you drop it ("float" = dwm)
  focus_follows_mouse = true,

  -- first entry is the default; available: spiral dwindle tile float monocle
  -- (bstack grid deck also exist if you ever want them)
  default_layout = "spiral",   -- every tag starts in spiral ("tile" = dwm)

  layouts = {
    { "S", "spiral" },       -- like dwindle, but new windows spiral inwards (default)
    { "D", "dwindle" },      -- like Hyprland: new windows split the one under the cursor
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

  -- built-in status bar (slstatus). Each entry: { source, format, argument }
  --   source: a builtin name, "run_command", or a Lua function returning a string
  --   format: %s is replaced by the value, %% is a literal %
  --   interval = N (seconds) per entry to update less often
  -- builtins: datetime cpu_perc cpu_freq ram_perc ram_used ram_free ram_total
  --   swap_perc swap_used battery_perc battery_state battery_remaining temp
  --   uptime load_avg disk_perc disk_free disk_used disk_total netspeed_rx
  --   netspeed_tx ipv4 wifi_perc kernel_release hostname username num_files entropy
  -- (they are also callable from Lua: trwm.sys.cpu_perc())
  -- set `status = nil` to use `xsetroot -name` / external slstatus instead.
  status = {
    interval = 1,
    { "cpu_perc", "\u{f4bc} %s%%  " },
    { "ram_perc", "\u{f035b} %s%%  " },
    { "run_command", "%s  ",
      "wpctl get-volume @DEFAULT_AUDIO_SINK@ | awk '/MUTED/{print \"\u{f0581} muted\";exit} {printf \"\u{f057e} %d%%\", $2*100+0.5}'" },
    { "run_command", "\u{f00df} %s  ", "brightnessctl -m | cut -d, -f4" },
    { "run_command", "\u{f0379} %s  ", "supergfxctl -g | sed 's/AsusMuxDgpu/dGPU/;s/Integrated/iGPU/'", interval = 10 },
    { "run_command", "\u{f0e7} %s  ", "cat /sys/firmware/acpi/platform_profile", interval = 5 },
    { "battery_state", "\u{f0079} %s", "BAT0" },
    { "battery_perc",  "%s%%  ", "BAT0" },
    { "datetime", "\u{f017} %s ", "%T" },
  },

  -- built-in screen locker (slock): Super+Shift+l, `trwm lock`,
  -- or `xss-lock -- trwm lock` in your .xinitrc
  lock = {
    colors = {
      init   = col.bg,          -- locked
      input  = col.selborder,   -- typing
      failed = col.red,         -- wrong password
      text   = col.selfg,       -- clock text
    },
    fail_on_clear = true,       -- clearing the input shows the failed colour
    monitor_off   = 5,          -- seconds until the screen turns off (0 = never)
    clock         = true,       -- show time/date (false = plain slock look)
    font          = "monospace:size=64",
    small_font    = "monospace:size=16",
    -- message    = "be right back",
    idle          = 0,          -- lock after N idle seconds (0 = off)
    pam_service   = "trwm",   -- uses /etc/pam.d/trwm if present, else "login"
    lockout       = false,    -- or { attempts = 3, minutes = 10 }
  },

  -- run once when trwm starts (not on reload or restart)
  autostart = {
    "dunst",                  -- notifications
    "udiskie",                -- automount USB/exFAT drives
    "lxqt-policykit-agent",   -- password prompts for GUI apps
    "picom -b",               -- compositor
  },
}
