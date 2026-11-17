//! `trwm lock`: the built-in screen locker (the trwm take on slock).
//!
//! It runs as its own process (spawned by the window manager, or by
//! `xss-lock -- trwm lock`) so that a crash in the window manager can never
//! unlock your screen. Passwords are checked with PAM, so no setuid is needed.

use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::time::{Duration, Instant};

use x11::{dpms, xinerama, xlib};

use crate::config::LockCfg;
use crate::drw::{Color, Drw};
use crate::pam;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Init,
    Input,
    Failed,
}

struct Screen {
    root: xlib::Window,
    win: xlib::Window,
    big: Drw,
    small: Drw,
    heads: Vec<(i32, i32, u32, u32)>,
    colors: [Color; 4],
}

pub fn run(cfg: &LockCfg, dpi: Option<u32>) -> Result<(), String> {
    let user = pam::username().ok_or("cannot determine user name")?;
    // prefer a dedicated /etc/pam.d/trwm (see README) so failed attempts at
    // the lock screen don't count towards pam_faillock for your login
    let mut cfg = cfg.clone();
    if cfg.pam_service == "trwm" && !std::path::Path::new("/etc/pam.d/trwm").exists() {
        cfg.pam_service = "login".into();
    }
    let cfg = &cfg;
    unsafe {
        let dpy = xlib::XOpenDisplay(ptr::null());
        if dpy.is_null() {
            return Err("cannot open display".into());
        }
        let nscreens = xlib::XScreenCount(dpy);
        let mut screens = Vec::new();
        for s in 0..nscreens {
            screens.push(lock_screen(dpy, s, cfg, dpi)?);
        }

        // grab keyboard and pointer; retry for a second because the key that
        // launched us may still be held down (the WM's passive grab)
        let mut grabbed = false;
        for _ in 0..1000 {
            let root = screens[0].root;
            let cursor = invisible_cursor(dpy, root);
            let p = xlib::XGrabPointer(
                dpy,
                root,
                xlib::False,
                (xlib::ButtonPressMask | xlib::ButtonReleaseMask | xlib::PointerMotionMask) as c_uint,
                xlib::GrabModeAsync,
                xlib::GrabModeAsync,
                0,
                cursor,
                xlib::CurrentTime,
            );
            let k = xlib::XGrabKeyboard(dpy, root, xlib::True, xlib::GrabModeAsync, xlib::GrabModeAsync, xlib::CurrentTime);
            if p == xlib::GrabSuccess && k == xlib::GrabSuccess {
                grabbed = true;
                break;
            }
            if p == xlib::GrabSuccess {
                xlib::XUngrabPointer(dpy, xlib::CurrentTime);
            }
            if k == xlib::GrabSuccess {
                xlib::XUngrabKeyboard(dpy, xlib::CurrentTime);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if !grabbed {
            for s in &screens {
                xlib::XDestroyWindow(dpy, s.win);
            }
            xlib::XCloseDisplay(dpy);
            return Err("unable to grab keyboard/pointer (is another locker running?)".into());
        }

        // turn the monitor off after `monitor_off` seconds
        let mut saved_dpms = None;
        let (mut eb, mut er) = (0, 0);
        if cfg.monitor_off > 0 && dpms::DPMSQueryExtension(dpy, &mut eb, &mut er) != 0 && dpms::DPMSCapable(dpy) != 0 {
            let (mut st, mut su, mut off) = (0u16, 0u16, 0u16);
            let mut level = 0u16;
            let mut enabled: u8 = 0;
            dpms::DPMSGetTimeouts(dpy, &mut st, &mut su, &mut off);
            dpms::DPMSInfo(dpy, &mut level, &mut enabled);
            saved_dpms = Some((st, su, off, enabled != 0));
            dpms::DPMSEnable(dpy);
            let t = cfg.monitor_off;
            dpms::DPMSSetTimeouts(dpy, t, t, t);
        }

        let mut password: Vec<u8> = Vec::with_capacity(256);
        let mut state = State::Init;
        let mut failures = 0u32;
        // `lockout`: wrong passwords in a row, and until when input is ignored
        let mut in_a_row = 0u32;
        let mut locked_until: Option<Instant> = None;
        let wait = |until: Option<Instant>| until.map(|t| t.saturating_duration_since(Instant::now()).as_secs() + 1);
        for s in &screens {
            draw(dpy, s, cfg, state, failures, None);
        }

        let fd = xlib::XConnectionNumber(dpy);
        let mut running = true;
        while running {
            if xlib::XPending(dpy) == 0 {
                let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                libc::poll(&mut pfd, 1, 1000);
                if locked_until.is_some_and(|t| Instant::now() >= t) {
                    locked_until = None;
                    in_a_row = 0;
                }
                if cfg.clock || locked_until.is_some() {
                    for s in &screens {
                        draw(dpy, s, cfg, state, failures, wait(locked_until));
                    }
                }
                continue;
            }
            let mut ev: xlib::XEvent = std::mem::zeroed();
            xlib::XNextEvent(dpy, &mut ev);
            if ev.get_type() == xlib::KeyPress {
                let mut buf = [0 as c_char; 32];
                let mut ks: xlib::KeySym = 0;
                let n = xlib::XLookupString(&mut ev.key, buf.as_mut_ptr(), buf.len() as c_int, &mut ks, ptr::null_mut());
                let ks = ks as u32;
                // keypad Enter etc. map to their main counterparts
                let ks = match ks {
                    x11::keysym::XK_KP_Enter => x11::keysym::XK_Return,
                    _ => ks,
                };
                let is_function_key = (x11::keysym::XK_F1..=x11::keysym::XK_F35).contains(&ks)
                    || (x11::keysym::XK_KP_Space..=x11::keysym::XK_KP_Equal).contains(&ks)
                    || (x11::keysym::XK_Select..=x11::keysym::XK_Break).contains(&ks)
                    || (x11::keysym::XK_KP_F1..=x11::keysym::XK_KP_F4).contains(&ks)
                    || (0x11000000..=0x1100FFFF).contains(&ks);
                if is_function_key {
                    continue;
                }
                if locked_until.is_some_and(|t| Instant::now() >= t) {
                    locked_until = None;
                    in_a_row = 0;
                }
                if locked_until.is_some() {
                    // too many wrong passwords: ignore typing until the time is up
                    pam::wipe(std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len()));
                    continue;
                }
                let prev = state;
                match ks {
                    x11::keysym::XK_Return => {
                        if pam::authenticate(&cfg.pam_service, &user, &password) {
                            running = false;
                        } else {
                            failures += 1;
                            in_a_row += 1;
                            if cfg.lockout_attempts > 0 && in_a_row >= cfg.lockout_attempts {
                                locked_until = Some(Instant::now() + Duration::from_secs(cfg.lockout_secs));
                            }
                            xlib::XBell(dpy, 100);
                            state = State::Failed;
                        }
                        pam::wipe(&mut password);
                        password.clear();
                    }
                    x11::keysym::XK_Escape => {
                        pam::wipe(&mut password);
                        password.clear();
                    }
                    x11::keysym::XK_BackSpace => {
                        // drop a whole UTF-8 character
                        while let Some(b) = password.pop() {
                            if b & 0xC0 != 0x80 {
                                break;
                            }
                        }
                    }
                    _ => {
                        if n > 0 && !(buf[0] as u8).is_ascii_control() && password.len() + (n as usize) < 1024 {
                            password.extend(buf[..n as usize].iter().map(|&b| b as u8));
                        }
                    }
                }
                if running {
                    state = if !password.is_empty() {
                        State::Input
                    } else if state == State::Failed || (cfg.fail_on_clear && prev == State::Input) {
                        State::Failed
                    } else {
                        state
                    };
                    for s in &screens {
                        draw(dpy, s, cfg, state, failures, wait(locked_until));
                    }
                }
                pam::wipe(std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len()));
            } else {
                // something else happened (a new window popped up, ...): stay on top
                for s in &screens {
                    xlib::XRaiseWindow(dpy, s.win);
                    if ev.get_type() == xlib::Expose {
                        draw(dpy, s, cfg, state, failures, wait(locked_until));
                    }
                }
            }
        }

        if let Some((st, su, off, enabled)) = saved_dpms {
            dpms::DPMSSetTimeouts(dpy, st, su, off);
            if !enabled {
                dpms::DPMSDisable(dpy);
            }
        }
        xlib::XUngrabPointer(dpy, xlib::CurrentTime);
        xlib::XUngrabKeyboard(dpy, xlib::CurrentTime);
        for s in screens {
            xlib::XDestroyWindow(dpy, s.win);
            drop(s.big);
            drop(s.small);
        }
        xlib::XCloseDisplay(dpy);
    }
    Ok(())
}

