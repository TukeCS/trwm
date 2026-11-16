//! `dark_mode = true`: tell programs to use their dark look.
//!
//! There is no single switch for this outside a desktop environment, so trwm
//! sets the usual ones:
//! - GTK 3 and 4 (and Firefox/Chromium, which follow GTK): the
//!   `gtk-application-prefer-dark-theme` key in ~/.config/gtk-{3,4}.0/settings.ini
//! - libadwaita apps without a portal backend: `ADW_DEBUG_COLOR_SCHEME` in
//!   the environment of programs trwm starts (see [`env`])
//! - Qt 6.5+: `QT_QPA_PLATFORMTHEME=gtk3` in the environment of programs trwm
//!   starts, so Qt takes its palette (dark or light) from the GTK settings
//!   above. Left alone when the user already picked a theme (qt6ct, kde...).
//! - Qt 5, which has no dark palette of its own: `QT_STYLE_OVERRIDE` set to a
//!   dark style when one is installed (adwaita-qt's `Adwaita-Dark`, or
//!   `kvantum-dark`). Qt 6 ignores a style it doesn't have, so it stays on gtk3.
//! - GTK 2: `gtk-theme-name="Adwaita-dark"` in ~/.gtkrc-2.0, when that theme
//!   is installed and the user hasn't picked another one
//! - the desktop portal (Firefox, Chromium, Electron, Qt, libadwaita), when
//!   the GNOME schema is installed: `gsettings ... color-scheme`
//! - D-Bus and systemd started programs: the environment above is handed to
//!   `dbus-update-activation-environment` too

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
    if let Err(e) = write_gtkrc(dark) {
        eprintln!("trwm: dark_mode: ~/.gtkrc-2.0: {e}");
    }
    let gnome = has_gnome_schema();
    let vars: Vec<String> = env(Some(dark)).iter().map(|(k, v)| format!("{k}={v}")).collect();
    // in a thread so a slow D-Bus never blocks the window manager
    std::thread::spawn(move || {
        let run = |c: &mut Command| {
            let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        };
        if gnome {
            let scheme = if dark { "prefer-dark" } else { "default" };
            run(Command::new("gsettings").args(["set", "org.gnome.desktop.interface", "color-scheme", scheme]));
        }
        run(Command::new("dbus-update-activation-environment").arg("--systemd").args(&vars));
    });
}

/// Environment for programs trwm starts.
pub fn env(dark: Option<bool>) -> Vec<(&'static str, &'static str)> {
    let Some(dark) = dark else { return vec![] };
    let mut v = vec![("ADW_DEBUG_COLOR_SCHEME", if dark { "prefer-dark" } else { "prefer-light" })];
    if qt_theme().is_none() {
        v.push(("QT_QPA_PLATFORMTHEME", "gtk3"));
    }
    if dark && qt_style().is_none()
        && let Some(style) = qt5_dark_style()
    {
        v.push(("QT_STYLE_OVERRIDE", style));
    }
    v
}

/// The Qt style the user set themselves, if any.
pub fn qt_style() -> Option<String> {
    std::env::var("QT_STYLE_OVERRIDE").ok().filter(|t| !t.is_empty())
}

/// Qt `ver`'s plugin directories that exist on this system.
fn qt_plugin_dirs(ver: u8) -> Vec<PathBuf> {
    let mut dirs = vec![
        format!("/usr/lib/qt{ver}/plugins"),
        format!("/usr/lib64/qt{ver}/plugins"),
        format!("/usr/lib/x86_64-linux-gnu/qt{ver}/plugins"),
        format!("/usr/lib/aarch64-linux-gnu/qt{ver}/plugins"),
    ];
    if ver == 5 {
        dirs.push("/usr/lib/qt/plugins".into()); // Arch
    }
    dirs.into_iter().map(PathBuf::from).filter(|d| d.is_dir()).collect()
}

/// Is Qt 5 installed?
pub fn has_qt5() -> bool {
    !qt_plugin_dirs(5).is_empty()
}

/// A dark style installed for Qt 5: adwaita-qt, else Kvantum.
pub fn qt5_dark_style() -> Option<&'static str> {
    let names: Vec<String> = qt_plugin_dirs(5)
        .iter()
        .filter_map(|d| std::fs::read_dir(d.join("styles")).ok())
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect();
    if names.iter().any(|n| n.contains("adwaita")) {
        Some("Adwaita-Dark")
    } else if names.iter().any(|n| n.contains("kvantum")) {
        Some("kvantum-dark")
    } else {
        None
    }
}

