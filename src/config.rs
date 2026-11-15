//! Lua configuration.
//!
//! The config file is plain Lua that `return`s a table. Every key binding is
//! just a Lua function; the `trwm.*` functions it calls push [`Cmd`]s onto a
//! queue that the window manager drains after the call returns.

use std::cell::RefCell;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use mlua::{Function, Lua, Table, Value};
use x11::xlib;

pub const DEFAULT_CONFIG: &str = include_str!("../config.lua");

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Spawn {
    Shell(String),
    Argv(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum LayoutSel {
    Toggle,
    Index(usize),
    Name(String),
}

#[derive(Debug, Clone)]
pub enum Cmd {
    Spawn(Spawn),
    /// None = go back to the previous tag set
    View(Option<u32>),
    ToggleView(u32),
    Tag(u32),
    ToggleTag(u32),
    FocusStack(i32),
    IncNMaster(i32),
    SetMFact(f32),
    Zoom,
    KillClient,
    SetLayout(LayoutSel),
    CycleLayout(i32),
    ToggleFloating,
    Retile,
    ToggleFullscreen,
    ToggleBar,
    FocusMon(i32),
    TagMon(i32),
    MoveMouse,
    ResizeMouse,
    ToggleGaps,
    DefaultGaps,
    IncGaps(i32),
    Wallpaper(Option<String>, Option<String>),
    RefreshStatus,
    Lock,
    Reload,
    Restart,
    Quit,
}

pub type Queue = Rc<RefCell<Vec<Cmd>>>;

// ---------------------------------------------------------------------------
// config data
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutKind {
    Tile,
    Monocle,
    Float,
    BStack,
    Grid,
    Deck,
    Dwindle,
    Spiral,
}

#[derive(Clone, Debug)]
pub struct LayoutDef {
    pub symbol: String,
    pub kind: LayoutKind,
}

#[derive(Clone, Debug, Default)]
pub struct Rule {
    pub class: Option<String>,
    pub instance: Option<String>,
    pub title: Option<String>,
    pub tags: u32,
    pub floating: bool,
    pub monitor: i32,
}

pub struct Key {
    pub mods: u32,
    pub keysym: xlib::KeySym,
    pub func: Function,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Click {
    TagBar,
    LtSymbol,
    StatusText,
    WinTitle,
    ClientWin,
    RootWin,
}

pub struct Button {
    pub click: Click,
    pub mods: u32,
    pub button: u32,
    pub func: Function,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaps {
    pub ih: i32,
    pub iv: i32,
    pub oh: i32,
    pub ov: i32,
    pub smart: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchemeCfg {
    pub fg: String,
    pub bg: String,
    pub border: String,
}

pub enum StatusSource {
    /// builtin name + optional argument
    Builtin(String, Option<String>),
    Command(String),
    Lua(Function),
}

pub struct StatusComponent {
    pub source: StatusSource,
    pub fmt: String,
    /// seconds between updates
    pub interval: u64,
}

pub struct StatusCfg {
    pub enabled: bool,
    pub components: Vec<StatusComponent>,
}

#[derive(Clone, Debug)]
pub struct LockCfg {
    pub init: String,
    pub input: String,
    pub failed: String,
    pub text: String,
    pub fail_on_clear: bool,
    pub monitor_off: u16,
    pub pam_service: String,
    pub clock: bool,
    pub font: String,
    pub small_font: String,
    pub message: Option<String>,
    pub idle: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallpaperCfg {
    pub path: String,
    pub mode: String,
    pub interval: u64,
}

pub struct Config {
    pub lua: Lua,
    pub queue: Queue,
    pub path: Option<PathBuf>,

    pub fonts: Vec<String>,
    /// screen DPI (like `xrandr --dpi` + `Xft.dpi`); None = leave it alone
    pub dpi: Option<u32>,
    /// mouse cursor size in pixels (Xcursor.size / XCURSOR_SIZE)
    pub cursor_size: Option<u32>,
    /// mouse cursor theme (Xcursor.theme / XCURSOR_THEME)
    pub cursor_theme: Option<String>,
    pub border_px: u32,
    pub snap: u32,
    pub gaps: Gaps,
    pub show_bar: bool,
    pub top_bar: bool,
    pub hide_empty_tags: bool,
    pub norm: SchemeCfg,
    pub sel: SchemeCfg,
    pub tags: Vec<String>,
    pub rules: Vec<Rule>,
    pub mfact: f32,
    pub nmaster: i32,
    pub resize_hints: bool,
    pub lock_fullscreen: bool,
    pub refresh_rate: u32,
    /// dragging a tiled window swaps it with the one under the pointer (else it floats, like dwm)
    pub drag_swap: bool,
    pub focus_follows_mouse: bool,
    pub layouts: Vec<LayoutDef>,
    pub keys: Vec<Key>,
    pub buttons: Vec<Button>,
    pub status: StatusCfg,
    pub lock: LockCfg,
    pub wallpaper: Option<WallpaperCfg>,
    pub autostart: Vec<String>,
}

// ---------------------------------------------------------------------------
// paths
// ---------------------------------------------------------------------------

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

pub fn expand_tilde(p: &str) -> String {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", home(), rest)
    } else {
        p.to_string()
    }
}

pub fn config_dir() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d).join("trwm"),
        _ => PathBuf::from(home()).join(".config/trwm"),
    }
}

pub fn default_path() -> PathBuf {
    config_dir().join("config.lua")
}

/// Newest modification time of the config file and any .lua file next to it
/// (so `require`d modules also trigger a reload).
pub fn newest_mtime(path: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(dir) = path.parent()
        && let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "lua")
                    && let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                        newest = Some(newest.map_or(t, |n| n.max(t)));
                    }
            }
        }
    newest
}