unsafe fn invisible_cursor(dpy: *mut xlib::Display, root: xlib::Window) -> xlib::Cursor {
    unsafe {
        let data = [0 as c_char; 8];
        let pm = xlib::XCreateBitmapFromData(dpy, root, data.as_ptr(), 8, 8);
        let mut c: xlib::XColor = std::mem::zeroed();
        let cur = xlib::XCreatePixmapCursor(dpy, pm, pm, &mut c, &mut c, 0, 0);
        xlib::XFreePixmap(dpy, pm);
        cur
    }
}

unsafe fn lock_screen(dpy: *mut xlib::Display, screen: c_int, cfg: &LockCfg, dpi: Option<u32>) -> Result<Screen, String> {
    unsafe {
        let root = xlib::XRootWindow(dpy, screen);
        let w = xlib::XDisplayWidth(dpy, screen) as u32;
        let h = xlib::XDisplayHeight(dpy, screen) as u32;

        let mut big = Drw::new(dpy, screen, root, w, h);
        let mut small = Drw::new(dpy, screen, root, 1, 1);
        big.set_fonts(&crate::config::with_dpi(&[cfg.font.clone(), "monospace:size=48".into()], dpi));
        small.set_fonts(&crate::config::with_dpi(&[cfg.small_font.clone(), "monospace:size=14".into()], dpi));
        let colors = [big.color(&cfg.init), big.color(&cfg.input), big.color(&cfg.failed), big.color(&cfg.text)];

        let mut wa: xlib::XSetWindowAttributes = std::mem::zeroed();
        wa.override_redirect = xlib::True;
        wa.background_pixel = colors[0].pixel;
        wa.event_mask = xlib::ExposureMask | xlib::VisibilityChangeMask;
        let win = xlib::XCreateWindow(
            dpy,
            root,
            0,
            0,
            w,
            h,
            0,
            xlib::XDefaultDepth(dpy, screen),
            xlib::CopyFromParent as c_uint,
            xlib::XDefaultVisual(dpy, screen),
            xlib::CWOverrideRedirect | xlib::CWBackPixel | xlib::CWEventMask,
            &mut wa,
        );
        let cursor = invisible_cursor(dpy, root);
        xlib::XDefineCursor(dpy, win, cursor);
        xlib::XMapRaised(dpy, win);
        xlib::XSelectInput(dpy, root, xlib::SubstructureNotifyMask);

        let mut heads = Vec::new();
        if xinerama::XineramaIsActive(dpy) != 0 {
            let mut n = 0;
            let info = xinerama::XineramaQueryScreens(dpy, &mut n);
            for i in 0..n as usize {
                let s = &*info.add(i);
                heads.push((s.x_org as i32, s.y_org as i32, s.width as u32, s.height as u32));
            }
            xlib::XFree(info as *mut _);
        }
        if heads.is_empty() {
            heads.push((0, 0, w, h));
        }
        Ok(Screen { root, win, big, small, heads, colors })
    }
}