/// The Qt platform theme the user set themselves, if any.
pub fn qt_theme() -> Option<String> {
    std::env::var("QT_QPA_PLATFORMTHEME").ok().filter(|t| !t.is_empty())
}

/// Is Qt 6's gtk3 platform theme plugin installed? (None = no Qt 6 found.)
pub fn has_qt_gtk3() -> Option<bool> {
    let qt6 = qt_plugin_dirs(6);
    if qt6.is_empty() {
        return None;
    }
    Some(qt6.iter().any(|d| d.join("platformthemes/libqgtk3.so").exists()))
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(crate::config::home()).join(".local/share"));
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(home).chain(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect()
}

/// Is the GTK 2 `Adwaita-dark` theme installed (gnome-themes-extra)?
pub fn has_gtk2_dark_theme() -> bool {
    let home = PathBuf::from(crate::config::home()).join(".themes");
    std::iter::once(home)
        .chain(data_dirs().into_iter().map(|d| d.join("themes")))
        .any(|d| d.join("Adwaita-dark/gtk-2.0").is_dir())
}

/// Is GTK 2 installed?
pub fn has_gtk2() -> bool {
    ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
        .iter()
        .any(|d| Path::new(d).join("libgtk-x11-2.0.so.0").exists())
}

pub fn gtkrc_path() -> PathBuf {
    match std::env::var("GTK2_RC_FILES") {
        Ok(f) if !f.is_empty() => PathBuf::from(f.split(':').next().unwrap_or(&f)),
        _ => PathBuf::from(crate::config::home()).join(".gtkrc-2.0"),
    }
}

fn write_gtkrc(dark: bool) -> std::io::Result<()> {
    if dark && !has_gtk2_dark_theme() {
        return Ok(());
    }
    let p = gtkrc_path();
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    if let Some(new) = set_gtkrc_theme(&old, dark) {
        std::fs::write(&p, new)?;
    }
    Ok(())
}

/// Switch the GTK 2 theme between Adwaita and Adwaita-dark. Another theme the
/// user picked is kept. None = nothing to change.
pub fn set_gtkrc_theme(text: &str, dark: bool) -> Option<String> {
    let is_key = |l: &str| l.trim().split_once('=').is_some_and(|(k, _)| k.trim() == "gtk-theme-name");
    let cur = text.lines().find(|l| is_key(l)).map(|l| {
        let v = l.split_once('=').unwrap().1.trim();
        v.trim_matches('"').to_string()
    });
    let want = match (cur.as_deref(), dark) {
        (None | Some("Adwaita"), true) => "Adwaita-dark",
        (Some("Adwaita-dark"), false) => "Adwaita",
        _ => return None,
    };
    let line = format!("gtk-theme-name=\"{want}\"");
    let mut out: Vec<String> = text.lines().map(|l| if is_key(l) { line.clone() } else { l.to_string() }).collect();
    if cur.is_none() {
        out.push(line);
    }
    Some(out.join("\n") + "\n")
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

    #[test]
    fn gtkrc() {
        assert_eq!(set_gtkrc_theme("", true).as_deref(), Some("gtk-theme-name=\"Adwaita-dark\"\n"));
        let t = "gtk-font-name=\"Sans 10\"\ngtk-theme-name = \"Adwaita\"\n";
        assert_eq!(
            set_gtkrc_theme(t, true).as_deref(),
            Some("gtk-font-name=\"Sans 10\"\ngtk-theme-name=\"Adwaita-dark\"\n")
        );
        assert_eq!(set_gtkrc_theme("gtk-theme-name=\"Adwaita-dark\"\n", true), None);
        assert_eq!(set_gtkrc_theme("gtk-theme-name=\"Adwaita-dark\"\n", false).as_deref(), Some("gtk-theme-name=\"Adwaita\"\n"));
        // the user's own theme is kept
        assert_eq!(set_gtkrc_theme("gtk-theme-name=\"Arc\"\n", true), None);
        assert_eq!(set_gtkrc_theme("", false), None);
    }
}