// ---------------------------------------------------------------------------
// helpers for converting Lua values
// ---------------------------------------------------------------------------

fn lerr(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(msg.into())
}

fn tag_mask(v: &Value) -> mlua::Result<u32> {
    Ok(match v {
        Value::Integer(0) => !0,
        Value::Integer(n) if *n >= 1 && *n <= 31 => 1u32 << (*n - 1),
        Value::Number(n) if *n >= 1.0 && *n <= 31.0 => 1u32 << (*n as u32 - 1),
        Value::Table(t) => {
            let mut m = 0;
            for v in t.sequence_values::<Value>() {
                m |= tag_mask(&v?)?;
            }
            m
        }
        other => return Err(lerr(format!("expected a tag number (1-based, 0 = all) or list, got {other:?}"))),
    })
}

fn opt_tag_mask(v: &Value) -> mlua::Result<Option<u32>> {
    if v.is_nil() { Ok(None) } else { tag_mask(v).map(Some) }
}

fn int_arg(v: &Value, default: i64) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Number(n) => *n as i64,
        _ => default,
    }
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string_lossy()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn spawn_arg(v: &Value) -> mlua::Result<Spawn> {
    match v {
        Value::String(s) => Ok(Spawn::Shell(s.to_string_lossy())),
        Value::Table(t) => {
            let mut argv = Vec::new();
            for v in t.sequence_values::<Value>() {
                argv.push(str_of(&v?).ok_or_else(|| lerr("spawn: argv entries must be strings"))?);
            }
            if argv.is_empty() {
                return Err(lerr("spawn: empty command"));
            }
            Ok(Spawn::Argv(argv))
        }
        _ => Err(lerr("spawn: expected a string (shell command) or a list of arguments")),
    }
}

pub fn parse_mods(token: &str, modkey: u32) -> Option<u32> {
    Some(match token.to_ascii_lowercase().as_str() {
        "mod" => modkey,
        "super" | "mod4" | "win" | "logo" => xlib::Mod4Mask,
        "alt" | "mod1" | "meta" => xlib::Mod1Mask,
        "shift" => xlib::ShiftMask,
        "ctrl" | "control" => xlib::ControlMask,
        "mod2" => xlib::Mod2Mask,
        "mod3" => xlib::Mod3Mask,
        "mod5" | "altgr" => xlib::Mod5Mask,
        _ => return None,
    })
}