fn now(fmt: &str) -> String {
    crate::status::builtin("datetime", Some(fmt))
}

/// `wait`: seconds left of a `lockout`, if one is running
unsafe fn draw(dpy: *mut xlib::Display, s: &Screen, cfg: &LockCfg, state: State, failures: u32, wait: Option<u64>) {
    unsafe {
        let bg = s.colors[match state {
            State::Init => 0,
            State::Input => 1,
            State::Failed => 2,
        }];
        let fg = s.colors[3];
        let canvas = s.big.drawable;
        s.big.fill_on(canvas, 0, 0, s.big.w, s.big.h, &bg);

        // (text, font) lines, centred on every monitor
        let mut lines: Vec<(String, &Drw)> = Vec::new();
        if cfg.clock {
            lines.push((now("%H:%M"), &s.big));
            lines.push((now("%A, %d %B %Y"), &s.small));
        }
        if let Some(m) = &cfg.message {
            lines.push((m.clone(), &s.small));
        }
        if failures > 0 && cfg.clock {
            lines.push((format!("{failures} failed attempt{}", if failures == 1 { "" } else { "s" }), &s.small));
        }
        if let Some(w) = wait {
            lines.push((format!("too many attempts, try again in {}:{:02}", w / 60, w % 60), &s.small));
        }
        let lh = |d: &Drw| d.font_h() + d.font_h() / 3;
        let total: u32 = lines.iter().map(|(_, d)| lh(d)).sum();
        for &(hx, hy, hw, hh) in &s.heads {
            let mut y = hy + (hh as i32 - total as i32) / 2;
            for (text, d) in &lines {
                let tw = d.text_width(text);
                let x = hx + (hw as i32 - tw as i32) / 2;
                d.text_on(canvas, x, y, tw, lh(d), 0, text, &fg, &bg);
                y += lh(d) as i32;
            }
        }
        s.big.map(s.win, 0, 0, s.big.w, s.big.h);
        xlib::XFlush(dpy);
    }
}
