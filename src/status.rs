//! Built-in status bar text (the trwm take on slstatus).
//!
//! Each component in `status = { ... }` is either a builtin (read straight
//! from /proc and /sys, no subprocesses), `run_command` (run in a background
//! thread so a slow command can never freeze the window manager), or a Lua
//! function.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::config::{Config, StatusSource};

pub const UNKNOWN: &str = "n/a";

pub const BUILTINS: &[&str] = &[
    "datetime",
    "cpu_perc",
    "cpu_freq",
    "ram_perc",
    "ram_used",
    "ram_free",
    "ram_total",
    "swap_perc",
    "swap_used",
    "battery_perc",
    "battery_state",
    "battery_remaining",
    "temp",
    "uptime",
    "load_avg",
    "disk_perc",
    "disk_free",
    "disk_used",
    "disk_total",
    "netspeed_rx",
    "netspeed_tx",
    "ipv4",
    "wifi_perc",
    "kernel_release",
    "hostname",
    "username",
    "num_files",
    "entropy",
    "keymap",
];

// state for the components that compute deltas between calls
/// the active keyboard layout, kept up to date by the window manager
pub static KEYMAP: Mutex<String> = Mutex::new(String::new());

static CPU: Mutex<Option<(u64, u64)>> = Mutex::new(None);
static NET: Mutex<Option<HashMap<String, (u64, Instant)>>> = Mutex::new(None);

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn human(bytes: f64, base: f64) -> String {
    let units = if base == 1024.0 { ["B", "Ki", "Mi", "Gi", "Ti", "Pi"] } else { ["B", "k", "M", "G", "T", "P"] };
    let mut v = bytes;
    let mut i = 0;
    while v >= base && i < units.len() - 1 {
        v /= base;
        i += 1;
    }
    format!("{v:.1} {}", units[i])
}

fn meminfo() -> HashMap<String, u64> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        for line in s.lines() {
            let mut it = line.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next())
                && let Ok(v) = v.parse::<u64>() {
                    m.insert(k.trim_end_matches(':').to_string(), v * 1024);
                }
        }
    }
    m
}

fn statvfs(path: &str) -> Option<libc::statvfs> {
    let c = CString::new(path).ok()?;
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(c.as_ptr(), &mut s) == 0).then_some(s)
    }
}

fn datetime(fmt: &str) -> Option<String> {
    let c = CString::new(fmt).ok()?;
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0 as libc::c_char; 256];
        let n = libc::strftime(buf.as_mut_ptr(), buf.len(), c.as_ptr(), &tm);
        (n > 0).then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

fn cpu_perc() -> Option<String> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 7 {
        return None;
    }
    // user nice system idle iowait irq softirq
    let total: u64 = v[..7].iter().sum();
    let idle = v[3] + v[4];
    let mut st = CPU.lock().ok()?;
    let prev = st.replace((total, idle));
    let (pt, pi) = prev?;
    let dt = total.saturating_sub(pt);
    if dt == 0 {
        return None;
    }
    let busy = dt.saturating_sub(idle.saturating_sub(pi));
    Some(format!("{}", (100 * busy / dt)))
}

fn net_bytes(iface: &str, dir: &str) -> Option<String> {
    let bytes: u64 = read(&format!("/sys/class/net/{iface}/statistics/{dir}_bytes"))?.parse().ok()?;
    let now = Instant::now();
    let mut g = NET.lock().ok()?;
    let map = g.get_or_insert_with(HashMap::new);
    let key = format!("{iface}/{dir}");
    let prev = map.insert(key, (bytes, now));
    let (pb, pt) = prev?;
    let secs = now.duration_since(pt).as_secs_f64().max(0.001);
    Some(format!("{}/s", human(bytes.saturating_sub(pb) as f64 / secs, 1024.0)))
}

fn ipv4(iface: &str) -> Option<String> {
    unsafe {
        let mut ifa: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifa) != 0 {
            return None;
        }
        let mut cur = ifa;
        let mut out = None;
        while !cur.is_null() {
            let a = &*cur;
            if !a.ifa_addr.is_null()
                && (*a.ifa_addr).sa_family as i32 == libc::AF_INET
                && CStr::from_ptr(a.ifa_name).to_bytes() == iface.as_bytes()
            {
                let sin = &*(a.ifa_addr as *const libc::sockaddr_in);
                let ip = u32::from_be(sin.sin_addr.s_addr);
                out = Some(std::net::Ipv4Addr::from(ip).to_string());
                break;
            }
            cur = a.ifa_next;
        }
        libc::freeifaddrs(ifa);
        out
    }
}

fn uname_field(f: impl Fn(&libc::utsname) -> &[libc::c_char; 65]) -> Option<String> {
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return None;
        }
        Some(CStr::from_ptr(f(&u).as_ptr()).to_string_lossy().into_owned())
    }
}