/// "Mod+Shift+Return" -> (mods, "Return")
fn split_combo(spec: &str, modkey: u32) -> Result<(u32, String), String> {
    let parts: Vec<&str> = spec.split('+').collect();
    let (last, mods) = parts.split_last().ok_or("empty key")?;
    let mut m = 0;
    for p in mods {
        m |= parse_mods(p.trim(), modkey).ok_or_else(|| format!("unknown modifier '{p}' in '{spec}'"))?;
    }
    Ok((m, last.trim().to_string()))
}

/// Add `:dpi=N` to font patterns that don't set one, so trwm's own text
/// scales the same way as other apps.
pub fn with_dpi(fonts: &[String], dpi: Option<u32>) -> Vec<String> {
    fonts
        .iter()
        .map(|f| match dpi {
            Some(d) if !f.contains("dpi=") => format!("{f}:dpi={d}"),
            _ => f.clone(),
        })
        .collect()
}

/// Set `key: value` in an X resource string (the RESOURCE_MANAGER property),
/// replacing any existing entry for `key` and keeping everything else.
pub fn set_resource(res: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}:");
    let mut out: String = res.lines().filter(|l| !l.trim_start().starts_with(&prefix)).map(|l| format!("{l}\n")).collect();
    out.push_str(&format!("{key}:\t{value}\n"));
    out
}

pub fn keysym(name: &str) -> xlib::KeySym {
    let Ok(c) = CString::new(name) else { return 0 };
    let ks = unsafe { xlib::XStringToKeysym(c.as_ptr()) };
    if ks != 0 {
        return ks;
    }
    // be forgiving: "return" -> "Return", "A" -> "a"
    let mut chars = name.chars();
    let alt = match chars.next() {
        Some(f) if name.len() > 1 => f.to_uppercase().collect::<String>() + chars.as_str(),
        _ => name.to_lowercase(),
    };
    let c = CString::new(alt).unwrap();
    unsafe { xlib::XStringToKeysym(c.as_ptr()) }
}

fn get<T: mlua::FromLua>(t: &Table, k: &str, default: T) -> mlua::Result<T> {
    let v: Option<T> = t.get(k).map_err(|e| lerr(format!("config key '{k}': {e}")))?;
    Ok(v.unwrap_or(default))
}

fn scheme_from(t: Option<Table>, d: SchemeCfg) -> mlua::Result<SchemeCfg> {
    let Some(t) = t else { return Ok(d) };
    Ok(SchemeCfg {
        fg: get(&t, "fg", d.fg)?,
        bg: get(&t, "bg", d.bg)?,
        border: get(&t, "border", d.border)?,
    })
}

// ---------------------------------------------------------------------------
// the `trwm` Lua module
// ---------------------------------------------------------------------------

