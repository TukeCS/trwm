---@meta trwm
---@diagnostic disable: missing-fields
-- Type definitions for the trwm config, for editors that use the Lua
-- language server (Neovim, VS Code, ...). trwm writes this file to
-- ~/.config/trwm/types/ together with a .luarc.json, so autocomplete and
-- error checking work in your config without any setup.
-- Don't edit it: trwm replaces it when it is updated.

---------------------------------------------------------------------------
-- names
---------------------------------------------------------------------------

---@alias trwm.LayoutName
---| "spiral"   # a new window splits the window under the cursor; the splits spiral inwards
---| "dwindle"  # like Hyprland: a new window splits the window under the cursor, on the cursor's side
---| "tile"     # dwm: master on the left, the rest stacked on the right
---| "float"    # no tiling
---| "monocle"  # one window at a time, full size
---| "bstack"   # master on top, the rest side by side below
---| "grid"     # everything in an even grid
---| "deck"     # master on the left, the rest on top of each other on the right
---| "fibonacci" # same as dwindle

---@alias trwm.StatusSource
---| "datetime"          # date/time; argument: strftime format, e.g. "%H:%M"
---| "cpu_perc"          # CPU usage in %
---| "cpu_freq"          # CPU frequency
---| "load_avg"          # load average
---| "uptime"
---| "ram_perc"
---| "ram_used"
---| "ram_free"
---| "ram_total"
---| "swap_perc"
---| "swap_used"
---| "battery_perc"      # argument: battery name, e.g. "BAT0"
---| "battery_state"     # + charging, - discharging, o full; argument: "BAT0"
---| "battery_remaining" # time left; argument: "BAT0"
---| "temp"              # °C; argument: e.g. "/sys/class/thermal/thermal_zone0/temp"
---| "disk_perc"         # argument: a path, e.g. "/"
---| "disk_free"
---| "disk_used"
---| "disk_total"
---| "netspeed_rx"       # download speed; argument: interface, e.g. "wlan0"
---| "netspeed_tx"       # upload speed
---| "ipv4"              # argument: interface
---| "wifi_perc"         # signal strength; argument: interface
---| "keymap"            # the active keyboard layout
---| "kernel_release"
---| "hostname"
---| "username"
---| "num_files"         # files in a folder; argument: the folder
---| "entropy"
---| "run_command"       # argument: a shell command; runs in the background

---@alias trwm.ClickArea
---| "tagbar"  # the tags
---| "layout"  # the layout symbol
---| "title"   # the window title
---| "status"  # the status text
---| "client"  # a window
---| "root"    # the desktop

---@alias trwm.Command string|string[] a shell command (string) or a program with its arguments (list)

---@alias trwm.Tag integer|integer[] a tag number (1-based), 0 for all tags, or a list of tags

---------------------------------------------------------------------------
-- the config table
---------------------------------------------------------------------------

---@alias trwm.Key { [1]: string, [2]: function }
--- { "Mod+Shift+Return", action }. Modifiers: Mod, Super, Alt, Shift, Ctrl.

---@alias trwm.Button { [1]: trwm.ClickArea, [2]: string, [3]: fun(tag: integer?) }
--- { where, "Mod+Button1", action }. Button1 left, 2 middle, 3 right, 4/5 scroll.

---@alias trwm.Layout { [1]: string, [2]: trwm.LayoutName }
--- { symbol shown in the bar, layout }

---@class trwm.StatusEntry
---@field [1] trwm.StatusSource|fun(): string? what to show: a builtin, "run_command", or a Lua function
---@field [2] string? format: %s is the value, %% a literal % (default "%s")
---@field [3] string? argument for the builtin, or the command for "run_command"
---@field interval integer? seconds between updates (default: status.interval)

---@class trwm.Status
---@field interval integer? seconds between updates (default 1)
---@field enabled boolean?
---@field [integer] trwm.StatusEntry

---@class trwm.Rule
---@field class string? matches the window class (substring); see `xprop` WM_CLASS
---@field instance string? matches the window instance (substring)
---@field title string? matches the window title (substring)
---@field tag trwm.Tag? put it on this tag
---@field tags trwm.Tag? put it on these tags
---@field floating boolean? make it float
---@field monitor integer? put it on this monitor (0 = first)

