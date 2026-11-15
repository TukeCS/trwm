//! Screen blanking and monitor power saving: what `xset s ...` and
//! `xset dpms ...` do.

use x11::{dpms, xlib};

/// `screen = { ... }` in the config. `None` = leave as it is, `Some(0)` = never.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenCfg {
    /// seconds of inactivity before the screen blanks (the X screensaver;
    /// this is also when `xss-lock` locks)
    pub blank: Option<u32>,
    /// seconds of inactivity before the monitor is switched off (DPMS)
    pub off: Option<u32>,
}

pub fn apply(dpy: *mut xlib::Display, cfg: &ScreenCfg) {
    unsafe {
        if let Some(b) = cfg.blank {
            let (mut timeout, mut interval, mut blanking, mut exposures) = (0, 0, 0, 0);
            xlib::XGetScreenSaver(dpy, &mut timeout, &mut interval, &mut blanking, &mut exposures);
            xlib::XSetScreenSaver(dpy, b.min(32767) as i32, interval, blanking, exposures);
        }
        if let Some(o) = cfg.off {
            let (mut eb, mut er) = (0, 0);
            if dpms::DPMSQueryExtension(dpy, &mut eb, &mut er) != 0 && dpms::DPMSCapable(dpy) != 0 {
                if o == 0 {
                    dpms::DPMSDisable(dpy);
                } else {
                    let t = o.min(65535) as u16;
                    dpms::DPMSSetTimeouts(dpy, t, t, t);
                    dpms::DPMSEnable(dpy);
                }
            }
        }
        xlib::XSync(dpy, xlib::False);
    }
}

/// (blank seconds, monitor-off seconds or None when DPMS is disabled)
pub fn current(dpy: *mut xlib::Display) -> (u32, Option<u32>) {
    unsafe {
        let (mut timeout, mut interval, mut blanking, mut exposures) = (0, 0, 0, 0);
        xlib::XGetScreenSaver(dpy, &mut timeout, &mut interval, &mut blanking, &mut exposures);
        let (mut eb, mut er) = (0, 0);
        let mut off = None;
        if dpms::DPMSQueryExtension(dpy, &mut eb, &mut er) != 0 && dpms::DPMSCapable(dpy) != 0 {
            let (mut level, mut enabled) = (0u16, 0u8);
            dpms::DPMSInfo(dpy, &mut level, &mut enabled);
            if enabled != 0 {
                let (mut st, mut su, mut o) = (0u16, 0u16, 0u16);
                dpms::DPMSGetTimeouts(dpy, &mut st, &mut su, &mut o);
                off = Some(o as u32);
            }
        }
        (timeout.max(0) as u32, off)
    }
}