fn install_api(lua: &Lua, queue: &Queue, dir: &Path) -> mlua::Result<()> {
    let trwm = lua.create_table()?;

    macro_rules! cmd {
        ($name:expr, |$v:ident| $body:expr) => {{
            let q = queue.clone();
            let f = lua.create_function(move |_, $v: mlua::Variadic<Value>| {
                let c: Cmd = $body;
                q.borrow_mut().push(c);
                Ok(())
            })?;
            trwm.set($name, f)?;
        }};
    }
    let a0 = |v: &mlua::Variadic<Value>| v.first().cloned().unwrap_or(Value::Nil);
    let a1 = |v: &mlua::Variadic<Value>| v.get(1).cloned().unwrap_or(Value::Nil);

    cmd!("spawn", |v| Cmd::Spawn(spawn_arg(&a0(&v))?));
    cmd!("view", |v| Cmd::View(opt_tag_mask(&a0(&v))?));
    cmd!("toggleview", |v| Cmd::ToggleView(tag_mask(&a0(&v))?));
    cmd!("tag", |v| Cmd::Tag(tag_mask(&a0(&v))?));
    cmd!("toggletag", |v| Cmd::ToggleTag(tag_mask(&a0(&v))?));
    cmd!("focusstack", |v| Cmd::FocusStack(int_arg(&a0(&v), 1) as i32));
    cmd!("incnmaster", |v| Cmd::IncNMaster(int_arg(&a0(&v), 1) as i32));
    cmd!("setmfact", |v| Cmd::SetMFact(match a0(&v) {
        Value::Number(n) => n as f32,
        Value::Integer(i) => i as f32,
        _ => 0.0,
    }));
    cmd!("zoom", |_v| Cmd::Zoom);
    cmd!("killclient", |_v| Cmd::KillClient);
    cmd!("setlayout", |v| Cmd::SetLayout(match a0(&v) {
        Value::Nil => LayoutSel::Toggle,
        Value::Integer(i) if i >= 1 => LayoutSel::Index(i as usize - 1),
        Value::String(s) => LayoutSel::Name(s.to_string_lossy()),
        other => return Err(lerr(format!("setlayout: bad argument {other:?}"))),
    }));
    cmd!("cyclelayout", |v| Cmd::CycleLayout(int_arg(&a0(&v), 1) as i32));
    cmd!("togglefloating", |_v| Cmd::ToggleFloating);
    cmd!("retile", |_v| Cmd::Retile);
    cmd!("togglefullscreen", |_v| Cmd::ToggleFullscreen);
    cmd!("togglebar", |_v| Cmd::ToggleBar);
    cmd!("focusmon", |v| Cmd::FocusMon(int_arg(&a0(&v), 1) as i32));
    cmd!("tagmon", |v| Cmd::TagMon(int_arg(&a0(&v), 1) as i32));
    cmd!("movemouse", |_v| Cmd::MoveMouse);
    cmd!("resizemouse", |_v| Cmd::ResizeMouse);
    cmd!("togglegaps", |_v| Cmd::ToggleGaps);
    cmd!("defaultgaps", |_v| Cmd::DefaultGaps);
    cmd!("incgaps", |v| Cmd::IncGaps(int_arg(&a0(&v), 1) as i32));
    cmd!("wallpaper", |v| Cmd::Wallpaper(str_of(&a0(&v)), str_of(&a1(&v))));
    cmd!("refresh", |_v| Cmd::RefreshStatus);
    cmd!("lock", |_v| Cmd::Lock);
    cmd!("reload", |_v| Cmd::Reload);
    cmd!("restart", |_v| Cmd::Restart);
    cmd!("quit", |_v| Cmd::Quit);

    // trwm.sh(cmd) -> output (blocking! keep it quick)
    trwm.set(
        "sh",
        lua.create_function(|_, cmd: String| {
            let out = std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).output().map_err(mlua::Error::external)?;
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        })?,
    )?;

    // trwm.sys.<builtin>(arg) gives Lua access to the slstatus-style modules
    let sys = lua.create_table()?;
    for name in crate::status::BUILTINS {
        let f = lua.create_function(move |_, arg: Option<String>| Ok(crate::status::builtin(name, arg.as_deref())))?;
        sys.set(*name, f)?;
    }
    trwm.set("sys", sys)?;

    trwm.set("home", home())?;
    trwm.set("config_dir", dir.to_string_lossy().to_string())?;
    trwm.set("version", env!("CARGO_PKG_VERSION"))?;
    lua.globals().set("trwm", trwm)?;

    // `trwm.action.X(args...)` returns a function that calls `trwm.X(args...)`.
    // When it is invoked with no preset args (e.g. a tag-bar click) it forwards
    // its own arguments instead.
    lua.load(
        r#"
        trwm.action = setmetatable({}, { __index = function(_, name)
            local f = trwm[name]
            if type(f) ~= "function" then error("trwm.action: unknown action '" .. tostring(name) .. "'", 2) end
            return function(...)
                local pre = table.pack(...)
                return function(...)
                    if pre.n > 0 then return f(table.unpack(pre, 1, pre.n)) end
                    return f(...)
                end
            end
        end })
        package.path = trwm.config_dir .. "/?.lua;" .. trwm.config_dir .. "/?/init.lua;" .. package.path
        "#,
    )
    .set_name("=trwm-prelude")
    .exec()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

