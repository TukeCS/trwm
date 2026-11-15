//! trwm — tuke's rust window manager.
//!
//! A dynamic tiling window manager for X11 inspired by dwm, with a built-in
//! status bar (slstatus) and screen locker (slock), configured in Lua with
//! hot reloading.

#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

mod bsp;
mod config;
mod doctor;
mod drw;
mod input;
mod outputs;
mod screen;
mod keyboard;
mod lock;
mod pam;
mod status;
mod wm;

use std::path::PathBuf;
use std::process::exit;

const USAGE: &str = "usage: trwm [-c config.lua] [command]

commands:
  (none)            run the window manager
  lock              lock the screen (e.g. xss-lock -- trwm lock)
  msg <lua|word>    send a command to the running trwm, e.g.
                      trwm msg reload
                      trwm msg lock
                      trwm msg 'trwm.view(3)'
                      trwm msg 'trwm.wallpaper(\"~/Pictures/wall.png\")'
  doctor            check your setup (programs, fonts, lock screen, X server,
                    config) and say how to fix what's missing
  types [dir]       write the editor files for autocomplete in the config
                    (types/trwm.lua and .luarc.json; trwm does this on start)
  status            print the status line once and exit
  keyboard [apply]  show the keyboard layout (and what the config sets), or
                    apply the config's keyboard settings now
  input [apply]     list mice/touchpads and their settings, or apply the config's
  monitors [apply]  list monitors (names for the config) and their modes, or
                    apply the config's monitor and screen settings
  check             check the config for errors and exit
  default-config    print the built-in default config
  -v, --version     print the version";

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg_path: Option<PathBuf> = None;
    if let Some(i) = args.iter().position(|a| a == "-c") {
        if i + 1 >= args.len() {
            die("-c needs a path");
        }
        cfg_path = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    let load = || match config::Config::load(cfg_path.as_deref()) {
        Ok(c) => c,
        Err(e) => die(&format!("config error: {e}")),
    };

    match args.first().map(String::as_str) {
        None => run_wm(cfg_path),
        Some("lock") => {
            let cfg = load();
            if let Err(e) = lock::run(&cfg.lock, cfg.dpi) {
                die(&format!("lock: {e}"));
            }
        }
        Some("msg") => {
            let msg = args[1..].join(" ");
            if msg.is_empty() {
                die("msg: nothing to send");
            }
            if let Err(e) = wm::send_message(&msg) {
                die(&e);
            }
        }
        Some("status") => {
            let cfg = load();
            let mut st = status::Status::new();
            st.reset(&cfg);
            st.tick(&cfg); // primes cpu/network counters
            std::thread::sleep(std::time::Duration::from_millis(500));
            st.refresh();
            st.wait_all(&cfg);
            println!("{}", st.text(&cfg));
        }
        Some("check") => {
            let cfg = load();
            match &cfg.path {
                Some(p) => println!("{}: ok", p.display()),
                None => println!("no config at {}, built-in default: ok", config::default_path().display()),
            }
            println!(
                "{} tags, {} layouts, {} keys, {} buttons, {} rules, {} status components",
                cfg.tags.len(),
                cfg.layouts.len(),
                cfg.keys.len(),
                cfg.buttons.len(),
                cfg.rules.len(),
                cfg.status.components.len()
            );
        }
        Some("doctor") => exit(doctor::run(cfg_path.as_deref())),
        Some("types") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| {
                cfg_path.as_deref().and_then(|p| p.parent()).map(PathBuf::from).unwrap_or_else(config::config_dir)
            });
            match config::write_editor_files(&dir) {
                Ok(w) if w.is_empty() => println!("editor files in {} are up to date", dir.display()),
                Ok(w) => w.iter().for_each(|p| println!("wrote {}", p.display())),
                Err(e) => die(&format!("types: {e}")),
            }
        }
        Some("keyboard") => keyboard_cmd(load(), args.get(1).map(String::as_str)),
        Some("input") => input_cmd(load(), args.get(1).map(String::as_str)),
        Some("monitors") => monitors_cmd(load(), args.get(1).map(String::as_str)),
        Some("default-config") => print!("{}", config::DEFAULT_CONFIG),
        Some("-v" | "--version") => println!("trwm-{}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => println!("{USAGE}"),
        Some(other) => die(&format!("unknown command '{other}'\n{USAGE}")),
    }
}

fn run_wm(cfg_path: Option<PathBuf>) {
    // first start: write the default config so there is a file to edit
    let first_start = cfg_path.is_none() && write_default_config().is_some();
    // a broken config must never leave you without a window manager:
    // fall back to the built-in default and show the error in the bar
    let (cfg, err) = match config::Config::load(cfg_path.as_deref()) {
        Ok(c) => (c, None),
        Err(e) => (
            config::Config::from_source(config::DEFAULT_CONFIG, "default config", None).unwrap_or_else(|e| die(&format!("built-in config: {e}"))),
            Some(e),
        ),
    };
    let watch = cfg_path.clone().or_else(|| Some(config::default_path()).filter(|p| p.exists()));
    // autocomplete for the config in editors (Lua language server)
    if let Some(dir) = watch.as_deref().and_then(|p| p.parent()) {
        let _ = config::write_editor_files(dir);
    }
    let mut wm = match wm::WindowManager::new(cfg, watch) {
        Ok(w) => w,
        Err(e) => die(&e),
    };
    if let Some(e) = err {
        wm.startup_error(format!("config error: {e}"));
    } else if first_start {
        wm.startup_message(
            "welcome! Mod+Return terminal · Mod+p launcher · Mod+Shift+q quit · config: ~/.config/trwm/config.lua · `trwm doctor` checks your setup".into(),
            90,
        );
    }
    if wm.run() {
        // restart: re-exec ourselves, keeping all windows (and their tags)
        use std::os::unix::process::CommandExt;
        let exe = wm::exe_path();
        let err = std::process::Command::new(exe).args(std::env::args().skip(1)).env("TRWM_RESTARTED", "1").exec();
        die(&format!("restart failed: {err}"));
    }
}