---@class trwm.Gaps
---@field inner integer? px between windows
---@field outer integer? px between windows and the screen edge
---@field inner_h integer?
---@field inner_v integer?
---@field outer_h integer?
---@field outer_v integer?
---@field smart boolean? no outer gap with a single window (default true)
---@field enabled boolean? start with gaps on (default true)

---@class trwm.Scheme
---@field fg string? text colour, e.g. "#eeeeee"
---@field bg string? background colour
---@field border string? window border colour

---@class trwm.Colors
---@field norm trwm.Scheme? normal
---@field sel trwm.Scheme? selected: the focused window, the current tag

---@class trwm.LockColors
---@field init string? locked
---@field input string? typing
---@field failed string? wrong password
---@field text string? clock / message text

---@class trwm.Lock
---@field colors trwm.LockColors?
---@field fail_on_clear boolean? clearing the input shows the "failed" colour
---@field clock boolean? show the time and date
---@field message string? text shown under the clock
---@field font string? clock font, e.g. "monospace:size=48"
---@field small_font string? date and message font
---@field monitor_off integer? switch the monitor off N seconds after locking (0 = never)
---@field idle integer? lock by itself after N seconds without input (0 = never)
---@field pam_service string? PAM service (default "trwm", falls back to "login")
---@field lockout (boolean|trwm.Lockout)? wait after too many wrong passwords (default false; true = 3 attempts, 10 minutes)

---@class trwm.Lockout
---@field attempts integer? wrong passwords in a row before the wait (default 3)
---@field minutes number? how long typing is ignored (default 10)

---@class trwm.Wallpaper
---@field path string an image, or a folder to pick a random image from
---@field mode ("fill"|"center"|"max"|"scale"|"tile")?
---@field interval integer? seconds until the next random image (folders only)

---@class trwm.Keyboard
---@field layout (string|string[])? e.g. "us" or { "us", "fr" }; see /usr/share/X11/xkb/rules/base.lst
---@field variant (string|string[])? one per layout, e.g. { "", "azerty" }
---@field model string? e.g. "pc105"
---@field options (string|string[])? e.g. { "caps:escape", "grp:alt_shift_toggle" }
---@field repeat_delay integer? ms before a held key starts repeating
---@field repeat_rate integer? repeats per second
---@field numlock boolean? num lock on when trwm starts

---@class trwm.Pointer
---@field speed number? -1.0 (slowest) to 1.0 (fastest)
---@field accel ("flat"|"adaptive")? "flat" = no acceleration (mouse default), "adaptive" = acceleration
---@field natural_scroll boolean? scroll like on a phone
---@field left_handed boolean? swap the left and right buttons
---@field middle_emulation boolean? left + right together = middle click

---@class trwm.Touchpad: trwm.Pointer
---@field tap boolean? tap to click
---@field disable_while_typing boolean?

