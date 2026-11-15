//! trwm — tuke's rust window manager.
//!
//! A dynamic tiling window manager for X11 inspired by dwm, with a built-in
//! status bar (slstatus) and screen locker (slock), configured in Lua with
//! hot reloading.

#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

mod bsp;
mod config;
mod drw;
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
  status            print the status line once and exit
  keyboard [apply]  show the keyboard layout (and what the config sets), or
                    apply the config's keyboard settings now
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
        Some("keyboard") => keyboard_cmd(load(), args.get(1).map(String::as_str)),
        Some("default-config") => print!("{}", config::DEFAULT_CONFIG),
        Some("-v" | "--version") => println!("trwm-{}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => println!("{USAGE}"),
        Some(other) => die(&format!("unknown command '{other}'\n{USAGE}")),
    }
}

fn run_wm(cfg_path: Option<PathBuf>) {
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
    let mut wm = match wm::WindowManager::new(cfg, watch) {
        Ok(w) => w,
        Err(e) => die(&e),
    };
    if let Some(e) = err {
        wm.startup_error(format!("config error: {e}"));
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

fn die(msg: &str) -> ! {
    eprintln!("trwm: {msg}");
    exit(1)
}
