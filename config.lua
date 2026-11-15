-- ============================================================================
--  trwm config
-- ============================================================================
--
--  This file is also the manual: every option trwm has is in here, explained.
--  Lines starting with `--` are comments. To turn an option on, remove the
--  `--` in front of it.
--
--  Your config lives in ~/.config/trwm/config.lua (trwm writes this file there
--  the first time it starts). Save it and the changes apply right away. If you
--  make a mistake, trwm keeps the last working config and shows the error in
--  the bar.
--
--  Useful commands (run them in a terminal):
--    trwm doctor      check your setup and tell you how to fix problems
--    trwm check       check this file for mistakes
--    trwm keyboard    show your keyboard layout
--    trwm input       show your mice/touchpads and their settings
--    trwm monitors    show your monitors, their names and resolutions
--    trwm msg reload  reload this file (also happens when you save it)
--
--  "Mod" below means the Super (Windows) key; see `modkey` to change it.
-- ============================================================================

local a = trwm.action
-- `a.spawn("kitty")` is short for `function() trwm.spawn("kitty") end`.
-- Every key binding is just a Lua function, so you can also write your own:
--   { "Mod+x", function() trwm.view(2); trwm.spawn("firefox") end },

-- Programs ---------------------------------------------------------------------
-- A string runs through the shell (so ~, && and pipes work), a list runs the
-- program directly with those arguments.

local term     = "kitty"                        -- e.g. "alacritty", "st", "wezterm"
local launcher = { "rofi", "-show", "drun" }    -- e.g. "dmenu_run"

-- Keys ---------------------------------------------------------------------------
-- { "Modifiers+Key", action }
--   Modifiers: Mod, Super, Alt, Shift, Ctrl
--   Key names: a, 1, Return, space, Tab, comma, period, equal, minus, Print,
--   F1, Left, XF86AudioRaiseVolume, ... Run `xev` and press a key to see its name.

local keys = {
  -- programs
  { "Mod+Return",       a.spawn(term) },
  { "Mod+p",            a.spawn(launcher) },
  { "Mod+Shift+l",      a.lock() },                -- lock the screen
  -- { "Mod+w",         a.spawn("firefox") },
  -- { "Print",         a.spawn("maim ~/screenshot.png") },
  -- { "XF86AudioRaiseVolume", a.spawn("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+; trwm msg refresh") },
  --   (`trwm msg refresh` updates the status bar right away)

  -- windows
  { "Mod+j",            a.focusstack(1) },         -- focus the next window
  { "Mod+k",            a.focusstack(-1) },        -- focus the previous window
  { "Mod+h",            a.setmfact(-0.05) },       -- shrink the master / split
  { "Mod+l",            a.setmfact(0.05) },        -- grow the master / split
  { "Mod+i",            a.incnmaster(1) },         -- more windows in the master area (tile)
  { "Mod+d",            a.incnmaster(-1) },        -- fewer windows in the master area
  { "Mod+Shift+Return", a.zoom() },                -- swap with the master / first window
  { "Mod+Shift+c",      a.killclient() },          -- close the window
  { "Mod+Shift+space",  a.togglefloating() },      -- float / tile the window
  { "Mod+Shift+f",      a.togglefullscreen() },
  { "Mod+b",            a.togglebar() },           -- hide / show the bar
  -- { "Mod+a",         a.retile() },              -- put every floated window back into the tiling

  -- layouts (see `layouts` below)
  { "Mod+r",            a.togglelayout("spiral", "dwindle") },
  { "Mod+t",            a.setlayout("tile") },
  { "Mod+f",            a.setlayout("float") },
  { "Mod+m",            a.setlayout("monocle") },
  { "Mod+space",        a.setlayout() },           -- back to the previous layout
  -- { "Mod+n",         a.cyclelayout(1) },        -- go through all layouts

  -- gaps between windows
  { "Mod+equal",        a.incgaps(1) },            -- bigger (use 5 for bigger steps)
  { "Mod+minus",        a.incgaps(-1) },           -- smaller
  { "Mod+g",            a.togglegaps() },          -- on / off
  { "Mod+Shift+g",      a.defaultgaps() },         -- back to the size set in `gaps`

  -- tags (workspaces) and monitors
  { "Mod+Tab",          a.view() },                -- back to the previous tag
  { "Mod+0",            a.view(0) },               -- view all tags
  { "Mod+Shift+0",      a.tag(0) },                -- put the window on all tags
  { "Mod+comma",        a.focusmon(-1) },          -- focus the previous monitor
  { "Mod+period",       a.focusmon(1) },           -- focus the next monitor
  { "Mod+Shift+comma",  a.tagmon(-1) },            -- send the window to the previous monitor
  { "Mod+Shift+period", a.tagmon(1) },             -- send the window to the next monitor

  -- keyboard layouts (see `keyboard` below)
  -- { "Mod+Ctrl+space", a.nextkeyboard() },       -- next keyboard layout
  -- { "Mod+F1",        a.setkeyboard("us") },     -- a specific one

  -- wallpaper (see `wallpaper` below)
  -- { "Mod+Shift+w",   a.wallpaper() },           -- pick a new random one from your folder

  -- trwm itself
  { "Mod+Ctrl+r",       a.reload() },              -- reload this file (also happens on save)
  { "Mod+Shift+r",      a.restart() },             -- restart trwm, keeping all windows
  { "Mod+Shift+q",      a.quit() },                -- quit trwm (log out)
}