impl Config {
    /// Load the user config, or the built-in default when no path exists.
    pub fn load(path: Option<&Path>) -> Result<Config, String> {
        let user = path.map(Path::to_path_buf).or_else(|| Some(default_path()));
        match user {
            Some(p) if p.exists() => {
                let src = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                Self::from_source(&src, &p.display().to_string(), Some(p))
            }
            _ => Self::from_source(DEFAULT_CONFIG, "default config", None),
        }
    }

    pub fn from_source(src: &str, name: &str, path: Option<PathBuf>) -> Result<Config, String> {
        let lua = Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let dir = path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(config_dir);
        install_api(&lua, &queue, &dir).map_err(|e| e.to_string())?;
        let table: Table = lua
            .load(src)
            .set_name(format!("@{name}"))
            .eval()
            .map_err(|e| e.to_string())?;
        let mut cfg = Self::parse(lua.clone(), queue, &table).map_err(|e| e.to_string())?;
        cfg.path = path;
        // Lua errors raised inside actions still go through the queue
        drop(table);
        Ok(cfg)
    }

    fn parse(lua: Lua, queue: Queue, t: &Table) -> mlua::Result<Config> {
        let modkey = parse_mods(&get(t, "modkey", "Super".to_string())?, xlib::Mod4Mask)
            .ok_or_else(|| lerr("modkey: unknown modifier"))?;

        // fonts: string or list
        let dpi: Option<u32> = t.get("dpi")?;
        if let Some(d) = dpi
            && !(30..=1000).contains(&d)
        {
            return Err(lerr(format!("dpi: {d} is out of range (30-1000)")));
        }

        let cursor_size: Option<u32> = t.get("cursor_size")?;
        if let Some(s) = cursor_size
            && !(8..=512).contains(&s)
        {
            return Err(lerr(format!("cursor_size: {s} is out of range (8-512)")));
        }

        let fonts = match t.get::<Value>("fonts")? {
            Value::String(s) => vec![s.to_string_lossy()],
            Value::Table(l) => l.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>()?,
            _ => match t.get::<Option<String>>("font")? {
                Some(f) => vec![f],
                None => vec!["monospace:size=10".into()],
            },
        };

        let gt: Option<Table> = t.get("gaps")?;
        let gaps = match gt {
            None => Gaps { ih: 0, iv: 0, oh: 0, ov: 0, smart: false, enabled: false },
            Some(g) => {
                let inner: i32 = get(&g, "inner", 0)?;
                let outer: i32 = get(&g, "outer", 0)?;
                Gaps {
                    ih: get(&g, "inner_h", inner)?,
                    iv: get(&g, "inner_v", inner)?,
                    oh: get(&g, "outer_h", outer)?,
                    ov: get(&g, "outer_v", outer)?,
                    smart: get(&g, "smart", true)?,
                    enabled: get(&g, "enabled", true)?,
                }
            }
        };

        let colors: Option<Table> = t.get("colors")?;
        let (norm, sel) = {
            let dn = SchemeCfg { fg: "#bbbbbb".into(), bg: "#222222".into(), border: "#444444".into() };
            let ds = SchemeCfg { fg: "#eeeeee".into(), bg: "#005577".into(), border: "#005577".into() };
            match colors {
                Some(c) => (scheme_from(c.get("norm")?, dn)?, scheme_from(c.get("sel")?, ds)?),
                None => (dn, ds),
            }
        };

        let tags: Vec<String> = match t.get::<Option<Table>>("tags")? {
            Some(l) => l.sequence_values::<Value>().map(|v| v.map(|v| str_of(&v).unwrap_or_default())).collect::<mlua::Result<_>>()?,
            None => (1..=9).map(|i| i.to_string()).collect(),
        };
        if tags.is_empty() || tags.len() > 31 {
            return Err(lerr("tags: need between 1 and 31 tags"));
        }

        let mut rules = Vec::new();
        if let Some(l) = t.get::<Option<Table>>("rules")? {
            for r in l.sequence_values::<Table>() {
                let r = r?;
                let mut tagsm = 0;
                if let Some(v) = r.get::<Option<Value>>("tag")? {
                    tagsm = tag_mask(&v)?;
                }
                if let Some(v) = r.get::<Option<Value>>("tags")? {
                    tagsm = tag_mask(&v)?;
                }
                rules.push(Rule {
                    class: r.get("class")?,
                    instance: r.get("instance")?,
                    title: r.get("title")?,
                    tags: tagsm,
                    floating: get(&r, "floating", false)?,
                    monitor: get::<i64>(&r, "monitor", -1)? as i32,
                });
            }
        }

        let mut layouts = Vec::new();
        match t.get::<Option<Table>>("layouts")? {
            Some(l) => {
                for e in l.sequence_values::<Table>() {
                    let e = e?;
                    let symbol: String = e.get(1)?;
                    let name: Option<String> = e.get(2)?;
                    let kind = match name.as_deref().unwrap_or("float") {
                        "tile" => LayoutKind::Tile,
                        "monocle" => LayoutKind::Monocle,
                        "float" | "floating" => LayoutKind::Float,
                        "bstack" => LayoutKind::BStack,
                        "grid" => LayoutKind::Grid,
                        "deck" => LayoutKind::Deck,
                        "dwindle" | "fibonacci" => LayoutKind::Dwindle,
                        "spiral" => LayoutKind::Spiral,
                        other => return Err(lerr(format!("layouts: unknown layout '{other}' (tile, monocle, float, bstack, grid, deck, dwindle, spiral)"))),
                    };
                    layouts.push(LayoutDef { symbol, kind });
                }
            }
            None => {
                layouts.push(LayoutDef { symbol: "[]=".into(), kind: LayoutKind::Tile });
                layouts.push(LayoutDef { symbol: "><>".into(), kind: LayoutKind::Float });
                layouts.push(LayoutDef { symbol: "[M]".into(), kind: LayoutKind::Monocle });
            }
        }
        if layouts.is_empty() {
            return Err(lerr("layouts: need at least one layout"));
        }

        let mut keys = Vec::new();
        if let Some(l) = t.get::<Option<Table>>("keys")? {
            for (i, e) in l.sequence_values::<Table>().enumerate() {
                let e = e?;
                let spec: String = e.get(1).map_err(|_| lerr(format!("keys[{}]: first entry must be a key like \"Mod+Return\"", i + 1)))?;
                let func: Function = e.get(2).map_err(|_| lerr(format!("keys[{}] ({spec}): second entry must be a function", i + 1)))?;
                let (mods, name) = split_combo(&spec, modkey).map_err(lerr)?;
                let ks = keysym(&name);
                if ks == 0 {
                    return Err(lerr(format!("keys[{}]: unknown key '{name}'", i + 1)));
                }
                keys.push(Key { mods, keysym: ks, func });
            }
        }

        let mut buttons = Vec::new();
        if let Some(l) = t.get::<Option<Table>>("buttons")? {
            for (i, e) in l.sequence_values::<Table>().enumerate() {
                let e = e?;
                let click: String = e.get(1)?;
                let spec: String = e.get(2)?;
                let func: Function = e.get(3)?;
                let click = match click.to_ascii_lowercase().as_str() {
                    "tagbar" | "tags" => Click::TagBar,
                    "layout" | "ltsymbol" => Click::LtSymbol,
                    "status" | "statustext" => Click::StatusText,
                    "title" | "wintitle" => Click::WinTitle,
                    "client" | "clientwin" => Click::ClientWin,
                    "root" | "rootwin" => Click::RootWin,
                    o => return Err(lerr(format!("buttons[{}]: unknown click target '{o}'", i + 1))),
                };
                let (mods, b) = split_combo(&spec, modkey).map_err(lerr)?;
                let b = b.to_ascii_lowercase();
                let num = b.strip_prefix("button").unwrap_or(&b).parse::<u32>().map_err(|_| lerr(format!("buttons[{}]: bad button '{b}'", i + 1)))?;
                buttons.push(Button { click, mods, button: num, func });
            }
        }

        // status (slstatus)
        let status = match t.get::<Option<Table>>("status")? {
            None => StatusCfg { enabled: false, components: vec![] },
            Some(s) => {
                let interval: u64 = get(&s, "interval", 1)?;
                let mut components = Vec::new();
                for e in s.sequence_values::<Table>() {
                    let e = e?;
                    let fmt: String = e.get::<Option<String>>(2)?.unwrap_or_else(|| "%s".into());
                    let arg: Option<String> = e.get::<Option<Value>>(3)?.as_ref().and_then(str_of);
                    let source = match e.get::<Value>(1)? {
                        Value::Function(f) => StatusSource::Lua(f),
                        Value::String(n) => {
                            let n = n.to_string_lossy();
                            if n == "run_command" {
                                StatusSource::Command(arg.ok_or_else(|| lerr("status: run_command needs a command"))?)
                            } else if crate::status::BUILTINS.contains(&n.as_str()) {
                                StatusSource::Builtin(n, arg)
                            } else {
                                return Err(lerr(format!("status: unknown component '{n}'")));
                            }
                        }
                        o => return Err(lerr(format!("status: bad component {o:?}"))),
                    };
                    let ci: u64 = get(&e, "interval", interval)?;
                    components.push(StatusComponent { source, fmt, interval: ci.max(1) });
                }
                StatusCfg { enabled: get(&s, "enabled", true)?, components }
            }
        };

        // lock (slock)
        let lock = {
            let l: Table = t.get::<Option<Table>>("lock")?.unwrap_or(lua.create_table()?);
            let c: Table = l.get::<Option<Table>>("colors")?.unwrap_or(lua.create_table()?);
            LockCfg {
                init: get(&c, "init", "black".to_string())?,
                input: get(&c, "input", "#005577".to_string())?,
                failed: get(&c, "failed", "#CC3333".to_string())?,
                text: get(&c, "text", "#ffffff".to_string())?,
                fail_on_clear: get(&l, "fail_on_clear", true)?,
                monitor_off: get::<u16>(&l, "monitor_off", 0)?,
                pam_service: get(&l, "pam_service", "trwm".to_string())?,
                clock: get(&l, "clock", false)?,
                font: get(&l, "font", "monospace:size=48".to_string())?,
                small_font: get(&l, "small_font", "monospace:size=14".to_string())?,
                message: l.get("message")?,
                idle: get(&l, "idle", 0)?,
            }
        };

        // wallpaper (feh)
        let wallpaper = match t.get::<Value>("wallpaper")? {
            Value::Nil | Value::Boolean(false) => None,
            Value::String(s) => Some(WallpaperCfg { path: s.to_string_lossy(), mode: "fill".into(), interval: 0 }),
            Value::Table(w) => Some(WallpaperCfg {
                path: w.get::<Option<String>>("path")?.or(w.get::<Option<String>>(1)?).ok_or_else(|| lerr("wallpaper: missing path"))?,
                mode: get(&w, "mode", "fill".to_string())?,
                interval: get(&w, "interval", 0)?,
            }),
            o => return Err(lerr(format!("wallpaper: expected a path or a table, got {o:?}"))),
        };
        if let Some(w) = &wallpaper
            && !matches!(w.mode.as_str(), "fill" | "center" | "max" | "scale" | "tile") {
                return Err(lerr("wallpaper.mode must be one of fill, center, max, scale, tile"));
            }

        let autostart = match t.get::<Option<Table>>("autostart")? {
            Some(l) => l.sequence_values::<String>().collect::<mlua::Result<_>>()?,
            None => vec![],
        };

        let mfact: f32 = get(t, "mfact", 0.55)?;
        Ok(Config {
            lua,
            queue,
            path: None,
            fonts,
            dpi,
            cursor_size,
            cursor_theme: t.get("cursor_theme")?,
            border_px: get(t, "borderpx", 1)?,
            snap: get(t, "snap", 32)?,
            gaps,
            show_bar: get(t, "showbar", true)?,
            top_bar: get(t, "topbar", true)?,
            hide_empty_tags: get(t, "hide_empty_tags", false)?,
            norm,
            sel,
            tags,
            rules,
            mfact: mfact.clamp(0.05, 0.95),
            nmaster: get(t, "nmaster", 1)?,
            resize_hints: get(t, "resizehints", false)?,
            lock_fullscreen: get(t, "lockfullscreen", true)?,
            refresh_rate: get::<u32>(t, "refresh_rate", 60)?.max(1),
            drag_swap: match get(t, "drag_tiled", "float".to_string())?.as_str() {
                "swap" => true,
                "float" => false,
                o => return Err(lerr(format!("drag_tiled: expected \"swap\" or \"float\", got '{o}'"))),
            },
            focus_follows_mouse: get(t, "focus_follows_mouse", true)?,
            layouts,
            keys,
            buttons,
            status,
            lock,
            wallpaper,
            autostart,
        })
    }