/// Evaluate a builtin. Never fails: errors become "n/a".
pub fn builtin(name: &str, arg: Option<&str>) -> String {
    let a = arg.unwrap_or("");
    let r: Option<String> = match name {
        "datetime" => datetime(if a.is_empty() { "%F %T" } else { a }),
        "cpu_perc" => cpu_perc(),
        "cpu_freq" => read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
            .and_then(|s| s.parse::<f64>().ok())
            .map(|khz| human(khz * 1000.0, 1000.0) + "Hz"),
        "ram_perc" | "ram_used" | "ram_free" | "ram_total" => {
            let m = meminfo();
            let total = *m.get("MemTotal").unwrap_or(&0);
            let avail = *m.get("MemAvailable").unwrap_or(&0);
            (total > 0).then(|| match name {
                "ram_perc" => format!("{}", 100 * (total - avail) / total),
                "ram_used" => human((total - avail) as f64, 1024.0),
                "ram_free" => human(avail as f64, 1024.0),
                _ => human(total as f64, 1024.0),
            })
        }
        "swap_perc" | "swap_used" => {
            let m = meminfo();
            let total = *m.get("SwapTotal").unwrap_or(&0);
            let free = *m.get("SwapFree").unwrap_or(&0);
            match name {
                "swap_perc" => (total > 0).then(|| format!("{}", 100 * (total - free) / total)),
                _ => Some(human((total - free) as f64, 1024.0)),
            }
        }
        "battery_perc" => read(&format!("/sys/class/power_supply/{}/capacity", or(a, "BAT0"))),
        "battery_state" => read(&format!("/sys/class/power_supply/{}/status", or(a, "BAT0"))).map(|s| {
            match s.as_str() {
                "Charging" => "+",
                "Discharging" => "-",
                "Full" | "Not charging" => "o",
                _ => "?",
            }
            .to_string()
        }),
        "battery_remaining" => {
            let b = or(a, "BAT0");
            let rd = |f: &str| read(&format!("/sys/class/power_supply/{b}/{f}")).and_then(|s| s.parse::<f64>().ok());
            let status = read(&format!("/sys/class/power_supply/{b}/status")).unwrap_or_default();
            let charge = rd("charge_now").or_else(|| rd("energy_now"));
            let rate = rd("current_now").or_else(|| rd("power_now"));
            match (status.as_str(), charge, rate) {
                ("Discharging", Some(c), Some(r)) if r > 0.0 => {
                    let h = c / r;
                    Some(format!("{}h {}m", h as u64, ((h.fract()) * 60.0) as u64))
                }
                _ => None,
            }
        }
        "temp" => read(or(a, "/sys/class/thermal/thermal_zone0/temp"))
            .and_then(|s| s.parse::<i64>().ok())
            .map(|t| format!("{}", t / 1000)),
        "uptime" => read("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).map(|up| {
            let up = up as u64;
            let (d, h, m) = (up / 86400, (up % 86400) / 3600, (up % 3600) / 60);
            if d > 0 { format!("{d}d {h}h {m}m") } else if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
        }),
        "load_avg" => read("/proc/loadavg").map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" ")),
        "disk_perc" | "disk_free" | "disk_used" | "disk_total" => statvfs(or(a, "/")).map(|s| {
            let bs = s.f_frsize as f64;
            let total = s.f_blocks as f64 * bs;
            let free = s.f_bavail as f64 * bs;
            let used = (s.f_blocks - s.f_bfree) as f64 * bs;
            match name {
                "disk_perc" => format!("{}", (100.0 * used / (used + free).max(1.0)) as u64),
                "disk_free" => human(free, 1024.0),
                "disk_used" => human(used, 1024.0),
                _ => human(total, 1024.0),
            }
        }),
        "netspeed_rx" => net_bytes(or(a, "wlan0"), "rx"),
        "netspeed_tx" => net_bytes(or(a, "wlan0"), "tx"),
        "ipv4" => ipv4(or(a, "wlan0")),
        "wifi_perc" => std::fs::read_to_string("/proc/net/wireless").ok().and_then(|s| {
            let iface = or(a, "wlan0");
            s.lines().find(|l| l.trim_start().starts_with(&format!("{iface}:"))).and_then(|l| {
                let q: f64 = l.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
                Some(format!("{}", (q / 70.0 * 100.0).min(100.0) as u64))
            })
        }),
        "kernel_release" => uname_field(|u| &u.release),
        "hostname" => uname_field(|u| &u.nodename),
        "username" => std::env::var("USER").ok(),
        "num_files" => std::fs::read_dir(crate::config::expand_tilde(a)).ok().map(|d| d.count().to_string()),
        "entropy" => read("/proc/sys/kernel/random/entropy_avail"),
        "keymap" => {
            let k = KEYMAP.lock().ok().map(|k| k.clone()).filter(|k| !k.is_empty());
            k.or_else(|| unsafe {
                // not running inside trwm (e.g. `trwm status`): ask the X server
                let d = x11::xlib::XOpenDisplay(std::ptr::null());
                if d.is_null() {
                    return None;
                }
                let n = crate::keyboard::active_name(d);
                x11::xlib::XCloseDisplay(d);
                n
            })
        }
        _ => None,
    };
    r.unwrap_or_else(|| UNKNOWN.to_string())
}

