//! `dark_mode = true`: tell programs to use their dark look.
//!
//! There is no single switch for this outside a desktop environment, so trwm
//! sets the usual ones:
//! - GTK 3 and 4 (and Firefox/Chromium, which follow GTK): the
//!   `gtk-application-prefer-dark-theme` key in ~/.config/gtk-{3,4}.0/settings.ini
//! - libadwaita apps without a portal backend: `ADW_DEBUG_COLOR_SCHEME` in
//!   the environment of programs trwm starts (see [`env`])
//! - the desktop portal (Firefox, Chromium, Electron, Qt, libadwaita), when
//!   the GNOME schema is installed: `gsettings ... color-scheme`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KEY: &str = "gtk-application-prefer-dark-theme";

/// Apply `dark_mode` (None = leave everything alone).
pub fn apply(dark: Option<bool>) {
    let Some(dark) = dark else { return };
    for v in ["gtk-3.0", "gtk-4.0"] {
        let p = crate::config::config_dir_of(v).join("settings.ini");
        if let Err(e) = write_ini(&p, dark) {
            eprintln!("trwm: dark_mode: {}: {e}", p.display());
        }
    }
    if has_gnome_schema() {
        let scheme = if dark { "prefer-dark" } else { "default" };
        // in a thread so a slow D-Bus never blocks the window manager
        std::thread::spawn(move || {
            let _ = Command::new("gsettings")
                .args(["set", "org.gnome.desktop.interface", "color-scheme", scheme])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        });
    }
}

/// Environment for programs trwm starts.
pub fn env(dark: Option<bool>) -> Option<(&'static str, &'static str)> {
    match dark? {
        true => Some(("ADW_DEBUG_COLOR_SCHEME", "prefer-dark")),
        false => Some(("ADW_DEBUG_COLOR_SCHEME", "prefer-light")),
    }
}

/// Is the `org.gnome.desktop.interface` schema installed? (Without it
/// gsettings can't store the colour scheme, and portals can't report it.)
pub fn has_gnome_schema() -> bool {
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.split(':')
        .filter(|d| !d.is_empty())
        .any(|d| Path::new(d).join("glib-2.0/schemas/org.gnome.desktop.interface.gschema.xml").exists())
}

fn write_ini(p: &PathBuf, dark: bool) -> std::io::Result<()> {
    let old = std::fs::read_to_string(p).unwrap_or_default();
    let new = set_ini_key(&old, "Settings", KEY, if dark { "1" } else { "0" });
    if new != old {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, new)?;
    }
    Ok(())
}

/// Set `key = value` in `[section]` of an ini file, keeping everything else.
pub fn set_ini_key(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let line = format!("{key}={value}");
    let mut out: Vec<String> = vec![];
    let (mut in_sec, mut seen_sec, mut done) = (false, false, false);
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_sec && !done {
                let at = out.len() - out.iter().rev().take_while(|l| l.trim().is_empty()).count();
                out.insert(at, line.clone());
                done = true;
            }
            in_sec = t == header;
            seen_sec |= in_sec;
        } else if in_sec && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !seen_sec {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(header);
        }
        out.push(line);
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini() {
        let k = "gtk-application-prefer-dark-theme";
        assert_eq!(set_ini_key("", "Settings", k, "1"), "[Settings]\ngtk-application-prefer-dark-theme=1\n");
        // replaces the key, keeps the rest
        let t = "[Settings]\ngtk-theme-name=Arc\ngtk-application-prefer-dark-theme = 0\n";
        assert_eq!(set_ini_key(t, "Settings", k, "1"), "[Settings]\ngtk-theme-name=Arc\ngtk-application-prefer-dark-theme=1\n");
        // adds it to the end of the right section
        let t = "[Settings]\ngtk-theme-name=Arc\n\n[Other]\nx=1\n";
        assert_eq!(set_ini_key(t, "Settings", k, "1"), "[Settings]\ngtk-theme-name=Arc\ngtk-application-prefer-dark-theme=1\n\n[Other]\nx=1\n");
        // adds the section
        assert_eq!(set_ini_key("[Other]\nx=1\n", "Settings", k, "1"), "[Other]\nx=1\n\n[Settings]\ngtk-application-prefer-dark-theme=1\n");
        // idempotent
        let once = set_ini_key("[Settings]\na=b\n", "Settings", k, "1");
        assert_eq!(set_ini_key(&once, "Settings", k, "1"), once);
    }
}
