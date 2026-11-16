//! `trwm doctor`: check that everything trwm and the config need is there,
//! and say how to fix what isn't.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use x11::xlib;

use crate::config::{self, Cmd, Config, Spawn, StatusSource};

struct Report {
    color: bool,
    problems: usize,
    warnings: usize,
}

impl Report {
    fn line(&self, tag: &str, code: &str, msg: &str) {
        if self.color {
            println!("  \x1b[{code}m{tag}\x1b[0m {msg}");
        } else {
            println!("  {tag} {msg}");
        }
    }
    fn ok(&self, msg: &str) {
        self.line(" ok ", "32", msg);
    }
    fn warn(&mut self, msg: &str, fix: &str) {
        self.warnings += 1;
        self.line("warn", "33", msg);
        self.fix(fix);
    }
    fn fail(&mut self, msg: &str, fix: &str) {
        self.problems += 1;
        self.line("FAIL", "31", msg);
        self.fix(fix);
    }
    fn fix(&self, fix: &str) {
        for l in fix.lines().filter(|l| !l.is_empty()) {
            if self.color {
                println!("       \x1b[2m{l}\x1b[0m");
            } else {
                println!("       {l}");
            }
        }
    }
    fn section(&self, name: &str) {
        if self.color {
            println!("\n\x1b[1m{name}\x1b[0m");
        } else {
            println!("\n{name}");
        }
    }
}

// ---------------------------------------------------------------------------
// distro / packages
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Distro {
    Gentoo,
    Arch,
    Debian,
    Fedora,
    Suse,
    Void,
    Alpine,
    Nix,
    Unknown,
}

fn distro() -> Distro {
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).unwrap_or_default();
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |w: &str| ids.split_whitespace().any(|x| x == w);
    if has("gentoo") {
        Distro::Gentoo
    } else if has("arch") {
        Distro::Arch
    } else if has("debian") || has("ubuntu") {
        Distro::Debian
    } else if has("fedora") || has("rhel") {
        Distro::Fedora
    } else if ids.contains("suse") {
        Distro::Suse
    } else if has("void") {
        Distro::Void
    } else if has("alpine") {
        Distro::Alpine
    } else if has("nixos") {
        Distro::Nix
    } else {
        Distro::Unknown
    }
}

/// The package that provides `prog`, when it isn't called the same.
fn package(prog: &str, d: Distro) -> String {
    match (prog, d) {
        ("wpctl", _) => "wireplumber".into(),
        ("nvim", _) => "neovim".into(),
        ("lxqt-policykit-agent", _) => "lxqt-policykit".into(),
        ("notify-send", Distro::Debian) => "libnotify-bin".into(),
        ("notify-send", _) => "libnotify".into(),
        ("fc-match", _) => "fontconfig".into(),
        ("firefox", Distro::Debian) => "firefox-esr".into(),
        _ => prog.into(),
    }
}

fn install_cmd(pkgs: &[String], d: Distro) -> String {
    let p = pkgs.join(" ");
    match d {
        Distro::Gentoo => format!("sudo emerge --ask {p}"),
        Distro::Arch => format!("sudo pacman -S --needed {p}"),
        Distro::Debian => format!("sudo apt install {p}"),
        Distro::Fedora => format!("sudo dnf install {p}"),
        Distro::Suse => format!("sudo zypper install {p}"),
        Distro::Void => format!("sudo xbps-install -S {p}"),
        Distro::Alpine => format!("sudo apk add {p}"),
        Distro::Nix => format!("add {p} to environment.systemPackages in configuration.nix"),
        Distro::Unknown => format!("install with your package manager: {p}"),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn which(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(config::expand_tilde(prog));
        return p.exists().then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(prog)).find(|p| p.is_file()))
}

const SHELL_WORDS: &[&str] = &[
    "cd", "echo", "printf", "test", "[", "[[", "true", "false", "exec", "export", "if", "then", "else", "fi", "for", "do", "done",
    "while", "case", "esac", "set", "read", "eval", "source", ".", "local", "return", "exit", "{", "}", "(", ")", "!", "time",
];