fn or<'a>(a: &'a str, d: &'a str) -> &'a str {
    if a.is_empty() { d } else { a }
}

/// slstatus-like format: "%s" is replaced by the value, "%%" by "%".
pub fn format(fmt: &str, val: &str) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    let mut used = false;
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                Some('s') if !used => {
                    chars.next();
                    out.push_str(val);
                    used = true;
                }
                _ => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the running status line
// ---------------------------------------------------------------------------

struct Slot {
    value: String,
    next: Instant,
    pending: bool,
}

pub struct Status {
    slots: Vec<Slot>,
    generation: u64,
    tx: Sender<(u64, usize, String)>,
    rx: Receiver<(u64, usize, String)>,
    /// background commands write a byte here so the event loop wakes up
    wake: [libc::c_int; 2],
}

impl Status {
    pub fn new() -> Status {
        let (tx, rx) = channel();
        let mut wake = [-1; 2];
        unsafe { libc::pipe2(wake.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        Status { slots: vec![], generation: 0, tx, rx, wake }
    }

    pub fn wake_fd(&self) -> libc::c_int {
        self.wake[0]
    }

    /// Collect finished `run_command` results; returns true if anything changed.
    pub fn drain(&mut self) -> bool {
        let mut buf = [0u8; 64];
        while unsafe { libc::read(self.wake[0], buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {}
        let mut changed = false;
        while let Ok((g, i, out)) = self.rx.try_recv() {
            if g == self.generation
                && let Some(s) = self.slots.get_mut(i)
            {
                s.pending = false;
                if s.value != out {
                    s.value = out;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Call after (re)loading the config.
    pub fn reset(&mut self, cfg: &Config) {
        self.generation += 1;
        let now = Instant::now();
        self.slots = cfg.status.components.iter().map(|_| Slot { value: String::new(), next: now, pending: false }).collect();
    }

    /// Force every component to update on the next tick.
    pub fn refresh(&mut self) {
        let now = Instant::now();
        for s in &mut self.slots {
            s.next = now;
        }
    }

    /// Update due components; returns true if the text may have changed.
    pub fn tick(&mut self, cfg: &Config) -> bool {
        let mut changed = self.drain();
        let now = Instant::now();
        for (i, comp) in cfg.status.components.iter().enumerate() {
            let Some(slot) = self.slots.get_mut(i) else { continue };
            if slot.next > now {
                continue;
            }
            slot.next = now + Duration::from_secs(comp.interval);
            let val = match &comp.source {
                StatusSource::Builtin(n, a) => builtin(n, a.as_deref()),
                StatusSource::Lua(f) => match f.call::<Option<String>>(()) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        eprintln!("trwm: status function: {e}");
                        UNKNOWN.to_string()
                    }
                },
                StatusSource::Command(cmd) => {
                    if !slot.pending {
                        slot.pending = true;
                        let (tx, g, cmd, wfd) = (self.tx.clone(), self.generation, cmd.clone(), self.wake[1]);
                        std::thread::spawn(move || {
                            let out = std::process::Command::new("/bin/sh")
                                .arg("-c")
                                .arg(&cmd)
                                .stdin(std::process::Stdio::null())
                                .output()
                                .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string())
                                .unwrap_or_else(|_| UNKNOWN.to_string());
                            let _ = tx.send((g, i, out));
                            unsafe { libc::write(wfd, b"x".as_ptr() as *const _, 1) };
                        });
                    }
                    continue;
                }
            };
            if slot.value != val {
                slot.value = val;
                changed = true;
            }
        }
        changed
    }

    pub fn text(&self, cfg: &Config) -> String {
        cfg.status
            .components
            .iter()
            .zip(&self.slots)
            .map(|(c, s)| format(&c.fmt, if s.value.is_empty() && s.pending { "…" } else { &s.value }))
            .collect()
    }

    /// Block until all components (including commands) have produced a value.
    pub fn wait_all(&mut self, cfg: &Config) {
        self.tick(cfg);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.slots.iter().any(|s| s.pending) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            self.drain();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_slstatus() {
        assert_eq!(format("CPU %s%%", "42"), "CPU 42%");
        assert_eq!(format("%s %s", "a"), "a %s");
        assert_eq!(format("100%", "x"), "100%");
    }

    #[test]
    fn builtins_never_panic() {
        for b in BUILTINS {
            let _ = builtin(b, None);
        }
        assert_eq!(builtin("datetime", Some("%%")), "%");
        assert_eq!(builtin("battery_perc", Some("NO_SUCH_BAT")), UNKNOWN);
    }
}