/// `trwm keyboard`: show the keyboard settings; `trwm keyboard apply`: apply
/// the `keyboard` section of the config now.
fn keyboard_cmd(cfg: config::Config, sub: Option<&str>) {
    unsafe {
        let dpy = x11::xlib::XOpenDisplay(std::ptr::null());
        if dpy.is_null() {
            die("cannot open display");
        }
        match sub {
            None | Some("show") => {
                match keyboard::current(dpy) {
                    Some(c) => println!(
                        "now:     layout={} variant={} model={} options={} (rules {})",
                        c.layout, c.variant, c.model, c.options, c.rules
                    ),
                    None => println!("now:     unknown (the X server didn't say)"),
                }
                println!("active:  {}", keyboard::active_name(dpy).unwrap_or_else(|| "?".into()));
                if cfg.keyboard.sets_layout() {
                    match keyboard::resolve(dpy, &cfg.keyboard) {
                        Ok(r) => println!(
                            "config:  layout={} variant={} model={} options={}\n         -> keycodes={} types={} compat={} symbols={}",
                            r.layout, r.variant, r.model, r.options, r.keycodes, r.types, r.compat, r.symbols
                        ),
                        Err(e) => println!("config:  {e}"),
                    }
                } else {
                    println!("config:  no keyboard layout set");
                }
            }
            Some("apply") => {
                if let Err(e) = keyboard::apply(dpy, &cfg.keyboard) {
                    die(&e);
                }
                println!("applied");
            }
            Some(o) => die(&format!("keyboard: unknown subcommand '{o}' (show, apply)")),
        }
        x11::xlib::XCloseDisplay(dpy);
    }
}

/// Write the default config to ~/.config/trwm/config.lua if there is none.
/// Returns its path if it was written.
fn write_default_config() -> Option<PathBuf> {
    let p = config::default_path();
    if p.exists() {
        return None;
    }
    std::fs::create_dir_all(p.parent()?).ok()?;
    std::fs::write(&p, config::DEFAULT_CONFIG).ok()?;
    eprintln!("trwm: wrote the default config to {}", p.display());
    Some(p)
}

fn open_display() -> *mut x11::xlib::Display {
    let dpy = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) };
    if dpy.is_null() {
        die("cannot open display");
    }
    dpy
}

/// `trwm input`: list mice and touchpads with their settings;
/// `trwm input apply`: apply the config's mouse/touchpad settings now.
fn input_cmd(cfg: config::Config, sub: Option<&str>) {
    let dpy = open_display();
    match sub {
        None | Some("show") => {
            let devs = input::devices(dpy);
            if devs.is_empty() {
                println!("no mice or touchpads found");
            }
            for d in devs {
                println!("{} ({}): {}", d.name, if d.touchpad { "touchpad" } else { "mouse" }, input::describe(dpy, &d));
            }
        }
        Some("apply") => {
            input::apply(dpy, &cfg.input);
            println!("applied");
        }
        Some(o) => die(&format!("input: unknown subcommand '{o}' (show, apply)")),
    }
    unsafe { x11::xlib::XCloseDisplay(dpy) };
}

/// `trwm monitors`: list the outputs (names for the config), their modes and
/// the screen blanking settings; `trwm monitors apply`: apply the config now.
fn monitors_cmd(cfg: config::Config, sub: Option<&str>) {
    let dpy = open_display();
    unsafe {
        let screen = x11::xlib::XDefaultScreen(dpy);
        let root = x11::xlib::XRootWindow(dpy, screen);
        match sub {
            None | Some("show") => {
                let Some(outs) = outputs::query(dpy, root) else { die("RandR is not available") };
                for o in &outs {
                    println!("{}", outputs::describe(o));
                }
                let (blank, off) = screen::current(dpy);
                let secs = |s: u32| if s == 0 { "never".to_string() } else { format!("after {s}s") };
                println!("screen: blank {}, monitor off {}", secs(blank), off.map_or("never".into(), secs));
                if !cfg.monitors.is_empty() {
                    match outputs::plan(&outs, &cfg.monitors) {
                        Ok((plans, (w, h), _)) => {
                            let n = plans.iter().filter(|p| p.mode != 0).count();
                            println!("config: {n} monitor(s) on, screen {w}x{h}");
                        }
                        Err(e) => println!("config: {e}"),
                    }
                }
            }
            Some("apply") => {
                screen::apply(dpy, &cfg.screen);
                match outputs::apply(dpy, root, screen, &cfg.monitors, cfg.dpi) {
                    Ok(true) => println!("applied"),
                    Ok(false) => println!("nothing to change"),
                    Err(e) => die(&e),
                }
            }
            Some(o) => die(&format!("monitors: unknown subcommand '{o}' (show, apply)")),
        }
        x11::xlib::XCloseDisplay(dpy);
    }
}

fn die(msg: &str) -> ! {
    eprintln!("trwm: {msg}");
    exit(1)
}