/// The programs a shell command line runs: the first word of every part
/// separated by `&&`, `||`, `;`, `|`, `&` or started by `$( )`. Separators
/// inside quotes don't count.
pub fn programs_in(cmd: &str) -> Vec<String> {
    // split into command parts, respecting quotes
    let mut parts: Vec<String> = vec![];
    let mut cur = String::new();
    let (mut sq, mut dq) = (false, false);
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !dq => {
                sq = !sq;
                cur.push(c);
            }
            '"' if !sq => {
                dq = !dq;
                cur.push(c);
            }
            '$' if !sq && chars.peek() == Some(&'(') => {
                chars.next();
                parts.push(std::mem::take(&mut cur));
            }
            ';' | '|' | '&' | '\n' if !sq && !dq => parts.push(std::mem::take(&mut cur)),
            '(' | ')' if !sq => cur.push(' '),
            _ => cur.push(c),
        }
    }
    parts.push(cur);

    let mut out = vec![];
    for part in parts {
        let words = part.split_whitespace().map(|w| w.trim_matches(|c| c == '"' || c == '\''));
        for w in words {
            if w.contains('=') && !w.starts_with('=') {
                continue; // VAR=value prefix
            }
            if matches!(w, "env" | "sudo" | "doas" | "nohup" | "setsid" | "command" | "exec") {
                continue;
            }
            if !w.is_empty() && !SHELL_WORDS.contains(&w) && !w.starts_with('$') && !w.starts_with('-') {
                out.push(w.to_string());
            }
            break;
        }
    }
    out
}

/// Every program the config may start: key and button actions, autostart,
/// status commands, the wallpaper.
fn programs(cfg: &Config) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // run the actions without side effects: only trwm.* commands are queued
    let _ = cfg.lua.load("os.execute = function() return true end; io.popen = nil; trwm.sh = function() return '' end").exec();
    let mut cmds: Vec<Cmd> = vec![];
    for f in cfg.keys.iter().map(|k| &k.func).chain(cfg.buttons.iter().map(|b| &b.func)) {
        if let Ok(c) = cfg.call(f, None) {
            cmds.extend(c);
        }
    }
    for c in cmds {
        match c {
            Cmd::Spawn(Spawn::Argv(a)) => {
                out.insert(a[0].clone());
                // a program started inside a terminal: kitty -e yazi
                if let Some(i) = a.iter().position(|x| x == "-e" || x == "--command" || x == "-x")
                    && let Some(p) = a.get(i + 1)
                {
                    out.insert(p.clone());
                }
            }
            Cmd::Spawn(Spawn::Shell(s)) => out.extend(programs_in(&s)),
            _ => {}
        }
    }
    for a in &cfg.autostart {
        out.extend(programs_in(a));
    }
    for c in &cfg.status.components {
        if let StatusSource::Command(s) = &c.source {
            out.extend(programs_in(s));
        }
    }
    if cfg.wallpaper.is_some() {
        out.insert("feh".into());
    }
    out.remove("trwm"); // checked separately
    out
}

/// Is a font family installed, or would fontconfig substitute another one?
fn font_installed(pattern: &str) -> Option<bool> {
    let family = pattern.split(':').next().unwrap_or("").trim();
    if family.is_empty() || matches!(family.to_ascii_lowercase().as_str(), "monospace" | "mono" | "sans" | "sans-serif" | "serif") {
        return Some(true);
    }
    let out = Command::new("fc-match").args(["-f", "%{family}", family]).output().ok()?;
    let got = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    Some(got.split(',').any(|f| f.trim() == family.to_ascii_lowercase()))
}

fn has_extension(dpy: *mut xlib::Display, name: &str) -> bool {
    let c = CString::new(name).unwrap();
    let (mut a, mut b, mut d) = (0, 0, 0);
    unsafe { xlib::XQueryExtension(dpy, c.as_ptr(), &mut a, &mut b, &mut d) != 0 }
}