-- Mod+1..9: view tag    Mod+Shift+1..9: move the window there
-- Mod+Ctrl+1..9: also view that tag    Mod+Ctrl+Shift+1..9: also put the window there
for i = 1, 9 do
  table.insert(keys, { "Mod+" .. i,            a.view(i) })
  table.insert(keys, { "Mod+Ctrl+" .. i,       a.toggleview(i) })
  table.insert(keys, { "Mod+Shift+" .. i,      a.tag(i) })
  table.insert(keys, { "Mod+Ctrl+Shift+" .. i, a.toggletag(i) })
end

-- ============================================================================

---@type trwm.Config
return {

  -- Basics -------------------------------------------------------------------

  modkey = "Super",                -- what "Mod" means: "Super" (Windows key) or "Alt"

  -- fonts for the bar; later fonts are used for characters the first one
  -- doesn't have (e.g. add "Symbols Nerd Font Mono:size=11" for icons)
  fonts = { "monospace:size=10" },

  -- Screen and cursor --------------------------------------------------------

  -- dpi          = 144,            -- for HiDPI screens: 96 = normal, 144 = 150%, 192 = 200%
  -- cursor_size  = 36,             -- mouse cursor size in pixels (24 is normal at 96 dpi)
  -- cursor_theme = "Adwaita",      -- a cursor theme from /usr/share/icons or ~/.icons

  -- when the screen blanks and the monitor switches off, in seconds (false = never)
  -- screen = { blank = 600, off = 900 },

  -- monitors: resolution, refresh rate and arrangement. Run `trwm monitors` to
  -- see the names of your outputs and their modes. Outputs you don't list stay
  -- as they are; a listed monitor turns on by itself when you plug it in.
  -- monitors = {
  --   { output = "eDP-1",  mode = "1920x1080", rate = 60, primary = true },
  --   { output = "HDMI-1", mode = "2560x1440", rate = 144, right_of = "eDP-1" },
  --   -- position: right_of / left_of / above / below / same_as = "name",
  --   --           or pos = "1920x0"
  --   -- rotate = "left" / "right" / "inverted",   off = true switches it off
  -- },

  -- Keyboard, mouse and touchpad -----------------------------------------------

  -- keyboard layout, key repeat and num lock. Leave out what you don't want to
  -- change. All layouts, variants and options are listed (with descriptions)
  -- in /usr/share/X11/xkb/rules/base.lst
  -- keyboard = "fr",                       -- just a layout, or:
  -- keyboard = {
  --   layout       = { "us", "fr" },       -- one or more layouts
  --   variant      = { "", "azerty" },     -- one per layout ("" = the normal one)
  --   options      = { "grp:alt_shift_toggle", "caps:escape" },
  --                  -- grp:alt_shift_toggle: Alt+Shift switches layout
  --                  -- caps:escape: Caps Lock is Escape, caps:ctrl_modifier: Ctrl
  --                  -- compose:ralt: right Alt types accents (é = Compose ' e)
  --   repeat_delay = 300,                  -- ms before a held key starts repeating
  --   repeat_rate  = 40,                   -- repeats per second
  --   numlock      = true,                 -- num lock on when trwm starts
  -- },

  -- mouse settings (every mouse, also ones plugged in later). `trwm input`
  -- shows your devices and their current settings.
  -- mouse = {
  --   speed            = 0.0,        -- -1.0 (slowest) to 1.0 (fastest)
  --   accel            = "flat",     -- "flat" = no acceleration (games), "adaptive" = normal
  --   natural_scroll   = false,      -- true = scroll like on a phone
  --   left_handed      = false,      -- swap the left and right buttons
  --   middle_emulation = false,      -- left + right together = middle click
  -- },

  -- touchpad settings (the same options as mouse, plus these)
  -- touchpad = {
  --   tap                  = true,   -- tap to click
  --   natural_scroll       = true,
  --   disable_while_typing = true,
  --   speed                = 0.3,
  -- },

  -- Look ------------------------------------------------------------------------

  borderpx = 1,                    -- window border width in pixels

  -- gaps between windows, in pixels. Change them live with Mod+= / Mod+-
  gaps = {
    inner = 10,                    -- between windows
    outer = 10,                    -- between windows and the screen edge
    smart = true,                  -- no outer gap when only one window is open
    -- enabled = false,            -- start with gaps off (Mod+g turns them on)
    -- inner_h = 10, inner_v = 10, outer_h = 10, outer_v = 10,  -- separate sizes
  },

  -- colours: norm = normal, sel = selected (the focused window, the current tag)
  colors = {
    norm = { fg = "#bbbbbb", bg = "#222222", border = "#444444" },
    sel  = { fg = "#eeeeee", bg = "#005577", border = "#005577" },
  },

  showbar = true,                  -- show the bar (Mod+b toggles it)
  topbar  = true,                  -- false = bar at the bottom
  -- hide_empty_tags = true,       -- only show tags that have windows

  -- wallpaper, set with feh: an image, or a folder to pick a random one from.
  -- mode: fill | center | max | scale | tile. interval: seconds until the next
  -- random one (folders only). Change it live: trwm msg 'trwm.wallpaper("~/x.png")'
  -- wallpaper = { path = "~/Pictures/wallpaper.png", mode = "fill" },
  -- wallpaper = { path = "~/Pictures/Wallpapers", mode = "fill", interval = 1800 },

  -- Tags (workspaces) -------------------------------------------------------------

  -- names shown in the bar: any text, emoji, or icons from a Nerd Font
  tags = { "1", "2", "3", "4", "5", "6", "7", "8", "9" },

  -- rules: open windows on a tag or floating. class / instance / title are
  -- matched by substring; run `xprop`, click a window and look at WM_CLASS
  -- ("instance", "class") and WM_NAME (the title).
  rules = {
    { class = "Gimp", floating = true },
    -- { class = "firefox", tag = 2 },
    -- { class = "discord", tag = 9, monitor = 1 },   -- monitor: 0 = first
    -- { instance = "steamwebhelper", title = "Friends List", floating = true },
  },

  -- Layouts ------------------------------------------------------------------------

  -- the layout every tag starts with: "spiral", "dwindle", or "tile" for dwm's
  -- classic master and stack. Any layout from the list below.
  default_layout = "spiral",

  -- { symbol shown in the bar, layout }. Switch with the layout keys above.
  --   spiral  - a new window splits the window under the cursor; the splits
  --             spiral inwards (right, down, left, up)
  --   dwindle - like Hyprland: a new window splits the window under the cursor,
  --             on the side the cursor is on
  --   tile    - dwm: master on the left, the rest stacked on the right
  --   float   - no tiling
  --   monocle - one window at a time, full size
  --   bstack  - master on top, the rest side by side below
  --   grid    - everything in an even grid
  --   deck    - master on the left, the rest on top of each other on the right
  layouts = {
    { "S",   "spiral" },
    { "D",   "dwindle" },
    { "[]=", "tile" },
    { "><>", "float" },
    { "[M]", "monocle" },
    -- { "TTT", "bstack" },
    -- { "###", "grid" },
    -- { "[D]", "deck" },
  },

  mfact   = 0.55,                  -- size of the master area / first window (0.05 - 0.95)
  nmaster = 1,                     -- windows in the master area (tile)

  -- Windows and mouse ----------------------------------------------------------

  -- dragging a tiled window with Mod + left mouse button:
  --   "float" - it becomes floating (like dwm)
  --   "swap"  - it swaps places with the window you drop it on (like Hyprland)
  drag_tiled = "float",

  focus_follows_mouse = true,      -- focus a window when the mouse moves onto it
  snap           = 32,             -- floating windows snap to screen edges within this many px
  resizehints    = false,          -- true = respect the size steps of terminals (can leave gaps)
  lockfullscreen = true,           -- Mod+j/k can't leave a fullscreen window
  -- refresh_rate = 60,            -- Hz, for moving / resizing with the mouse

  keys = keys,

  -- mouse buttons: { where, "Modifiers+Button", action }
  --   where: tagbar, layout (the symbol), title, status, client (a window), root
  --   Button1 = left, Button2 = middle, Button3 = right, Button4/5 = scroll
  --   On the tag bar, actions without an argument get the clicked tag.
  buttons = {
    { "client", "Mod+Button1", a.movemouse() },      -- move a window
    { "client", "Mod+Button3", a.resizemouse() },    -- resize a window
    { "client", "Mod+Button2", a.togglefloating() },
    { "tagbar", "Button1",     a.view() },           -- view the clicked tag
    { "tagbar", "Button3",     a.toggleview() },     -- also view it
    { "tagbar", "Mod+Button1", a.tag() },            -- move the window there
    { "tagbar", "Mod+Button3", a.toggletag() },      -- also put the window there
    { "layout", "Button1",     a.setlayout() },      -- previous layout
    { "layout", "Button3",     a.setlayout("monocle") },
    { "title",  "Button2",     a.zoom() },
    { "status", "Button2",     a.spawn(term) },
    -- { "tagbar", "Button4",  a.view() },           -- any action works here
  },

  -- Status bar --------------------------------------------------------------------

  -- the text on the right of the bar. Each entry: { source, "format", argument }
  --   %s is replaced by the value, %% is a literal %. Add `interval = N` to an
  --   entry to update it every N seconds instead of every second.
  -- sources:
  --   datetime ("%H:%M", strftime format)   cpu_perc   cpu_freq   load_avg   uptime
  --   ram_perc ram_used ram_free ram_total  swap_perc swap_used
  --   battery_perc battery_state battery_remaining ("BAT0")
  --   temp ("/sys/class/thermal/thermal_zone0/temp")
  --   disk_perc disk_free disk_used disk_total ("/")
  --   netspeed_rx netspeed_tx ipv4 wifi_perc ("wlan0")
  --   keymap (keyboard layout)  kernel_release  hostname  username
  --   num_files ("~/Mail/new")  entropy
  --   "run_command" with a shell command as argument (runs in the background)
  --   a Lua function that returns a string
  -- Set `status = nil` to show `xsetroot -name` text instead (e.g. from slstatus).
  status = {
    { "cpu_perc", "cpu %s%% | " },
    { "ram_perc", "ram %s%% | " },
    -- { "battery_perc", "bat %s%% | ", "BAT0" },
    -- { "keymap", "%s | " },
    -- { "run_command", "vol %s | ", "wpctl get-volume @DEFAULT_AUDIO_SINK@ | awk '{print $2*100}'", interval = 5 },
    -- { function() return os.getenv("USER") end, "%s | " },
    { "datetime", "%s", "%a %d %b %H:%M" },
  },

  -- Lock screen ----------------------------------------------------------------------

  -- Mod+Shift+l, `trwm lock`, or `xss-lock -- trwm lock` in ~/.xinitrc to lock
  -- on suspend. Type your password and press Enter. The colour shows the state.
  lock = {
    colors = {
      init   = "black",            -- locked
      input  = "#005577",          -- typing
      failed = "#CC3333",          -- wrong password
      -- text = "#ffffff",         -- clock / message text
    },
    fail_on_clear = true,          -- clearing the input with Escape shows the "failed" colour
    clock         = false,         -- true = show the time and date
    -- message    = "be right back",
    -- font       = "monospace:size=48",   -- clock font
    -- small_font = "monospace:size=14",   -- date and message font
    monitor_off   = 0,             -- switch the monitor off N seconds after locking (0 = never)
    idle          = 0,             -- lock by itself after N seconds without input (0 = never)
    -- pam_service = "trwm",       -- uses /etc/pam.d/trwm if it exists, else "login"
  },

  -- Startup --------------------------------------------------------------------------

  -- commands run once when trwm starts (not on reload or restart)
  autostart = {
    -- "dunst",                    -- notifications
    -- "picom -b",                 -- compositor (transparency, shadows)
    -- "nm-applet",                -- network
  },
}