---@class trwm.Monitor
---@field output string the output name from `trwm monitors`
---@field mode string? resolution, e.g. "1920x1080" (default: the monitor's preferred one)
---@field rate number? refresh rate in Hz (default: the highest)
---@field right_of string? place it right of this output
---@field left_of string?
---@field above string?
---@field below string?
---@field same_as string? mirror this output
---@field pos (string|integer[])? exact position, e.g. "1920x0" or { 1920, 0 }
---@field primary boolean?
---@field rotate ("normal"|"left"|"right"|"inverted")?
---@field off boolean? switch this output off

---@class trwm.Screen
---@field blank (integer|false)? seconds without input before the screen blanks (false = never)
---@field off (integer|false)? seconds without input before the monitor switches off (false = never)

---@class trwm.Config
---@field modkey ("Super"|"Alt"|"Mod4"|"Mod1")? what "Mod" means in keys and buttons
---@field fonts (string|string[])? bar fonts; later ones are used for missing characters
---@field dpi integer? screen DPI: 96 normal, 144 = 150%, 192 = 200%
---@field cursor_size integer? mouse cursor size in pixels
---@field cursor_theme string? cursor theme from /usr/share/icons or ~/.icons
---@field dark_mode boolean? dark look for GTK/libadwaita/portal apps (unset = leave them alone)
---@field screen trwm.Screen? screen blanking and monitor power saving
---@field monitors trwm.Monitor[]? monitor resolution and arrangement
---@field keyboard (string|trwm.Keyboard)? keyboard layout, key repeat, num lock
---@field mouse trwm.Pointer? settings for every mouse
---@field touchpad trwm.Touchpad? settings for every touchpad
---@field borderpx integer? window border width
---@field gaps trwm.Gaps? gaps between windows
---@field colors trwm.Colors?
---@field showbar boolean? show the bar
---@field topbar boolean? false = bar at the bottom
---@field hide_empty_tags boolean? only show tags that have windows
---@field wallpaper (string|trwm.Wallpaper|false)? set with feh
---@field tags string[]? tag names shown in the bar
---@field rules trwm.Rule[]? open windows on a tag / floating
---@field default_layout (trwm.LayoutName|string)? the layout every tag starts with (name or symbol)
---@field layouts trwm.Layout[]?
---@field mfact number? size of the master area / first window, 0.05 - 0.95
---@field nmaster integer? windows in the master area (tile)
---@field drag_tiled ("float"|"swap")? Mod+drag on a tiled window: float it (dwm) or swap it (Hyprland)
---@field focus_follows_mouse boolean?
---@field snap integer? floating windows snap to edges within this many px
---@field resizehints boolean? respect the size steps of terminals
---@field lockfullscreen boolean? Mod+j/k can't leave a fullscreen window
---@field refresh_rate integer? Hz, for moving / resizing with the mouse
---@field keys trwm.Key[]?
---@field buttons trwm.Button[]?
---@field status trwm.Status? the status bar text
---@field lock trwm.Lock? the lock screen
---@field autostart string[]? commands run once when trwm starts

---------------------------------------------------------------------------
-- the trwm module
---------------------------------------------------------------------------

---@class trwm.Actions
--- `a.spawn("kitty")` is short for `function() trwm.spawn("kitty") end`.
---@field spawn fun(cmd: trwm.Command): function run a program
---@field view fun(tag?: trwm.Tag): function view a tag (no argument: the previous one)
---@field toggleview fun(tag?: trwm.Tag): function also view a tag
---@field tag fun(tag?: trwm.Tag): function move the focused window to a tag
---@field toggletag fun(tag?: trwm.Tag): function also put the focused window on a tag
---@field focusstack fun(dir: integer): function focus the next (1) / previous (-1) window
---@field incnmaster fun(n: integer): function more / fewer windows in the master area
---@field setmfact fun(delta: number): function resize the master area or split
---@field zoom fun(): function swap the focused window with the master / first window
---@field killclient fun(): function close the focused window
---@field setlayout fun(layout?: trwm.LayoutName|string|integer): function switch layout (no argument: the previous one)
---@field togglelayout fun(a: trwm.LayoutName|string, b: trwm.LayoutName|string): function switch between two layouts
---@field cyclelayout fun(dir: integer): function go through the layouts
---@field togglefloating fun(): function
---@field togglefullscreen fun(): function
---@field togglebar fun(): function
---@field retile fun(): function put every floated window back into the tiling
---@field focusmon fun(dir: integer): function focus the next / previous monitor
---@field tagmon fun(dir: integer): function send the window to the next / previous monitor
---@field movemouse fun(): function move the window with the mouse (for buttons)
---@field resizemouse fun(): function resize the window with the mouse (for buttons)
---@field togglegaps fun(): function
---@field defaultgaps fun(): function
---@field incgaps fun(n: integer): function bigger / smaller gaps
---@field wallpaper fun(path?: string, mode?: string): function set the wallpaper (no argument: from the config)
---@field nextkeyboard fun(dir?: integer): function next keyboard layout
---@field setkeyboard fun(layout: string|integer): function a keyboard layout by name or number
---@field refresh fun(): function update the status bar now
---@field lock fun(): function lock the screen
---@field reload fun(): function reload the config
---@field restart fun(): function restart trwm, keeping all windows
---@field quit fun(): function quit trwm

---@class trwm.Sys
--- The status bar builtins as functions, for your own status functions.
---@field datetime fun(format?: string): string
---@field cpu_perc fun(): string
---@field cpu_freq fun(): string
---@field load_avg fun(): string
---@field uptime fun(): string
---@field ram_perc fun(): string
---@field ram_used fun(): string
---@field ram_free fun(): string
---@field ram_total fun(): string
---@field swap_perc fun(): string
---@field swap_used fun(): string
---@field battery_perc fun(battery?: string): string
---@field battery_state fun(battery?: string): string
---@field battery_remaining fun(battery?: string): string
---@field temp fun(file?: string): string
---@field disk_perc fun(path?: string): string
---@field disk_free fun(path?: string): string
---@field disk_used fun(path?: string): string
---@field disk_total fun(path?: string): string
---@field netspeed_rx fun(iface?: string): string
---@field netspeed_tx fun(iface?: string): string
---@field ipv4 fun(iface?: string): string
---@field wifi_perc fun(iface?: string): string
---@field keymap fun(): string
---@field kernel_release fun(): string
---@field hostname fun(): string
---@field username fun(): string
---@field num_files fun(dir?: string): string
---@field entropy fun(): string

---@class trwm.Module
---@field action trwm.Actions actions for key and button bindings
---@field sys trwm.Sys status bar builtins
---@field home string your home folder
---@field config_dir string the folder this config is in
---@field version string the trwm version
trwm = {}

---Run a program: a string runs through the shell, a list runs directly.
---@param cmd trwm.Command
function trwm.spawn(cmd) end

---View a tag (1-based; 0 = all; no argument = the previous tags).
---@param tag? trwm.Tag
function trwm.view(tag) end

---Also view a tag.
---@param tag trwm.Tag
function trwm.toggleview(tag) end

---Move the focused window to a tag.
---@param tag trwm.Tag
function trwm.tag(tag) end

---Also put the focused window on a tag.
---@param tag trwm.Tag
function trwm.toggletag(tag) end

---Focus the next (1) or previous (-1) window.
---@param dir integer
function trwm.focusstack(dir) end

---@param n integer
function trwm.incnmaster(n) end

---Resize the master area (or, in spiral/dwindle, the split next to the focused window).
---@param delta number e.g. 0.05 or -0.05
function trwm.setmfact(delta) end

function trwm.zoom() end
function trwm.killclient() end

---Switch layout by name, symbol or number; no argument = the previous layout.
---@param layout? trwm.LayoutName|string|integer
function trwm.setlayout(layout) end

---Switch to `b` if `a` is active, otherwise to `a`.
---@param a trwm.LayoutName|string
---@param b trwm.LayoutName|string
function trwm.togglelayout(a, b) end

---@param dir integer
function trwm.cyclelayout(dir) end

function trwm.togglefloating() end
function trwm.togglefullscreen() end
function trwm.togglebar() end

---Put every window you floated back into the tiling.
function trwm.retile() end

---@param dir integer
function trwm.focusmon(dir) end

---@param dir integer
function trwm.tagmon(dir) end

function trwm.movemouse() end
function trwm.resizemouse() end
function trwm.togglegaps() end
function trwm.defaultgaps() end

---@param n integer
function trwm.incgaps(n) end

---Set the wallpaper with feh. No argument: the one from the config.
---@param path? string
---@param mode? "fill"|"center"|"max"|"scale"|"tile"
function trwm.wallpaper(path, mode) end

---@param dir? integer
function trwm.nextkeyboard(dir) end

---@param layout string|integer
function trwm.setkeyboard(layout) end

---Update the status bar now.
function trwm.refresh() end
function trwm.lock() end
function trwm.reload() end
function trwm.restart() end
function trwm.quit() end

---Run a shell command and return its output. Blocks trwm until it finishes,
---so keep it quick (use "run_command" in the status bar for slow commands).
---@param cmd string
---@return string
function trwm.sh(cmd) end

---@type trwm.Actions
trwm.action = {}

---@type trwm.Sys
trwm.sys = {}

---@type string
trwm.home = ""

---@type string
trwm.config_dir = ""

---@type string
trwm.version = ""