/// The name of the running window manager, if any (_NET_SUPPORTING_WM_CHECK).
fn running_wm(dpy: *mut xlib::Display) -> Option<String> {
    unsafe {
        let root = xlib::XDefaultRootWindow(dpy);
        let check = CString::new("_NET_SUPPORTING_WM_CHECK").unwrap();
        let name = CString::new("_NET_WM_NAME").unwrap();
        let utf8 = CString::new("UTF8_STRING").unwrap();
        let (a_check, a_name, a_utf8) =
            (xlib::XInternAtom(dpy, check.as_ptr(), 0), xlib::XInternAtom(dpy, name.as_ptr(), 0), xlib::XInternAtom(dpy, utf8.as_ptr(), 0));
        let get = |w: xlib::Window, prop: xlib::Atom, kind: xlib::Atom| -> Option<Vec<u8>> {
            let (mut real, mut format, mut n, mut after) = (0, 0, 0, 0);
            let mut p: *mut u8 = ptr::null_mut();
            let r = xlib::XGetWindowProperty(dpy, w, prop, 0, 1024, 0, kind, &mut real, &mut format, &mut n, &mut after, &mut p);
            if r != 0 || p.is_null() {
                return None;
            }
            let bytes = match format {
                32 => std::slice::from_raw_parts(p as *const std::os::raw::c_ulong, n as usize).iter().flat_map(|v| v.to_ne_bytes()).collect(),
                _ => std::slice::from_raw_parts(p, n as usize).to_vec(),
            };
            xlib::XFree(p as *mut _);
            Some(bytes)
        };
        let w = get(root, a_check, xlib::XA_WINDOW)?;
        let win = std::os::raw::c_ulong::from_ne_bytes(w.get(..8)?.try_into().ok()?);
        let n = get(win, a_name, a_utf8)?;
        Some(String::from_utf8_lossy(&n).trim_end_matches('\0').to_string())
    }
}

// ---------------------------------------------------------------------------
// the checks
// ---------------------------------------------------------------------------