    /// Run a Lua function (a key/button action) and return the queued commands.
    pub fn call(&self, f: &Function, arg: Option<u32>) -> Result<Vec<Cmd>, String> {
        let r = match arg {
            Some(a) => f.call::<()>(a),
            None => f.call::<()>(()),
        };
        let cmds = std::mem::take(&mut *self.queue.borrow_mut());
        r.map_err(|e| e.to_string())?;
        Ok(cmds)
    }

    /// Evaluate a chunk of Lua (used by `trwm msg`).
    pub fn eval(&self, src: &str) -> Result<Vec<Cmd>, String> {
        let r = self.lua.load(src).set_name("=trwm msg").exec();
        let cmds = std::mem::take(&mut *self.queue.borrow_mut());
        r.map_err(|e| e.to_string())?;
        Ok(cmds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_is_added_to_fonts() {
        let f = vec!["monospace:size=12".to_string(), "Sym:size=13:dpi=96".to_string()];
        assert_eq!(with_dpi(&f, Some(144)), vec!["monospace:size=12:dpi=144", "Sym:size=13:dpi=96"]);
        assert_eq!(with_dpi(&f, None), f);
    }

    #[test]
    fn resource_is_replaced_not_duplicated() {
        let res = "Xft.dpi:\t96\nXcursor.size:\t36\n";
        assert_eq!(set_resource(res, "Xft.dpi", "144"), "Xcursor.size:\t36\nXft.dpi:\t144\n");
        assert_eq!(set_resource("", "Xft.dpi", "144"), "Xft.dpi:\t144\n");
    }

    #[test]
    fn bad_dpi_is_rejected() {
        assert!(Config::from_source("return { dpi = 5 }", "t", None).is_err());
        assert_eq!(Config::from_source("return { dpi = 144 }", "t", None).unwrap().dpi, Some(144));
        assert_eq!(Config::from_source("return {}", "t", None).unwrap().dpi, None);
        assert!(Config::from_source("return { cursor_size = 2 }", "t", None).is_err());
        let c = Config::from_source("return { cursor_size = 36, cursor_theme = 'Adwaita' }", "t", None).unwrap();
        assert_eq!((c.cursor_size, c.cursor_theme.as_deref()), (Some(36), Some("Adwaita")));
    }
}