pub fn run(cfg_path: Option<&Path>) -> i32 {
    let mut r = Report { color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(), problems: 0, warnings: 0 };
    let d = distro();
    println!("trwm doctor (trwm {})", env!("CARGO_PKG_VERSION"));

    // --- config ---------------------------------------------------------
    r.section("Config");
    let path = cfg_path.map(Path::to_path_buf).unwrap_or_else(config::default_path);
    if path.exists() {
        r.ok(&format!("config file: {}", path.display()));
    } else if cfg_path.is_some() {
        r.fail(&format!("{} does not exist", path.display()), "");
    } else {
        r.warn(
            &format!("no config file yet ({})", path.display()),
            "trwm writes one the first time it starts, or create it now:\ntrwm default-config > ~/.config/trwm/config.lua",
        );
    }
    let cfg = match Config::load(cfg_path) {
        Ok(c) => {
            r.ok(&format!("config loads: {} keys, {} layouts, {} rules", c.keys.len(), c.layouts.len(), c.rules.len()));
            Some(c)
        }
        Err(e) => {
            r.fail("the config has an error", &format!("{e}\ntrwm still starts (with the built-in default) and shows this error in the bar"));
            Config::from_source(config::DEFAULT_CONFIG, "default config", None).ok()
        }
    };
    let Some(cfg) = cfg else { return 1 };
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(config::config_dir);
    let types_ok = std::fs::read_to_string(dir.join("types/trwm.lua")).ok().as_deref() == Some(config::TYPES);
    if types_ok && dir.join(".luarc.json").exists() {
        r.ok("editor autocomplete is set up (types/trwm.lua, .luarc.json)");
    } else {
        r.warn(
            "editor autocomplete files are missing or outdated",
            "trwm writes them when it starts, or run: trwm types\n(for editors with the Lua language server: Neovim, VS Code, ...)",
        );
    }
    if let Some(w) = &cfg.wallpaper {
        let p = config::expand_tilde(&w.path);
        if Path::new(&p).exists() {
            r.ok(&format!("wallpaper: {}", w.path));
        } else {
            r.warn(&format!("wallpaper {} does not exist", w.path), "change `wallpaper` in the config to an image (or a folder of images) you have");
        }
    }

    // --- programs -------------------------------------------------------
    r.section("Programs your config starts");
    let progs = programs(&cfg);
    let missing: Vec<String> = progs.iter().filter(|p| which(p).is_none()).cloned().collect();
    let found: Vec<&String> = progs.iter().filter(|p| !missing.contains(p)).collect();
    if !found.is_empty() {
        r.ok(&format!("found: {}", found.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if !missing.is_empty() {
        let mut pkgs: Vec<String> = missing.iter().filter(|p| !p.contains('/')).map(|p| package(p, d)).collect();
        pkgs.dedup();
        let files: Vec<&String> = missing.iter().filter(|p| p.contains('/')).collect();
        if !pkgs.is_empty() {
            r.fail(
                &format!("not installed: {}", missing.iter().filter(|p| !p.contains('/')).cloned().collect::<Vec<_>>().join(", ")),
                &format!("{}\n(or change the keys / autostart / status entries that use them)", install_cmd(&pkgs, d)),
            );
        }
        for f in files {
            r.warn(&format!("{f} does not exist"), "");
        }
    }
    if which("trwm").is_some() {
        r.ok("trwm is in your PATH (needed by `xss-lock -- trwm lock` and `trwm msg`)");
    } else {
        r.warn(
            "trwm is not in your PATH",
            "add its folder to PATH in ~/.bash_profile (for cargo install: . \"$HOME/.cargo/env\"),\nor install it to /usr/local/bin: sudo install -m755 target/release/trwm /usr/local/bin/",
        );
    }
    for (prog, what) in [("feh", "wallpapers"), ("xss-lock", "locking on suspend")] {
        if !progs.contains(prog) && which(prog).is_none() && (prog != "feh" || cfg.wallpaper.is_some()) {
            r.warn(&format!("{prog} is not installed (optional, for {what})"), &install_cmd(&[package(prog, d)], d));
        }
    }

    // --- fonts ------------------------------------------------------------
    r.section("Fonts");
    if which("fc-match").is_none() {
        r.warn("can't check fonts: fc-match is missing", &install_cmd(&["fontconfig".into()], d));
    } else {
        let mut fonts: Vec<String> = cfg.fonts.clone();
        fonts.push(cfg.lock.font.clone());
        fonts.push(cfg.lock.small_font.clone());
        fonts.dedup();
        for f in fonts {
            match font_installed(&f) {
                Some(true) => r.ok(&format!("font: {f}")),
                _ => r.warn(
                    &format!("font '{}' is not installed: another font is used instead", f.split(':').next().unwrap_or(&f)),
                    if f.to_ascii_lowercase().contains("nerd") {
                        "icons will show as boxes; install a Nerd Font: https://www.nerdfonts.com (or your distro's nerd-fonts package)"
                    } else {
                        "install it, or change `fonts` in the config"
                    },
                ),
            }
        }
    }

    // --- dark mode ----------------------------------------------------------
    if let Some(dark) = cfg.dark_mode {
        r.section("Dark mode");
        let want = if dark { "1" } else { "0" };
        let ok = ["gtk-3.0", "gtk-4.0"].iter().all(|v| {
            let p = crate::config::config_dir_of(v).join("settings.ini");
            let s = std::fs::read_to_string(p).unwrap_or_default();
            crate::appearance::set_ini_key(&s, "Settings", "gtk-application-prefer-dark-theme", want) == s
        });
        if ok {
            r.ok(&format!("GTK 3/4 prefer the {} look", if dark { "dark" } else { "light" }));
        } else {
            r.warn("~/.config/gtk-{3,4}.0/settings.ini don't match dark_mode yet", "trwm writes them when it starts or reloads the config");
        }
        match (crate::appearance::qt_theme(), crate::appearance::has_qt_gtk3()) {
            (Some(t), _) => r.ok(&format!("Qt apps use QT_QPA_PLATFORMTHEME={t}, set its colours there")),
            (None, Some(true)) | (None, None) => r.ok("Qt 6 apps follow GTK (QT_QPA_PLATFORMTHEME=gtk3)"),
            (None, Some(false)) => r.warn(
                "Qt 6's gtk3 platform theme isn't installed, so Qt apps stay light",
                "install it: qt6-base with USE=gtk (Gentoo), qt6-gtk-platformtheme (Debian/Ubuntu)",
            ),
        }
        if dark && crate::appearance::has_qt5() {
            match (crate::appearance::qt_style(), crate::appearance::qt5_dark_style()) {
                (Some(st), _) => r.ok(&format!("Qt 5 apps use QT_STYLE_OVERRIDE={st}")),
                (None, Some(st)) => r.ok(&format!("Qt 5 apps use the {st} style")),
                (None, None) => r.warn(
                    "Qt 5 has no dark mode of its own, so Qt 5 apps stay light",
                    "install adwaita-qt (or Kvantum) for Qt 5; trwm then uses its dark style",
                ),
            }
        }
        if dark && crate::appearance::has_gtk2() && !crate::appearance::has_gtk2_dark_theme() {
            r.warn(
                "GTK 2 apps stay light: the Adwaita-dark GTK 2 theme isn't installed",
                "install gnome-themes-extra",
            );
        }
        if !crate::appearance::has_gnome_schema() {
            r.ok("apps that ask the desktop portal (Electron, Qt 6) may stay light: for them, install");
            r.fix("gsettings-desktop-schemas and xdg-desktop-portal-gtk (optional)");
        }
    }

    // --- lock screen ------------------------------------------------------
    r.section("Lock screen");
    if Path::new("/etc/pam.d/trwm").exists() {
        r.ok("PAM service /etc/pam.d/trwm is set up");
    } else {
        let faillock = std::fs::read_to_string("/etc/pam.d/system-auth").map(|s| s.contains("faillock")).unwrap_or(false);
        let fix = "printf 'auth required pam_unix.so\\n' | sudo tee /etc/pam.d/trwm";
        if faillock {
            r.fail("/etc/pam.d/trwm is missing, and your system uses pam_faillock: 3 typos at the lock screen lock your account for 10 minutes", fix);
        } else {
            r.warn("/etc/pam.d/trwm is missing (the lock screen uses the \"login\" PAM service)", fix);
        }
    }

    // --- X server -----------------------------------------------------------
    r.section("X server");
    let dpy = unsafe { xlib::XOpenDisplay(ptr::null()) };
    if dpy.is_null() {
        r.warn(
            "not running inside X: skipping the X checks",
            "run `trwm doctor` again from a terminal inside your X session for the full check",
        );
    } else {
        match running_wm(dpy) {
            Some(n) if n == "trwm" => r.ok("trwm is running"),
            Some(n) => r.ok(&format!("window manager running now: {n} (start trwm instead of it to use trwm)")),
            None => r.ok("no window manager running"),
        }
        let needed = [("RANDR", "monitors, DPI"), ("XInputExtension", "mouse/touchpad settings"), ("XKEYBOARD", "keyboard layouts"), ("XINERAMA", "multi-monitor")];
        let optional = [("DPMS", "monitor power saving"), ("MIT-SCREEN-SAVER", "idle auto-lock")];
        for (ext, what) in needed {
            if has_extension(dpy, ext) {
                r.ok(&format!("{ext} ({what})"));
            } else {
                r.warn(&format!("the X server has no {ext} extension: {what} won't work"), "");
            }
        }
        for (ext, what) in optional {
            if !has_extension(dpy, ext) {
                r.warn(&format!("no {ext} extension: {what} won't work"), "");
            }
        }

        // the hardware sections of the config
        if cfg.keyboard.sets_layout() {
            match crate::keyboard::resolve(dpy, &cfg.keyboard) {
                Ok(k) => r.ok(format!("keyboard: {} {}", k.layout, k.variant).trim_end()),
                Err(e) => r.fail(&e, "fix `keyboard` in the config"),
            }
        }
        if !cfg.monitors.is_empty() {
            let screen = unsafe { xlib::XDefaultScreen(dpy) };
            let root = unsafe { xlib::XRootWindow(dpy, screen) };
            let outs = crate::outputs::query(dpy, root);
            for m in outs.as_deref().map(|o| crate::outputs::missing(o, &cfg.monitors)).unwrap_or_default() {
                r.warn(&format!("monitors: no output called {m} right now, skipping it"), "`trwm monitors` lists the names; output = { \"a\", \"b\" } accepts several");
            }
            match outs.map(|o| crate::outputs::plan(&o, &cfg.monitors)) {
                Some(Ok(_)) => r.ok("monitors: the arrangement is valid"),
                Some(Err(e)) => r.fail(&e, "fix `monitors` in the config (`trwm monitors` lists the names and modes)"),
                None => r.warn("can't check monitors: RandR unavailable", ""),
            }
        }
        if !cfg.input.mouse.is_empty() || !cfg.input.touchpad.is_empty() {
            let devs = crate::input::devices(dpy);
            let not_libinput: Vec<&str> = devs.iter().filter(|d| !d.libinput).map(|d| d.name.as_str()).collect();
            if not_libinput.is_empty() {
                r.ok(&format!("mouse/touchpad: {} device(s), all use libinput", devs.len()));
            } else {
                r.warn(
                    &format!("these devices don't use the libinput driver, so mouse/touchpad settings can't change them: {}", not_libinput.join(", ")),
                    &install_cmd(&[if d == Distro::Gentoo { "x11-drivers/xf86-input-libinput".into() } else { "xf86-input-libinput".into() }], d),
                );
            }
        }
        unsafe { xlib::XCloseDisplay(dpy) };
    }

    // --- starting trwm --------------------------------------------------------
    r.section("Starting trwm");
    let xinitrc = std::fs::read_to_string(format!("{}/.xinitrc", config::home())).unwrap_or_default();
    let session = Path::new("/usr/share/xsessions/trwm.desktop").exists();
    if xinitrc.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("trwm") && l.contains("exec")) {
        r.ok("~/.xinitrc starts trwm (startx)");
    } else if session {
        r.ok("display manager session: /usr/share/xsessions/trwm.desktop");
    } else {
        r.warn(
            "nothing starts trwm yet",
            "for startx: put `exec trwm` as the last line of ~/.xinitrc\nfor a display manager (LightDM, SDDM, ly, ...): sudo cp extra/trwm.desktop /usr/share/xsessions/",
        );
    }

    println!();
    match (r.problems, r.warnings) {
        (0, 0) => println!("Everything looks good."),
        (0, w) => println!("No problems, {w} warning(s)."),
        (p, w) => println!("{p} problem(s), {w} warning(s): see the fixes above."),
    }
    if r.problems > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::programs_in;

    #[test]
    fn finds_programs_in_shell_commands() {
        assert_eq!(programs_in("kitty"), vec!["kitty"]);
        assert_eq!(programs_in("wpctl set-volume @DEFAULT@ 5%+; trwm msg refresh"), vec!["wpctl", "trwm"]);
        assert_eq!(
            programs_in("mkdir -p ~/x && f=~/x/$(date +%F).png && maim \"$f\" && xclip -selection clipboard -i \"$f\""),
            vec!["mkdir", "date", "maim", "xclip"]
        );
        assert_eq!(programs_in("FOO=1 env BAR=2 picom -b"), vec!["picom"]);
        assert_eq!(programs_in("brightnessctl -m | cut -d, -f4"), vec!["brightnessctl", "cut"]);
        assert_eq!(programs_in("sleep 2 && nm-applet &"), vec!["sleep", "nm-applet"]);
        // separators inside quotes are not commands
        assert_eq!(
            programs_in(r#"wpctl get-volume @X@ | awk '/MUTED/{print " muted";exit} {printf " %d%%", $2*100}'"#),
            vec!["wpctl", "awk"]
        );
        assert_eq!(programs_in("supergfxctl -g | sed 's/A/dGPU/;s/Integrated/iGPU/'"), vec!["supergfxctl", "sed"]);
        assert_eq!(programs_in(r#"notify-send "$(date)""#), vec!["notify-send", "date"]);
    }
}
