//! The window manager proper: dwm's model (monitors, tags, a client list and
//! a focus stack per monitor, tiling layouts) rebuilt in Rust.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use x11::xlib::{self, Atom, Window};
use x11::{xcursor, xinerama, xrandr, xss};

use crate::config::{self, Click, Cmd, Config, Gaps, LayoutKind, LayoutSel, Spawn};
use crate::drw::{Drw, Scheme};
use crate::status::Status;

// ---------------------------------------------------------------------------
// atoms
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Wm {
    Protocols,
    Delete,
    State,
    TakeFocus,
}
const WM_NAMES: &[&str] = &["WM_PROTOCOLS", "WM_DELETE_WINDOW", "WM_STATE", "WM_TAKE_FOCUS"];

#[derive(Clone, Copy)]
enum Net {
    Supported,
    WMName,
    WMState,
    WMCheck,
    WMFullscreen,
    ActiveWindow,
    WMWindowType,
    WMWindowTypeDialog,
    ClientList,
    CurrentDesktop,
    NumberOfDesktops,
    DesktopNames,
}
const NET_NAMES: &[&str] = &[
    "_NET_SUPPORTED",
    "_NET_WM_NAME",
    "_NET_WM_STATE",
    "_NET_SUPPORTING_WM_CHECK",
    "_NET_WM_STATE_FULLSCREEN",
    "_NET_ACTIVE_WINDOW",
    "_NET_WM_WINDOW_TYPE",
    "_NET_WM_WINDOW_TYPE_DIALOG",
    "_NET_CLIENT_LIST",
    "_NET_CURRENT_DESKTOP",
    "_NET_NUMBER_OF_DESKTOPS",
    "_NET_DESKTOP_NAMES",
];

pub const CMD_ATOM: &str = "_TRWM_COMMAND";
const TAGS_ATOM: &str = "_TRWM_CLIENT_TAGS";

// ---------------------------------------------------------------------------
// X error handling
// ---------------------------------------------------------------------------

static OTHER_WM: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn xerror_start(_: *mut xlib::Display, _: *mut xlib::XErrorEvent) -> c_int {
    OTHER_WM.store(true, Ordering::SeqCst);
    0
}

/// There's no way to check accesses to destroyed windows, thus those cases
/// are ignored (especially on UnmapNotify's). Everything else is logged.
unsafe extern "C" fn xerror(_: *mut xlib::Display, ee: *mut xlib::XErrorEvent) -> c_int {
    let e = &*ee;
    let (req, code) = (e.request_code, e.error_code);
    use x11::xlib::*;
    const X_SET_INPUT_FOCUS: u8 = 42;
    const X_POLY_TEXT8: u8 = 74;
    const X_POLY_FILL_RECTANGLE: u8 = 70;
    const X_POLY_SEGMENT: u8 = 66;
    const X_CONFIGURE_WINDOW: u8 = 12;
    const X_GRAB_BUTTON: u8 = 28;
    const X_GRAB_KEY: u8 = 33;
    const X_COPY_AREA: u8 = 62;
    if code == BadWindow
        || (req == X_SET_INPUT_FOCUS && code == BadMatch)
        || (req == X_POLY_TEXT8 && code == BadDrawable)
        || (req == X_POLY_FILL_RECTANGLE && code == BadDrawable)
        || (req == X_POLY_SEGMENT && code == BadDrawable)
        || (req == X_CONFIGURE_WINDOW && code == BadMatch)
        || (req == X_GRAB_BUTTON && code == BadAccess)
        || (req == X_GRAB_KEY && code == BadAccess)
        || (req == X_COPY_AREA && code == BadDrawable)
    {
        return 0;
    }
    eprintln!("trwm: X error: request code={req}, error code={code}");
    0
}

// ---------------------------------------------------------------------------
// data
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Client {
    name: String,
    mina: f32,
    maxa: f32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    oldx: i32,
    oldy: i32,
    oldw: i32,
    oldh: i32,
    basew: i32,
    baseh: i32,
    incw: i32,
    inch: i32,
    maxw: i32,
    maxh: i32,
    minw: i32,
    minh: i32,
    hintsvalid: bool,
    bw: i32,
    oldbw: i32,
    tags: u32,
    isfixed: bool,
    isfloating: bool,
    isurgent: bool,
    neverfocus: bool,
    oldstate: bool,
    isfullscreen: bool,
    mon: usize,
    win: Window,
}

impl Client {
    fn width(&self) -> i32 {
        self.w + 2 * self.bw
    }
    fn height(&self) -> i32 {
        self.h + 2 * self.bw
    }
}

struct Monitor {
    ltsymbol: String,
    mfact: f32,
    nmaster: i32,
    num: i32,
    by: i32,
    mx: i32,
    my: i32,
    mw: i32,
    mh: i32,
    wx: i32,
    wy: i32,
    ww: i32,
    wh: i32,
    gaps: Gaps,
    seltags: usize,
    sellt: usize,
    tagset: [u32; 2],
    showbar: bool,
    topbar: bool,
    /// attach order; index 0 is the master
    clients: Vec<Window>,
    /// focus order; index 0 is the most recently focused
    stack: Vec<Window>,
    sel: Option<Window>,
    lt: [usize; 2],
    barwin: Window,
    // where things were drawn in the bar, for click detection
    bar_tags: Vec<(i32, i32, usize)>,
    bar_lt_end: i32,
    bar_status_x: i32,
}

pub struct WindowManager {
    dpy: *mut xlib::Display,
    screen: c_int,
    sw: i32,
    sh: i32,
    root: Window,
    wmcheckwin: Window,
    bh: i32,
    lrpad: i32,
    numlockmask: c_uint,
    drw: Drw,
    norm: Scheme,
    sel: Scheme,
    cursors: [xlib::Cursor; 3],
    wmatom: Vec<Atom>,
    netatom: Vec<Atom>,
    cmd_atom: Atom,
    tags_atom: Atom,
    utf8: Atom,
    mons: Vec<Monitor>,
    selmon: usize,
    motion_mon: Option<usize>,
    clients: HashMap<Window, Client>,

    cfg: Config,
    cfg_path: Option<PathBuf>,
    cfg_mtime: Option<SystemTime>,
    status: Status,
    stext: String,
    error: Option<(String, Instant)>,
    children: Vec<Child>,
    lock_child: Option<Child>,
    idle_locked: bool,
    next_tick: Instant,
    wallpaper_next: Option<Instant>,
    running: bool,
    restart: bool,
    scanning: bool,
}

const WITHDRAWN_STATE: c_long = 0;
const NORMAL_STATE: c_long = 1;
const ICONIC_STATE: c_long = 3;
const BUTTON_MASK: c_long = xlib::ButtonPressMask | xlib::ButtonReleaseMask;
const MOUSE_MASK: c_long = BUTTON_MASK | xlib::PointerMotionMask;

// ---------------------------------------------------------------------------
// setup / teardown
// ---------------------------------------------------------------------------

impl WindowManager {
    pub fn new(cfg: Config, cfg_path: Option<PathBuf>) -> Result<WindowManager, String> {
        unsafe {
            let dpy = xlib::XOpenDisplay(ptr::null());
            if dpy.is_null() {
                return Err("cannot open display".into());
            }
            // check for another window manager
            xlib::XSetErrorHandler(Some(xerror_start));
            xlib::XSelectInput(dpy, xlib::XDefaultRootWindow(dpy), xlib::SubstructureRedirectMask);
            xlib::XSync(dpy, xlib::False);
            if OTHER_WM.load(Ordering::SeqCst) {
                return Err("another window manager is already running".into());
            }
            xlib::XSetErrorHandler(Some(xerror));
            xlib::XSync(dpy, xlib::False);

            let screen = xlib::XDefaultScreen(dpy);
            let sw = xlib::XDisplayWidth(dpy, screen);
            let sh = xlib::XDisplayHeight(dpy, screen);
            let root = xlib::XRootWindow(dpy, screen);
            let mut drw = Drw::new(dpy, screen, root, sw as u32, 64);
            if !drw.set_fonts(&config::with_dpi(&cfg.fonts, cfg.dpi)) && !drw.set_fonts(&["monospace:size=10".into()]) {
                return Err("no fonts could be loaded".into());
            }
            let fh = drw.font_h() as i32;
            let norm = drw.scheme(&cfg.norm.fg, &cfg.norm.bg, &cfg.norm.border);
            let sel = drw.scheme(&cfg.sel.fg, &cfg.sel.bg, &cfg.sel.border);

            let intern = |n: &str| {
                let c = CString::new(n).unwrap();
                xlib::XInternAtom(dpy, c.as_ptr(), xlib::False)
            };
            let wmatom: Vec<Atom> = WM_NAMES.iter().map(|n| intern(n)).collect();
            let netatom: Vec<Atom> = NET_NAMES.iter().map(|n| intern(n)).collect();
            let cursors = [
                xlib::XCreateFontCursor(dpy, 68),  // XC_left_ptr
                xlib::XCreateFontCursor(dpy, 120), // XC_sizing
                xlib::XCreateFontCursor(dpy, 52),  // XC_fleur
            ];

            let mut wm = WindowManager {
                dpy,
                screen,
                sw,
                sh,
                root,
                wmcheckwin: 0,
                bh: fh + 2,
                lrpad: fh,
                numlockmask: 0,
                drw,
                norm,
                sel,
                cursors,
                wmatom,
                netatom,
                cmd_atom: intern(CMD_ATOM),
                tags_atom: intern(TAGS_ATOM),
                utf8: intern("UTF8_STRING"),
                mons: Vec::new(),
                selmon: 0,
                motion_mon: None,
                clients: HashMap::new(),
                cfg_mtime: cfg.path.as_deref().and_then(config::newest_mtime),
                cfg,
                cfg_path,
                status: Status::new(),
                stext: String::new(),
                error: None,
                children: Vec::new(),
                lock_child: None,
                idle_locked: false,
                next_tick: Instant::now(),
                wallpaper_next: None,
                running: true,
                scanning: false,
                restart: false,
            };
            wm.drw.resize(sw as u32, wm.bh as u32);
            wm.setup();
            Ok(wm)
        }
    }

    fn wa(&self, a: Wm) -> Atom {
        self.wmatom[a as usize]
    }
    fn na(&self, a: Net) -> Atom {
        self.netatom[a as usize]
    }

    unsafe fn setup(&mut self) {
        let dpy = self.dpy;
        self.updategeom();

        // supporting window for NetWMCheck
        self.wmcheckwin = xlib::XCreateSimpleWindow(dpy, self.root, 0, 0, 1, 1, 0, 0, 0);
        let check = self.wmcheckwin;
        xlib::XChangeProperty(dpy, check, self.na(Net::WMCheck), xlib::XA_WINDOW, 32, xlib::PropModeReplace, &check as *const _ as *const c_uchar, 1);
        let name = b"trwm";
        xlib::XChangeProperty(dpy, check, self.na(Net::WMName), self.utf8, 8, xlib::PropModeReplace, name.as_ptr(), name.len() as c_int);
        xlib::XChangeProperty(dpy, self.root, self.na(Net::WMCheck), xlib::XA_WINDOW, 32, xlib::PropModeReplace, &check as *const _ as *const c_uchar, 1);
        xlib::XChangeProperty(
            dpy,
            self.root,
            self.na(Net::Supported),
            xlib::XA_ATOM,
            32,
            xlib::PropModeReplace,
            self.netatom.as_ptr() as *const c_uchar,
            self.netatom.len() as c_int,
        );
        xlib::XDeleteProperty(dpy, self.root, self.na(Net::ClientList));
        xlib::XDeleteProperty(dpy, self.root, self.cmd_atom);
        self.update_desktop_props();

        let mut wa: xlib::XSetWindowAttributes = std::mem::zeroed();
        wa.cursor = self.cursors[0];
        wa.event_mask = xlib::SubstructureRedirectMask
            | xlib::SubstructureNotifyMask
            | xlib::ButtonPressMask
            | xlib::PointerMotionMask
            | xlib::EnterWindowMask
            | xlib::LeaveWindowMask
            | xlib::StructureNotifyMask
            | xlib::PropertyChangeMask;
        xlib::XChangeWindowAttributes(dpy, self.root, xlib::CWEventMask | xlib::CWCursor, &mut wa);
        xlib::XSelectInput(dpy, self.root, wa.event_mask);
        self.grabkeys();
        self.focus(None);

        self.apply_dpi();
        self.apply_cursor();
        self.status.reset(&self.cfg);
        self.apply_wallpaper(true);
        if std::env::var_os("TRWM_RESTARTED").is_none() {
            for cmd in self.cfg.autostart.clone() {
                self.spawn(&Spawn::Shell(cmd));
            }
        }
        // SAFETY: single-threaded at this point
        unsafe { std::env::remove_var("TRWM_RESTARTED") };
    }

    pub fn run(mut self) -> bool {
        unsafe {
            self.scanning = true;
            self.scan();
            self.scanning = false;
            xlib::XSync(self.dpy, xlib::False);
            let fd = xlib::XConnectionNumber(self.dpy);
            while self.running {
                while self.running && xlib::XPending(self.dpy) > 0 {
                    let mut ev: xlib::XEvent = std::mem::zeroed();
                    xlib::XNextEvent(self.dpy, &mut ev);
                    // a bug in one handler must not take the whole X session down
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(&mut ev)));
                    if r.is_err() {
                        self.error = Some(("internal error, see stderr".into(), Instant::now()));
                    }
                }
                if !self.running {
                    break;
                }
                let now = Instant::now();
                if now >= self.next_tick {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.tick()));
                    // align ticks to wall-clock seconds so the clock is exact
                    let sub = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
                    self.next_tick = Instant::now() + Duration::from_millis(1000 - sub as u64 + 5);
                }
                let timeout = self.next_tick.saturating_duration_since(Instant::now()).as_millis().min(1000) as c_int;
                xlib::XFlush(self.dpy);
                let mut pfd = [
                    libc::pollfd { fd, events: libc::POLLIN, revents: 0 },
                    libc::pollfd { fd: self.status.wake_fd(), events: libc::POLLIN, revents: 0 },
                ];
                libc::poll(pfd.as_mut_ptr(), 2, timeout.max(1));
                // a background run_command finished: show it right away
                if pfd[1].revents & libc::POLLIN != 0 && self.status.drain() {
                    self.updatestatus();
                }
            }
            self.cleanup();
        }
        self.restart
    }

    /// Called once a second: status, hot reload, idle lock, children.
    unsafe fn tick(&mut self) {
        self.children.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_)) | Err(_)));
        if let Some(c) = &mut self.lock_child
            && !matches!(c.try_wait(), Ok(None)) {
                self.lock_child = None;
            }

        // hot reload
        if let Some(p) = &self.cfg_path {
            let m = config::newest_mtime(p);
            if m.is_some() && m != self.cfg_mtime {
                self.cfg_mtime = m;
                self.reload();
            }
        } else if config::default_path().exists() {
            // a config file was created while we ran on the built-in default
            self.cfg_path = Some(config::default_path());
            self.reload();
        }

        // auto lock when idle
        if self.cfg.lock.idle > 0 {
            let mut eb = 0;
            let mut er = 0;
            if xss::XScreenSaverQueryExtension(self.dpy, &mut eb, &mut er) != 0 {
                let info = xss::XScreenSaverAllocInfo();
                xss::XScreenSaverQueryInfo(self.dpy, self.root, info);
                let idle = (*info).idle;
                xlib::XFree(info as *mut _);
                if idle >= self.cfg.lock.idle * 1000 {
                    if !self.idle_locked {
                        self.idle_locked = true;
                        self.lock();
                    }
                } else {
                    self.idle_locked = false;
                }
            }
        }

        if let Some(t) = self.wallpaper_next
            && Instant::now() >= t {
                self.apply_wallpaper(false);
            }

        if let Some((_, t)) = &self.error
            && t.elapsed() > Duration::from_secs(20) {
                self.error = None;
                self.updatestatus();
            }
        if self.cfg.status.enabled && self.status.tick(&self.cfg) {
            self.updatestatus();
        }
    }

    unsafe fn cleanup(&mut self) {
        let dpy = self.dpy;
        // show everything so nothing is lost offscreen
        for m in 0..self.mons.len() {
            let st = self.mons[m].seltags;
            self.mons[m].tagset[st] = !0;
        }
        let fl = self.layout_index(LayoutKind::Float).unwrap_or(0);
        let sm = self.selmon;
        let sl = self.mons[sm].sellt;
        self.mons[sm].lt[sl] = fl;
        for m in 0..self.mons.len() {
            while let Some(&w) = self.mons[m].stack.first() {
                self.unmanage(w, false);
            }
        }
        xlib::XUngrabKey(dpy, xlib::AnyKey, xlib::AnyModifier, self.root);
        for m in &self.mons {
            xlib::XUnmapWindow(dpy, m.barwin);
            xlib::XDestroyWindow(dpy, m.barwin);
        }
        for c in self.cursors {
            xlib::XFreeCursor(dpy, c);
        }
        xlib::XDestroyWindow(dpy, self.wmcheckwin);
        xlib::XSync(dpy, xlib::False);
        xlib::XSetInputFocus(dpy, xlib::PointerRoot as Window, xlib::RevertToPointerRoot, xlib::CurrentTime);
        xlib::XDeleteProperty(dpy, self.root, self.na(Net::ActiveWindow));
    }

    /// Adopt windows that already exist (after a restart, for example).
    unsafe fn scan(&mut self) {
        let (mut d1, mut d2) = (0, 0);
        let mut wins: *mut Window = ptr::null_mut();
        let mut num: c_uint = 0;
        if xlib::XQueryTree(self.dpy, self.root, &mut d1, &mut d2, &mut wins, &mut num) == 0 {
            return;
        }
        let list: Vec<Window> = std::slice::from_raw_parts(wins, num as usize).to_vec();
        if !wins.is_null() {
            xlib::XFree(wins as *mut _);
        }
        let mut wa: xlib::XWindowAttributes = std::mem::zeroed();
        // normal windows first, then transients
        for pass in 0..2 {
            for &w in &list {
                if xlib::XGetWindowAttributes(self.dpy, w, &mut wa) == 0 || wa.override_redirect != 0 {
                    continue;
                }
                let mut tr: Window = 0;
                let is_trans = xlib::XGetTransientForHint(self.dpy, w, &mut tr) != 0;
                if (pass == 0) == is_trans {
                    continue;
                }
                if wa.map_state == xlib::IsViewable || self.getstate(w) == Some(ICONIC_STATE) {
                    self.manage(w, &wa);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // config reload
    // -----------------------------------------------------------------------

    pub fn startup_error(&mut self, msg: String) {
        self.show_error(msg);
    }

    fn show_error(&mut self, msg: String) {
        eprintln!("trwm: {msg}");
        let first = msg.lines().next().unwrap_or("").to_string();
        self.error = Some((first, Instant::now()));
        unsafe { self.updatestatus() };
    }

    unsafe fn reload(&mut self) {
        match Config::load(self.cfg_path.as_deref()) {
            Ok(new) => {
                self.apply_config(new);
                eprintln!("trwm: config reloaded");
            }
            Err(e) => self.show_error(format!("config error: {e}")),
        }
    }

    unsafe fn apply_config(&mut self, new: Config) {
        let old = std::mem::replace(&mut self.cfg, new);
        let cfg = &self.cfg;

        let fonts = config::with_dpi(&cfg.fonts, cfg.dpi);
        if fonts != config::with_dpi(&old.fonts, old.dpi) && !self.drw.set_fonts(&fonts) {
            self.error = Some(("could not load any configured font".into(), Instant::now()));
        }
        let fh = self.drw.font_h() as i32;
        self.bh = fh + 2;
        self.lrpad = fh;
        self.drw.resize(self.sw as u32, self.bh as u32);
        self.norm = self.drw.scheme(&cfg.norm.fg, &cfg.norm.bg, &cfg.norm.border);
        self.sel = self.drw.scheme(&cfg.sel.fg, &cfg.sel.bg, &cfg.sel.border);

        let tagmask = self.tagmask();
        let nlayouts = cfg.layouts.len();
        for m in &mut self.mons {
            if cfg.mfact != old.mfact {
                m.mfact = cfg.mfact;
            }
            if cfg.nmaster != old.nmaster {
                m.nmaster = cfg.nmaster;
            }
            if cfg.show_bar != old.show_bar {
                m.showbar = cfg.show_bar;
            }
            m.topbar = cfg.top_bar;
            if cfg.gaps != old.gaps {
                m.gaps = cfg.gaps;
            }
            for l in &mut m.lt {
                if *l >= nlayouts {
                    *l = 0;
                }
            }
            for t in &mut m.tagset {
                *t &= tagmask;
                if *t == 0 {
                    *t = 1;
                }
            }
        }
        for c in self.clients.values_mut() {
            c.tags &= tagmask;
            if c.tags == 0 {
                c.tags = 1;
            }
            if !c.isfullscreen {
                c.bw = cfg.border_px as i32;
                xlib::XSetWindowBorderWidth(self.dpy, c.win, c.bw as c_uint);
            }
            xlib::XSetWindowBorder(self.dpy, c.win, self.norm.border.pixel);
        }
        for m in 0..self.mons.len() {
            let l = self.mons[m].lt[self.mons[m].sellt];
            self.mons[m].ltsymbol = self.cfg.layouts[l].symbol.clone();
            self.updatebarpos(m);
            let mm = &self.mons[m];
            xlib::XMoveResizeWindow(self.dpy, mm.barwin, mm.wx, mm.by, mm.ww as c_uint, self.bh as c_uint);
        }
        self.grabkeys();
        let sel = self.mons[self.selmon].sel;
        for w in self.clients.keys().copied().collect::<Vec<_>>() {
            self.grabbuttons(w, Some(w) == sel);
        }
        if let Some(s) = sel {
            xlib::XSetWindowBorder(self.dpy, s, self.sel.border.pixel);
        }
        if old.dpi != self.cfg.dpi {
            self.apply_dpi();
        }
        if old.cursor_size != self.cfg.cursor_size || old.cursor_theme != self.cfg.cursor_theme {
            self.apply_cursor();
        }
        if old.wallpaper != self.cfg.wallpaper {
            self.apply_wallpaper(true);
        }
        self.status.reset(&self.cfg);
        self.status.tick(&self.cfg);
        self.error = None;
        self.update_desktop_props();
        self.updatestatus();
        self.arrange(None);
    }

    // -----------------------------------------------------------------------
    // event dispatch
    // -----------------------------------------------------------------------

    unsafe fn handle(&mut self, ev: &mut xlib::XEvent) {
        match ev.get_type() {
            xlib::ButtonPress => self.buttonpress(&ev.button),
            xlib::ClientMessage => self.clientmessage(&ev.client_message),
            xlib::ConfigureRequest => self.configurerequest(&ev.configure_request),
            xlib::ConfigureNotify => self.configurenotify(&ev.configure),
            xlib::DestroyNotify => {
                let w = ev.destroy_window.window;
                if self.clients.contains_key(&w) {
                    self.unmanage(w, true);
                }
            }
            xlib::EnterNotify => self.enternotify(&ev.crossing),
            xlib::Expose => {
                if ev.expose.count == 0
                    && let Some(m) = self.wintomon(ev.expose.window) {
                        self.drawbar(m);
                    }
            }
            xlib::FocusIn => {
                // there are some broken focus acquiring clients needing extra handling
                if let Some(s) = self.mons[self.selmon].sel
                    && ev.focus_change.window != s {
                        self.setfocus(s);
                    }
            }
            xlib::KeyPress => self.keypress(&mut ev.key),
            xlib::MappingNotify => {
                xlib::XRefreshKeyboardMapping(&mut ev.mapping);
                if ev.mapping.request == xlib::MappingKeyboard {
                    self.grabkeys();
                }
            }
            xlib::MapRequest => {
                let w = ev.map_request.window;
                let mut wa: xlib::XWindowAttributes = std::mem::zeroed();
                if xlib::XGetWindowAttributes(self.dpy, w, &mut wa) == 0 || wa.override_redirect != 0 {
                    return;
                }
                if !self.clients.contains_key(&w) {
                    self.manage(w, &wa);
                }
            }
            xlib::MotionNotify => self.motionnotify(&ev.motion),
            xlib::PropertyNotify => self.propertynotify(&ev.property),
            xlib::UnmapNotify => {
                let e = &ev.unmap;
                if self.clients.contains_key(&e.window) {
                    if e.send_event != 0 {
                        self.setclientstate(e.window, WITHDRAWN_STATE);
                    } else {
                        self.unmanage(e.window, false);
                    }
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // commands (what Lua actions do)
    // -----------------------------------------------------------------------

    unsafe fn run_lua(&mut self, f: mlua::Function, arg: Option<u32>) {
        match self.cfg.call(&f, arg) {
            Ok(cmds) => {
                for c in cmds {
                    self.exec(c);
                    if !self.running {
                        break;
                    }
                }
            }
            Err(e) => self.show_error(format!("lua: {e}")),
        }
    }

    unsafe fn exec(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Spawn(s) => self.spawn(&s),
            Cmd::View(t) => self.view(t),
            Cmd::ToggleView(t) => self.toggleview(t),
            Cmd::Tag(t) => self.tag(t),
            Cmd::ToggleTag(t) => self.toggletag(t),
            Cmd::FocusStack(d) => self.focusstack(d),
            Cmd::IncNMaster(d) => {
                let m = self.selmon;
                self.mons[m].nmaster = (self.mons[m].nmaster + d).max(0);
                self.arrange(Some(m));
            }
            Cmd::SetMFact(f) => self.setmfact(f),
            Cmd::Zoom => self.zoom(),
            Cmd::KillClient => self.killclient(),
            Cmd::SetLayout(l) => self.setlayout(l),
            Cmd::CycleLayout(d) => {
                let m = &self.mons[self.selmon];
                let n = self.cfg.layouts.len() as i32;
                let cur = m.lt[m.sellt] as i32;
                let next = (cur + d).rem_euclid(n) as usize;
                self.setlayout(LayoutSel::Index(next));
            }
            Cmd::ToggleFloating => self.togglefloating(),
            Cmd::ToggleFullscreen => {
                if let Some(s) = self.mons[self.selmon].sel {
                    let fs = self.clients[&s].isfullscreen;
                    self.setfullscreen(s, !fs);
                }
            }
            Cmd::ToggleBar => {
                let m = self.selmon;
                self.mons[m].showbar = !self.mons[m].showbar;
                self.updatebarpos(m);
                let mm = &self.mons[m];
                xlib::XMoveResizeWindow(self.dpy, mm.barwin, mm.wx, mm.by, mm.ww as c_uint, self.bh as c_uint);
                self.arrange(Some(m));
            }
            Cmd::FocusMon(d) => {
                if self.mons.len() <= 1 {
                    return;
                }
                let m = self.dirtomon(d);
                if m == self.selmon {
                    return;
                }
                if let Some(s) = self.mons[self.selmon].sel {
                    self.unfocus(s, false);
                }
                self.selmon = m;
                self.focus(None);
            }
            Cmd::TagMon(d) => {
                if let Some(s) = self.mons[self.selmon].sel
                    && self.mons.len() > 1 {
                        let m = self.dirtomon(d);
                        self.sendmon(s, m);
                    }
            }
            Cmd::MoveMouse => self.movemouse(),
            Cmd::ResizeMouse => self.resizemouse(),
            Cmd::ToggleGaps => {
                let m = self.selmon;
                self.mons[m].gaps.enabled = !self.mons[m].gaps.enabled;
                self.arrange(Some(m));
            }
            Cmd::DefaultGaps => {
                let m = self.selmon;
                self.mons[m].gaps = Gaps { enabled: true, ..self.cfg.gaps };
                self.arrange(Some(m));
            }
            Cmd::IncGaps(d) => {
                let m = self.selmon;
                let g = &mut self.mons[m].gaps;
                g.ih = (g.ih + d).max(0);
                g.iv = (g.iv + d).max(0);
                g.oh = (g.oh + d).max(0);
                g.ov = (g.ov + d).max(0);
                self.arrange(Some(m));
            }
            Cmd::Wallpaper(path, mode) => {
                let path = path.or_else(|| self.cfg.wallpaper.as_ref().map(|w| w.path.clone()));
                let mode = mode.or_else(|| self.cfg.wallpaper.as_ref().map(|w| w.mode.clone())).unwrap_or_else(|| "fill".into());
                if let Some(p) = path {
                    self.feh(&p, &mode);
                }
            }
            Cmd::RefreshStatus => {
                self.status.refresh();
                self.next_tick = Instant::now();
            }
            Cmd::Lock => self.lock(),
            Cmd::Reload => {
                if self.cfg_path.is_none() && config::default_path().exists() {
                    self.cfg_path = Some(config::default_path());
                }
                self.reload();
            }
            Cmd::Restart => {
                self.restart = true;
                self.running = false;
            }
            Cmd::Quit => self.running = false,
        }
    }

    fn spawn(&mut self, s: &Spawn) {
        let mut cmd = match s {
            Spawn::Shell(sh) => {
                let mut c = Command::new("/bin/sh");
                c.arg("-c").arg(sh);
                c
            }
            Spawn::Argv(a) => {
                let mut c = Command::new(&a[0]);
                c.args(&a[1..]);
                c
            }
        };
        cmd.stdin(Stdio::null());
        // programs started from trwm get the configured cursor
        if let Some(s) = self.cfg.cursor_size {
            cmd.env("XCURSOR_SIZE", s.to_string());
        }
        if let Some(t) = &self.cfg.cursor_theme {
            cmd.env("XCURSOR_THEME", t);
        }
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(c) => self.children.push(c),
            Err(e) => self.show_error(format!("spawn {s:?}: {e}")),
        }
    }

    fn lock(&mut self) {
        if self.lock_child.is_some() {
            return;
        }
        let exe = exe_path();
        let mut cmd = Command::new(exe);
        if let Some(p) = &self.cfg_path {
            cmd.arg("-c").arg(p);
        }
        cmd.arg("lock").stdin(Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(c) => self.lock_child = Some(c),
            Err(e) => self.show_error(format!("lock: {e}")),
        }
    }

    /// `cursor_size` / `cursor_theme`: what `Xcursor.size` / `Xcursor.theme` in
    /// ~/.Xresources do, plus trwm's own cursors (root window, bar, move, resize).
    unsafe fn apply_cursor(&mut self) {
        let (size, theme) = (self.cfg.cursor_size, self.cfg.cursor_theme.clone());
        if size.is_none() && theme.is_none() {
            return;
        }
        if let Some(s) = size {
            xcursor::XcursorSetDefaultSize(self.dpy, s as c_int);
        }
        if let Some(t) = &theme
            && let Ok(c) = CString::new(t.as_str())
        {
            xcursor::XcursorSetTheme(self.dpy, c.as_ptr());
        }
        // the resource database, for programs not started by trwm
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        let mut res = String::new();
        if xlib::XGetWindowProperty(self.dpy, self.root, xlib::XA_RESOURCE_MANAGER, 0, 1 << 22, xlib::False, xlib::XA_STRING, &mut real, &mut format, &mut n, &mut extra, &mut p)
            == xlib::Success as c_int
            && !p.is_null()
        {
            res = String::from_utf8_lossy(std::slice::from_raw_parts(p, n as usize)).into_owned();
            xlib::XFree(p as *mut _);
        }
        if let Some(s) = size {
            res = config::set_resource(&res, "Xcursor.size", &s.to_string());
        }
        if let Some(t) = &theme {
            res = config::set_resource(&res, "Xcursor.theme", t);
        }
        xlib::XChangeProperty(self.dpy, self.root, xlib::XA_RESOURCE_MANAGER, xlib::XA_STRING, 8, xlib::PropModeReplace, res.as_ptr(), res.len() as c_int);

        // recreate our own cursors with the new size/theme
        let new = [
            xlib::XCreateFontCursor(self.dpy, 68),  // XC_left_ptr
            xlib::XCreateFontCursor(self.dpy, 120), // XC_sizing
            xlib::XCreateFontCursor(self.dpy, 52),  // XC_fleur
        ];
        xlib::XDefineCursor(self.dpy, self.root, new[0]);
        for m in &self.mons {
            xlib::XDefineCursor(self.dpy, m.barwin, new[0]);
        }
        for c in std::mem::replace(&mut self.cursors, new) {
            xlib::XFreeCursor(self.dpy, c);
        }
        xlib::XSync(self.dpy, xlib::False);
    }

    /// `dpi = N`: what `xrandr --dpi N` and `Xft.dpi: N` in ~/.Xresources do.
    /// Programs started afterwards pick it up.
    unsafe fn apply_dpi(&mut self) {
        let Some(dpi) = self.cfg.dpi else { return };
        // 1. the physical screen size the X server reports (xrandr --dpi)
        let (mut eb, mut er) = (0, 0);
        if xrandr::XRRQueryExtension(self.dpy, &mut eb, &mut er) != 0 {
            let mm = |px: i32| ((px as f64 * 25.4 / dpi as f64).round() as c_int).max(1);
            xrandr::XRRSetScreenSize(self.dpy, self.root, self.sw, self.sh, mm(self.sw), mm(self.sh));
        }
        // 2. Xft.dpi in the X resource database (what xrdb sets), keeping everything else
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        let mut res = String::new();
        if xlib::XGetWindowProperty(self.dpy, self.root, xlib::XA_RESOURCE_MANAGER, 0, 1 << 22, xlib::False, xlib::XA_STRING, &mut real, &mut format, &mut n, &mut extra, &mut p)
            == xlib::Success as c_int
            && !p.is_null()
        {
            res = String::from_utf8_lossy(std::slice::from_raw_parts(p, n as usize)).into_owned();
            xlib::XFree(p as *mut _);
        }
        let out = config::set_resource(&res, "Xft.dpi", &dpi.to_string());
        xlib::XChangeProperty(self.dpy, self.root, xlib::XA_RESOURCE_MANAGER, xlib::XA_STRING, 8, xlib::PropModeReplace, out.as_ptr(), out.len() as c_int);
        xlib::XSync(self.dpy, xlib::False);
    }

    fn feh(&mut self, path: &str, mode: &str) {
        let p = config::expand_tilde(path);
        let mut argv = vec!["feh".to_string(), "--no-fehbg".into(), format!("--bg-{mode}")];
        if std::path::Path::new(&p).is_dir() {
            argv.push("--randomize".into());
            argv.push("--recursive".into());
        }
        argv.push(p);
        self.spawn(&Spawn::Argv(argv));
    }

    fn apply_wallpaper(&mut self, _initial: bool) {
        self.wallpaper_next = None;
        let Some(w) = self.cfg.wallpaper.clone() else { return };
        self.feh(&w.path, &w.mode);
        if w.interval > 0 {
            self.wallpaper_next = Some(Instant::now() + Duration::from_secs(w.interval));
        }
    }

    // -----------------------------------------------------------------------
    // helpers
    // -----------------------------------------------------------------------

    fn tagmask(&self) -> u32 {
        let n = self.cfg.tags.len();
        if n >= 32 { !0 } else { (1u32 << n) - 1 }
    }

    fn cleanmask(&self, m: c_uint) -> c_uint {
        m & !(self.numlockmask | xlib::LockMask)
            & (xlib::ShiftMask | xlib::ControlMask | xlib::Mod1Mask | xlib::Mod2Mask | xlib::Mod3Mask | xlib::Mod4Mask | xlib::Mod5Mask)
    }

    fn layout_index(&self, k: LayoutKind) -> Option<usize> {
        self.cfg.layouts.iter().position(|l| l.kind == k)
    }

    fn cur_layout(&self, m: usize) -> LayoutKind {
        let mon = &self.mons[m];
        self.cfg.layouts.get(mon.lt[mon.sellt]).map(|l| l.kind).unwrap_or(LayoutKind::Tile)
    }

    fn arranges(&self, m: usize) -> bool {
        self.cur_layout(m) != LayoutKind::Float
    }

    fn is_visible(&self, w: Window) -> bool {
        let c = &self.clients[&w];
        let m = &self.mons[c.mon];
        c.tags & m.tagset[m.seltags] != 0
    }

    fn tiled(&self, m: usize) -> Vec<Window> {
        self.mons[m].clients.iter().copied().filter(|w| self.is_visible(*w) && !self.clients[w].isfloating).collect()
    }

    fn textw(&self, s: &str) -> i32 {
        self.drw.text_width(s) as i32 + self.lrpad
    }

    fn wintomon(&self, w: Window) -> Option<usize> {
        if w == self.root {
            let (x, y) = unsafe { self.getrootptr() }?;
            return Some(self.recttomon(x, y, 1, 1));
        }
        if let Some(m) = self.mons.iter().position(|m| m.barwin == w) {
            return Some(m);
        }
        if let Some(c) = self.clients.get(&w) {
            return Some(c.mon);
        }
        Some(self.selmon)
    }

    fn recttomon(&self, x: i32, y: i32, w: i32, h: i32) -> usize {
        let mut r = self.selmon;
        let mut area = 0;
        for (i, m) in self.mons.iter().enumerate() {
            let a = (x + w).min(m.wx + m.ww).saturating_sub(x.max(m.wx)).max(0) * (y + h).min(m.wy + m.wh).saturating_sub(y.max(m.wy)).max(0);
            if a > area {
                area = a;
                r = i;
            }
        }
        r
    }

    fn dirtomon(&self, dir: i32) -> usize {
        (self.selmon as i32 + dir).rem_euclid(self.mons.len() as i32) as usize
    }

    unsafe fn getrootptr(&self) -> Option<(i32, i32)> {
        let (mut d, mut d2) = (0, 0);
        let (mut x, mut y, mut di, mut dj) = (0, 0, 0, 0);
        let mut mask = 0;
        (xlib::XQueryPointer(self.dpy, self.root, &mut d, &mut d2, &mut x, &mut y, &mut di, &mut dj, &mut mask) != 0).then_some((x, y))
    }

    unsafe fn getstate(&self, w: Window) -> Option<c_long> {
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        let st = self.wa(Wm::State);
        if xlib::XGetWindowProperty(self.dpy, w, st, 0, 2, xlib::False, st, &mut real, &mut format, &mut n, &mut extra, &mut p) != xlib::Success as c_int {
            return None;
        }
        let r = (n != 0 && !p.is_null()).then(|| *(p as *const c_long));
        if !p.is_null() {
            xlib::XFree(p as *mut _);
        }
        r
    }

    unsafe fn getatomprop(&self, w: Window, prop: Atom) -> Atom {
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        let mut atom = 0;
        if xlib::XGetWindowProperty(self.dpy, w, prop, 0, std::mem::size_of::<Atom>() as c_long, xlib::False, xlib::XA_ATOM, &mut real, &mut format, &mut n, &mut extra, &mut p)
            == xlib::Success as c_int
            && !p.is_null()
        {
            if n > 0 {
                atom = *(p as *const Atom);
            }
            xlib::XFree(p as *mut _);
        }
        atom
    }

    unsafe fn gettextprop(&self, w: Window, atom: Atom) -> Option<String> {
        let mut name: xlib::XTextProperty = std::mem::zeroed();
        if xlib::XGetTextProperty(self.dpy, w, &mut name, atom) == 0 || name.nitems == 0 {
            return None;
        }
        let mut out = None;
        if name.encoding == xlib::XA_STRING {
            out = Some(CStr::from_ptr(name.value as *const c_char).to_string_lossy().into_owned());
        } else {
            let mut list: *mut *mut c_char = ptr::null_mut();
            let mut n = 0;
            if xlib::Xutf8TextPropertyToTextList(self.dpy, &name, &mut list, &mut n) >= xlib::Success as c_int && n > 0 && !list.is_null() {
                out = Some(CStr::from_ptr(*list).to_string_lossy().into_owned());
                xlib::XFreeStringList(list);
            }
        }
        xlib::XFree(name.value as *mut _);
        out
    }

    unsafe fn sendevent(&self, w: Window, proto: Atom) -> bool {
        let mut protocols: *mut Atom = ptr::null_mut();
        let mut n = 0;
        let mut exists = false;
        if xlib::XGetWMProtocols(self.dpy, w, &mut protocols, &mut n) != 0 {
            exists = std::slice::from_raw_parts(protocols, n as usize).contains(&proto);
            xlib::XFree(protocols as *mut _);
        }
        if exists {
            let mut ev: xlib::XEvent = std::mem::zeroed();
            ev.client_message.type_ = xlib::ClientMessage;
            ev.client_message.window = w;
            ev.client_message.message_type = self.wa(Wm::Protocols);
            ev.client_message.format = 32;
            ev.client_message.data.set_long(0, proto as c_long);
            ev.client_message.data.set_long(1, xlib::CurrentTime as c_long);
            xlib::XSendEvent(self.dpy, w, xlib::False, xlib::NoEventMask, &mut ev);
        }
        exists
    }

    unsafe fn setclientstate(&self, w: Window, state: c_long) {
        let data: [c_long; 2] = [state, 0];
        let st = self.wa(Wm::State);
        xlib::XChangeProperty(self.dpy, w, st, st, 32, xlib::PropModeReplace, data.as_ptr() as *const c_uchar, 2);
    }

    /// Remember tags + monitor on the window so `restart` keeps them.
    unsafe fn save_client_tags(&self, w: Window) {
        let c = &self.clients[&w];
        let data: [c_long; 2] = [c.tags as c_long, self.mons[c.mon].num as c_long];
        xlib::XChangeProperty(self.dpy, w, self.tags_atom, xlib::XA_CARDINAL, 32, xlib::PropModeReplace, data.as_ptr() as *const c_uchar, 2);
    }

    unsafe fn load_client_tags(&self, w: Window) -> Option<(u32, i32)> {
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        if xlib::XGetWindowProperty(self.dpy, w, self.tags_atom, 0, 2, xlib::False, xlib::XA_CARDINAL, &mut real, &mut format, &mut n, &mut extra, &mut p) != xlib::Success as c_int
            || p.is_null()
        {
            return None;
        }
        let r = (n == 2).then(|| {
            let d = std::slice::from_raw_parts(p as *const c_long, 2);
            (d[0] as u32, d[1] as i32)
        });
        xlib::XFree(p as *mut _);
        r
    }

    unsafe fn update_desktop_props(&self) {
        let n: c_long = self.cfg.tags.len() as c_long;
        xlib::XChangeProperty(self.dpy, self.root, self.na(Net::NumberOfDesktops), xlib::XA_CARDINAL, 32, xlib::PropModeReplace, &n as *const _ as *const c_uchar, 1);
        let mut names = Vec::new();
        for t in &self.cfg.tags {
            names.extend_from_slice(t.as_bytes());
            names.push(0);
        }
        xlib::XChangeProperty(self.dpy, self.root, self.na(Net::DesktopNames), self.utf8, 8, xlib::PropModeReplace, names.as_ptr(), names.len() as c_int);
        if let Some(m) = self.mons.get(self.selmon) {
            let cur: c_long = m.tagset[m.seltags].trailing_zeros() as c_long;
            xlib::XChangeProperty(self.dpy, self.root, self.na(Net::CurrentDesktop), xlib::XA_CARDINAL, 32, xlib::PropModeReplace, &cur as *const _ as *const c_uchar, 1);
        }
    }

    // -----------------------------------------------------------------------
    // monitors & bar
    // -----------------------------------------------------------------------

    fn createmon(&self) -> Monitor {
        let lt1 = if self.cfg.layouts.len() > 1 { 1 } else { 0 };
        Monitor {
            ltsymbol: self.cfg.layouts[0].symbol.clone(),
            mfact: self.cfg.mfact,
            nmaster: self.cfg.nmaster,
            num: 0,
            by: 0,
            mx: 0,
            my: 0,
            mw: 0,
            mh: 0,
            wx: 0,
            wy: 0,
            ww: 0,
            wh: 0,
            gaps: self.cfg.gaps,
            seltags: 0,
            sellt: 0,
            tagset: [1, 1],
            showbar: self.cfg.show_bar,
            topbar: self.cfg.top_bar,
            clients: vec![],
            stack: vec![],
            sel: None,
            lt: [0, lt1],
            barwin: 0,
            bar_tags: vec![],
            bar_lt_end: 0,
            bar_status_x: 0,
        }
    }

    fn updatebarpos(&mut self, m: usize) {
        let bh = self.bh;
        let mon = &mut self.mons[m];
        mon.wy = mon.my;
        mon.wh = mon.mh;
        if mon.showbar {
            mon.wh -= bh;
            mon.by = if mon.topbar { mon.wy } else { mon.wy + mon.wh };
            mon.wy = if mon.topbar { mon.wy + bh } else { mon.wy };
        } else {
            mon.by = -bh;
        }
    }

    unsafe fn updatebars(&mut self) {
        let mut wa: xlib::XSetWindowAttributes = std::mem::zeroed();
        wa.override_redirect = xlib::True;
        wa.background_pixmap = xlib::ParentRelative as c_ulong;
        wa.event_mask = xlib::ButtonPressMask | xlib::ExposureMask;
        let class = CString::new("trwm").unwrap();
        for m in 0..self.mons.len() {
            if self.mons[m].barwin != 0 {
                continue;
            }
            let mm = &self.mons[m];
            let win = xlib::XCreateWindow(
                self.dpy,
                self.root,
                mm.wx,
                mm.by,
                mm.ww as c_uint,
                self.bh as c_uint,
                0,
                xlib::XDefaultDepth(self.dpy, self.screen),
                xlib::CopyFromParent as c_uint,
                xlib::XDefaultVisual(self.dpy, self.screen),
                xlib::CWOverrideRedirect | xlib::CWBackPixmap | xlib::CWEventMask,
                &mut wa,
            );
            xlib::XDefineCursor(self.dpy, win, self.cursors[0]);
            xlib::XMapRaised(self.dpy, win);
            let mut ch = xlib::XClassHint { res_name: class.as_ptr() as *mut _, res_class: class.as_ptr() as *mut _ };
            xlib::XSetClassHint(self.dpy, win, &mut ch);
            self.mons[m].barwin = win;
        }
    }

    /// (Re)read the monitor layout from Xinerama. Returns true if it changed.
    unsafe fn updategeom(&mut self) -> bool {
        let mut dirty = false;
        let mut heads: Vec<(i32, i32, i32, i32)> = Vec::new();
        if xinerama::XineramaIsActive(self.dpy) != 0 {
            let mut n = 0;
            let info = xinerama::XineramaQueryScreens(self.dpy, &mut n);
            for i in 0..n as usize {
                let s = &*info.add(i);
                let g = (s.x_org as i32, s.y_org as i32, s.width as i32, s.height as i32);
                if !heads.contains(&g) {
                    heads.push(g); // only unique geometries
                }
            }
            if !info.is_null() {
                xlib::XFree(info as *mut _);
            }
        }
        if heads.is_empty() {
            heads.push((0, 0, self.sw, self.sh));
        }
        while self.mons.len() < heads.len() {
            let m = self.createmon();
            self.mons.push(m);
            dirty = true;
        }
        for (i, &(x, y, w, h)) in heads.iter().enumerate() {
            let m = &mut self.mons[i];
            if dirty || m.mx != x || m.my != y || m.mw != w || m.mh != h {
                dirty = true;
                m.num = i as i32;
                m.mx = x;
                m.wx = x;
                m.my = y;
                m.wy = y;
                m.mw = w;
                m.ww = w;
                m.mh = h;
                m.wh = h;
                self.updatebarpos(i);
            }
        }
        // removed monitors: move their clients to the first one
        while self.mons.len() > heads.len() {
            dirty = true;
            let gone = self.mons.pop().unwrap();
            for w in gone.clients.iter().rev() {
                if let Some(c) = self.clients.get_mut(w) {
                    c.mon = 0;
                }
                self.mons[0].clients.insert(0, *w);
                self.mons[0].stack.insert(0, *w);
            }
            if gone.barwin != 0 {
                xlib::XUnmapWindow(self.dpy, gone.barwin);
                xlib::XDestroyWindow(self.dpy, gone.barwin);
            }
            if self.selmon >= self.mons.len() {
                self.selmon = 0;
            }
        }
        if dirty {
            self.selmon = 0;
            if let Some((x, y)) = self.getrootptr() {
                self.selmon = self.recttomon(x, y, 1, 1);
            }
        }
        self.updatebars();
        dirty
    }

    unsafe fn updatestatus(&mut self) {
        self.stext = if let Some((e, _)) = &self.error {
            format!("trwm: {e}")
        } else if self.cfg.status.enabled {
            self.status.text(&self.cfg)
        } else {
            // like dwm: fall back to the root window name (xsetroot -name)
            self.gettextprop(self.root, xlib::XA_WM_NAME).unwrap_or_else(|| format!("trwm-{}", env!("CARGO_PKG_VERSION")))
        };
        self.drawbar(self.selmon);
    }

    unsafe fn drawbars(&mut self) {
        for m in 0..self.mons.len() {
            self.drawbar(m);
        }
    }

    unsafe fn drawbar(&mut self, m: usize) {
        if !self.mons[m].showbar || self.mons[m].barwin == 0 {
            return;
        }
        let bh = self.bh;
        let boxs = self.drw.font_h() as i32 / 9;
        let boxw = self.drw.font_h() as i32 / 6 + 2;
        let ww = self.mons[m].ww;
        let lrpad = self.lrpad;
        let mut tw = 0;

        // status is only drawn on the selected monitor
        if m == self.selmon {
            let n = self.norm;
            self.drw.set_scheme(&n);
            tw = self.drw.text_width(&self.stext) as i32 + 2;
            self.drw.text(ww - tw, 0, tw as u32, bh as u32, 0, &self.stext, false);
        }
        self.mons[m].bar_status_x = ww - tw;

        let (mut occ, mut urg) = (0u32, 0u32);
        for w in &self.mons[m].clients {
            let c = &self.clients[w];
            if c.tags != self.tagmask() {
                occ |= c.tags;
            }
            if c.isurgent {
                urg |= c.tags;
            }
        }
        let mon = &self.mons[m];
        let curtags = mon.tagset[mon.seltags];
        let seltags = mon.sel.map(|s| self.clients[&s].tags).unwrap_or(0);
        let mut x = 0;
        let mut ranges = Vec::new();
        for (i, name) in self.cfg.tags.iter().enumerate() {
            let bit = 1 << i;
            if self.cfg.hide_empty_tags && occ & bit == 0 && curtags & bit == 0 {
                continue;
            }
            let w = self.textw(name);
            let s = if curtags & bit != 0 { self.sel } else { self.norm };
            self.drw.set_scheme(&s);
            self.drw.text(x, 0, w as u32, bh as u32, (lrpad / 2) as u32, name, urg & bit != 0);
            if occ & bit != 0 {
                self.drw.rect(x + boxs, boxs, boxw as u32, boxw as u32, m == self.selmon && seltags & bit != 0, urg & bit != 0);
            }
            ranges.push((x, x + w, i));
            x += w;
        }
        let w = self.textw(&mon.ltsymbol);
        let n = self.norm;
        self.drw.set_scheme(&n);
        x = self.drw.text(x, 0, w as u32, bh as u32, (lrpad / 2) as u32, &mon.ltsymbol, false);
        let lt_end = x;

        let w = ww - tw - x;
        if w > bh {
            if let Some(s) = mon.sel {
                let c = &self.clients[&s];
                let sch = if m == self.selmon { self.sel } else { self.norm };
                self.drw.set_scheme(&sch);
                self.drw.text(x, 0, w as u32, bh as u32, (lrpad / 2) as u32, &c.name, false);
                if c.isfloating {
                    self.drw.rect(x + boxs, boxs, boxw as u32, boxw as u32, c.isfixed, false);
                }
            } else {
                self.drw.set_scheme(&n);
                self.drw.rect(x, 0, w as u32, bh as u32, true, true);
            }
        }
        self.drw.map(mon.barwin, 0, 0, ww as u32, bh as u32);
        let mon = &mut self.mons[m];
        mon.bar_tags = ranges;
        mon.bar_lt_end = lt_end;
    }

    // -----------------------------------------------------------------------
    // input
    // -----------------------------------------------------------------------

    unsafe fn updatenumlockmask(&mut self) {
        self.numlockmask = 0;
        let mm = xlib::XGetModifierMapping(self.dpy);
        let max = (*mm).max_keypermod as usize;
        let numlock = xlib::XKeysymToKeycode(self.dpy, x11::keysym::XK_Num_Lock as xlib::KeySym);
        for i in 0..8 {
            for j in 0..max {
                if *(*mm).modifiermap.add(i * max + j) == numlock {
                    self.numlockmask = 1 << i;
                }
            }
        }
        xlib::XFreeModifiermap(mm);
    }

    unsafe fn grabkeys(&mut self) {
        self.updatenumlockmask();
        let mods = [0, xlib::LockMask, self.numlockmask, self.numlockmask | xlib::LockMask];
        xlib::XUngrabKey(self.dpy, xlib::AnyKey, xlib::AnyModifier, self.root);
        for k in &self.cfg.keys {
            let code = xlib::XKeysymToKeycode(self.dpy, k.keysym);
            if code == 0 {
                continue;
            }
            for m in mods {
                xlib::XGrabKey(self.dpy, code as c_int, k.mods | m, self.root, xlib::True, xlib::GrabModeAsync, xlib::GrabModeAsync);
            }
        }
    }

    unsafe fn grabbuttons(&mut self, w: Window, focused: bool) {
        let mods = [0, xlib::LockMask, self.numlockmask, self.numlockmask | xlib::LockMask];
        xlib::XUngrabButton(self.dpy, xlib::AnyButton as c_uint, xlib::AnyModifier, w);
        if !focused {
            xlib::XGrabButton(self.dpy, xlib::AnyButton as c_uint, xlib::AnyModifier, w, xlib::False, BUTTON_MASK as c_uint, xlib::GrabModeSync, xlib::GrabModeSync, 0, 0);
        }
        for b in &self.cfg.buttons {
            if b.click == Click::ClientWin {
                for m in mods {
                    xlib::XGrabButton(self.dpy, b.button, b.mods | m, w, xlib::False, BUTTON_MASK as c_uint, xlib::GrabModeAsync, xlib::GrabModeSync, 0, 0);
                }
            }
        }
    }

    unsafe fn keypress(&mut self, e: &mut xlib::XKeyEvent) {
        let ks = xlib::XKeycodeToKeysym(self.dpy, e.keycode as xlib::KeyCode, 0);
        let state = self.cleanmask(e.state);
        let funcs: Vec<mlua::Function> = self
            .cfg
            .keys
            .iter()
            .filter(|k| k.keysym == ks && self.cleanmask(k.mods) == state)
            .map(|k| k.func.clone())
            .collect();
        for f in funcs {
            self.run_lua(f, None);
        }
    }

    unsafe fn buttonpress(&mut self, e: &xlib::XButtonEvent) {
        let mut click = Click::RootWin;
        let mut arg = None;
        // focus monitor if necessary
        if let Some(m) = self.wintomon(e.window)
            && m != self.selmon {
                if let Some(s) = self.mons[self.selmon].sel {
                    self.unfocus(s, true);
                }
                self.selmon = m;
                self.focus(None);
            }
        let sm = self.selmon;
        if e.window == self.mons[sm].barwin {
            let mon = &self.mons[sm];
            if let Some(&(_, _, i)) = mon.bar_tags.iter().find(|(a, b, _)| e.x >= *a && e.x < *b) {
                click = Click::TagBar;
                arg = Some(i as u32 + 1);
            } else if e.x < mon.bar_lt_end {
                click = Click::LtSymbol;
            } else if e.x > mon.bar_status_x {
                click = Click::StatusText;
            } else {
                click = Click::WinTitle;
            }
        } else if self.clients.contains_key(&e.window) {
            self.focus(Some(e.window));
            self.restack(self.selmon);
            xlib::XAllowEvents(self.dpy, xlib::ReplayPointer, xlib::CurrentTime);
            click = Click::ClientWin;
        }
        let state = self.cleanmask(e.state);
        let funcs: Vec<mlua::Function> = self
            .cfg
            .buttons
            .iter()
            .filter(|b| b.click == click && b.button == e.button && self.cleanmask(b.mods) == state)
            .map(|b| b.func.clone())
            .collect();
        for f in funcs {
            self.run_lua(f, arg);
        }
    }

    unsafe fn enternotify(&mut self, e: &xlib::XCrossingEvent) {
        if (e.mode != xlib::NotifyNormal || e.detail == xlib::NotifyInferior) && e.window != self.root {
            return;
        }
        let c = self.clients.contains_key(&e.window).then_some(e.window);
        let m = match c {
            Some(w) => self.clients[&w].mon,
            None => match self.wintomon(e.window) {
                Some(m) => m,
                None => return,
            },
        };
        if m != self.selmon {
            if let Some(s) = self.mons[self.selmon].sel {
                self.unfocus(s, true);
            }
            self.selmon = m;
        } else if !self.cfg.focus_follows_mouse || c.is_none() || c == self.mons[self.selmon].sel {
            return;
        }
        if self.cfg.focus_follows_mouse || c.is_none() {
            self.focus(c);
        } else {
            self.focus(None);
        }
    }

    unsafe fn motionnotify(&mut self, e: &xlib::XMotionEvent) {
        if e.window != self.root {
            return;
        }
        let m = self.recttomon(e.x_root, e.y_root, 1, 1);
        if let Some(prev) = self.motion_mon
            && m != prev {
                if let Some(s) = self.mons[self.selmon].sel {
                    self.unfocus(s, true);
                }
                self.selmon = m;
                self.focus(None);
            }
        self.motion_mon = Some(m);
    }

    // -----------------------------------------------------------------------
    // client messages / properties
    // -----------------------------------------------------------------------

    unsafe fn clientmessage(&mut self, e: &xlib::XClientMessageEvent) {
        let w = e.window;
        if !self.clients.contains_key(&w) {
            return;
        }
        if e.message_type == self.na(Net::WMState) {
            let fs = self.na(Net::WMFullscreen);
            if e.data.get_long(1) as Atom == fs || e.data.get_long(2) as Atom == fs {
                let isfs = self.clients[&w].isfullscreen;
                let action = e.data.get_long(0);
                // 1 = _NET_WM_STATE_ADD, 2 = _NET_WM_STATE_TOGGLE
                self.setfullscreen(w, action == 1 || (action == 2 && !isfs));
            }
        } else if e.message_type == self.na(Net::ActiveWindow)
            && Some(w) != self.mons[self.selmon].sel && !self.clients[&w].isurgent {
                self.seturgent(w, true);
            }
    }

    unsafe fn propertynotify(&mut self, e: &xlib::XPropertyEvent) {
        if e.window == self.root {
            if e.atom == xlib::XA_WM_NAME && !self.cfg.status.enabled {
                self.updatestatus();
            } else if e.atom == self.cmd_atom && e.state == xlib::PropertyNewValue {
                self.handle_ipc();
            }
            return;
        }
        if e.state == xlib::PropertyDelete || !self.clients.contains_key(&e.window) {
            return;
        }
        let w = e.window;
        match e.atom {
            xlib::XA_WM_TRANSIENT_FOR => {
                let mut trans: Window = 0;
                if !self.clients[&w].isfloating && xlib::XGetTransientForHint(self.dpy, w, &mut trans) != 0 && self.clients.contains_key(&trans) {
                    self.clients.get_mut(&w).unwrap().isfloating = true;
                    self.arrange(Some(self.clients[&w].mon));
                }
            }
            xlib::XA_WM_NORMAL_HINTS => self.clients.get_mut(&w).unwrap().hintsvalid = false,
            xlib::XA_WM_HINTS => {
                self.updatewmhints(w);
                self.drawbars();
            }
            _ => {}
        }
        if e.atom == xlib::XA_WM_NAME || e.atom == self.na(Net::WMName) {
            self.updatetitle(w);
            let m = self.clients[&w].mon;
            if self.mons[m].sel == Some(w) {
                self.drawbar(m);
            }
        }
        if e.atom == self.na(Net::WMWindowType) {
            self.updatewindowtype(w);
        }
    }

    /// `trwm msg ...` appends NUL-separated Lua snippets to a root property.
    unsafe fn handle_ipc(&mut self) {
        let mut real: Atom = 0;
        let mut format = 0;
        let (mut n, mut extra) = (0, 0);
        let mut p: *mut c_uchar = ptr::null_mut();
        if xlib::XGetWindowProperty(self.dpy, self.root, self.cmd_atom, 0, 1 << 20, xlib::True, xlib::AnyPropertyType as Atom, &mut real, &mut format, &mut n, &mut extra, &mut p)
            != xlib::Success as c_int
            || p.is_null()
        {
            return;
        }
        let data = std::slice::from_raw_parts(p, n as usize).to_vec();
        xlib::XFree(p as *mut _);
        for msg in data.split(|b| *b == 0) {
            let msg = String::from_utf8_lossy(msg).trim().to_string();
            if msg.is_empty() {
                continue;
            }
            // bare words are shortcuts: "lock" -> trwm.lock()
            let src = if msg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { format!("trwm.{msg}()") } else { msg };
            match self.cfg.eval(&src) {
                Ok(cmds) => {
                    for c in cmds {
                        self.exec(c);
                    }
                }
                Err(e) => self.show_error(format!("msg: {e}")),
            }
        }
    }

    unsafe fn updatetitle(&mut self, w: Window) {
        let name = self
            .gettextprop(w, self.na(Net::WMName))
            .or_else(|| self.gettextprop(w, xlib::XA_WM_NAME))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "broken".into());
        self.clients.get_mut(&w).unwrap().name = name;
    }

    unsafe fn updatewindowtype(&mut self, w: Window) {
        let state = self.getatomprop(w, self.na(Net::WMState));
        let wtype = self.getatomprop(w, self.na(Net::WMWindowType));
        if state == self.na(Net::WMFullscreen) {
            self.setfullscreen(w, true);
        }
        if wtype == self.na(Net::WMWindowTypeDialog) {
            self.clients.get_mut(&w).unwrap().isfloating = true;
        }
    }

    unsafe fn updatewmhints(&mut self, w: Window) {
        let wmh = xlib::XGetWMHints(self.dpy, w);
        if wmh.is_null() {
            return;
        }
        let is_sel = self.mons[self.selmon].sel == Some(w);
        let c = self.clients.get_mut(&w).unwrap();
        if is_sel && (*wmh).flags & xlib::XUrgencyHint != 0 {
            (*wmh).flags &= !xlib::XUrgencyHint;
            xlib::XSetWMHints(self.dpy, w, wmh);
        } else {
            c.isurgent = (*wmh).flags & xlib::XUrgencyHint != 0;
        }
        c.neverfocus = if (*wmh).flags & xlib::InputHint != 0 { (*wmh).input == 0 } else { false };
        xlib::XFree(wmh as *mut _);
    }

    unsafe fn updatesizehints(&mut self, w: Window) {
        let mut size: xlib::XSizeHints = std::mem::zeroed();
        let mut msize: c_long = 0;
        if xlib::XGetWMNormalHints(self.dpy, w, &mut size, &mut msize) == 0 {
            size.flags = xlib::PSize; // size is uninitialized, ensure that size.flags aren't used
        }
        let c = self.clients.get_mut(&w).unwrap();
        let f = size.flags;
        if f & xlib::PBaseSize != 0 {
            c.basew = size.base_width;
            c.baseh = size.base_height;
        } else if f & xlib::PMinSize != 0 {
            c.basew = size.min_width;
            c.baseh = size.min_height;
        } else {
            c.basew = 0;
            c.baseh = 0;
        }
        if f & xlib::PResizeInc != 0 {
            c.incw = size.width_inc;
            c.inch = size.height_inc;
        } else {
            c.incw = 0;
            c.inch = 0;
        }
        if f & xlib::PMaxSize != 0 {
            c.maxw = size.max_width;
            c.maxh = size.max_height;
        } else {
            c.maxw = 0;
            c.maxh = 0;
        }
        if f & xlib::PMinSize != 0 {
            c.minw = size.min_width;
            c.minh = size.min_height;
        } else if f & xlib::PBaseSize != 0 {
            c.minw = size.base_width;
            c.minh = size.base_height;
        } else {
            c.minw = 0;
            c.minh = 0;
        }
        if f & xlib::PAspect != 0 {
            c.mina = size.min_aspect.y as f32 / size.min_aspect.x.max(1) as f32;
            c.maxa = size.max_aspect.x as f32 / size.max_aspect.y.max(1) as f32;
        } else {
            c.mina = 0.0;
            c.maxa = 0.0;
        }
        c.isfixed = c.maxw != 0 && c.maxh != 0 && c.maxw == c.minw && c.maxh == c.minh;
        c.hintsvalid = true;
    }

    unsafe fn updateclientlist(&self) {
        xlib::XDeleteProperty(self.dpy, self.root, self.na(Net::ClientList));
        for m in &self.mons {
            for w in &m.clients {
                xlib::XChangeProperty(self.dpy, self.root, self.na(Net::ClientList), xlib::XA_WINDOW, 32, xlib::PropModeAppend, w as *const Window as *const c_uchar, 1);
            }
        }
    }

    // -----------------------------------------------------------------------
    // configure
    // -----------------------------------------------------------------------

    unsafe fn configure(&self, w: Window) {
        let c = &self.clients[&w];
        let mut ev: xlib::XEvent = std::mem::zeroed();
        let ce = &mut ev.configure;
        ce.type_ = xlib::ConfigureNotify;
        ce.display = self.dpy;
        ce.event = w;
        ce.window = w;
        ce.x = c.x;
        ce.y = c.y;
        ce.width = c.w;
        ce.height = c.h;
        ce.border_width = c.bw;
        ce.above = 0;
        ce.override_redirect = xlib::False;
        xlib::XSendEvent(self.dpy, w, xlib::False, xlib::StructureNotifyMask, &mut ev);
    }

    unsafe fn configurenotify(&mut self, e: &xlib::XConfigureEvent) {
        if e.window != self.root {
            return;
        }
        let dirty = self.sw != e.width || self.sh != e.height;
        self.sw = e.width;
        self.sh = e.height;
        if self.updategeom() || dirty {
            self.drw.resize(self.sw as u32, self.bh as u32);
            for m in 0..self.mons.len() {
                for w in self.mons[m].clients.clone() {
                    if self.clients[&w].isfullscreen {
                        let mm = &self.mons[m];
                        let (x, y, ww, hh) = (mm.mx, mm.my, mm.mw, mm.mh);
                        self.resizeclient(w, x, y, ww, hh);
                    }
                }
                let mm = &self.mons[m];
                xlib::XMoveResizeWindow(self.dpy, mm.barwin, mm.wx, mm.by, mm.ww as c_uint, self.bh as c_uint);
            }
            self.focus(None);
            self.arrange(None);
        }
    }

    unsafe fn configurerequest(&mut self, e: &xlib::XConfigureRequestEvent) {
        let w = e.window;
        if self.clients.contains_key(&w) {
            let vm = e.value_mask as c_uint;
            let floating_mode = self.clients[&w].isfloating || !self.arranges(self.selmon);
            if vm & xlib::CWBorderWidth as c_uint != 0 {
                self.clients.get_mut(&w).unwrap().bw = e.border_width;
            } else if floating_mode {
                let visible = self.is_visible(w);
                let c = self.clients.get_mut(&w).unwrap();
                let m = &self.mons[c.mon];
                if vm & xlib::CWX as c_uint != 0 {
                    c.oldx = c.x;
                    c.x = m.mx + e.x;
                }
                if vm & xlib::CWY as c_uint != 0 {
                    c.oldy = c.y;
                    c.y = m.my + e.y;
                }
                if vm & xlib::CWWidth as c_uint != 0 {
                    c.oldw = c.w;
                    c.w = e.width;
                }
                if vm & xlib::CWHeight as c_uint != 0 {
                    c.oldh = c.h;
                    c.h = e.height;
                }
                if c.x + c.w > m.mx + m.mw && c.isfloating {
                    c.x = m.mx + (m.mw / 2 - c.width() / 2); // center in x direction
                }
                if c.y + c.h > m.my + m.mh && c.isfloating {
                    c.y = m.my + (m.mh / 2 - c.height() / 2); // center in y direction
                }
                let (x, y, cw, ch) = (c.x, c.y, c.w, c.h);
                if vm & (xlib::CWX | xlib::CWY) as c_uint != 0 && vm & (xlib::CWWidth | xlib::CWHeight) as c_uint == 0 {
                    self.configure(w);
                }
                if visible {
                    xlib::XMoveResizeWindow(self.dpy, w, x, y, cw as c_uint, ch as c_uint);
                }
            } else {
                self.configure(w);
            }
        } else {
            let mut wc = xlib::XWindowChanges {
                x: e.x,
                y: e.y,
                width: e.width,
                height: e.height,
                border_width: e.border_width,
                sibling: e.above,
                stack_mode: e.detail,
            };
            xlib::XConfigureWindow(self.dpy, w, e.value_mask as c_uint, &mut wc);
        }
        xlib::XSync(self.dpy, xlib::False);
    }

    // -----------------------------------------------------------------------
    // manage / unmanage
    // -----------------------------------------------------------------------

    unsafe fn applyrules(&mut self, w: Window) {
        let mut ch: xlib::XClassHint = std::mem::zeroed();
        xlib::XGetClassHint(self.dpy, w, &mut ch);
        let class = if ch.res_class.is_null() { "broken".to_string() } else { CStr::from_ptr(ch.res_class).to_string_lossy().into_owned() };
        let instance = if ch.res_name.is_null() { "broken".to_string() } else { CStr::from_ptr(ch.res_name).to_string_lossy().into_owned() };
        if !ch.res_class.is_null() {
            xlib::XFree(ch.res_class as *mut _);
        }
        if !ch.res_name.is_null() {
            xlib::XFree(ch.res_name as *mut _);
        }
        let name = self.clients[&w].name.clone();
        let mut floating = false;
        let mut tags = 0;
        let mut mon = self.clients[&w].mon;
        for r in &self.cfg.rules {
            let matches = r.title.as_ref().is_none_or(|t| name.contains(t.as_str()))
                && r.class.as_ref().is_none_or(|t| class.contains(t.as_str()))
                && r.instance.as_ref().is_none_or(|t| instance.contains(t.as_str()));
            if matches {
                floating |= r.floating;
                tags |= r.tags;
                if r.monitor >= 0
                    && let Some(i) = self.mons.iter().position(|m| m.num == r.monitor) {
                        mon = i;
                    }
            }
        }
        let mask = self.tagmask();
        let c = self.clients.get_mut(&w).unwrap();
        c.isfloating = floating;
        c.mon = mon;
        let m = &self.mons[mon];
        c.tags = if tags & mask != 0 { tags & mask } else { m.tagset[m.seltags] };
    }

    unsafe fn manage(&mut self, w: Window, wa: &xlib::XWindowAttributes) {
        let mut c = Client {
            win: w,
            x: wa.x,
            y: wa.y,
            w: wa.width,
            h: wa.height,
            oldx: wa.x,
            oldy: wa.y,
            oldw: wa.width,
            oldh: wa.height,
            oldbw: wa.border_width,
            mon: self.selmon,
            ..Default::default()
        };
        let mut trans: Window = 0;
        let has_trans = xlib::XGetTransientForHint(self.dpy, w, &mut trans) != 0 && self.clients.contains_key(&trans);
        if has_trans {
            let t = &self.clients[&trans];
            c.mon = t.mon;
            c.tags = t.tags;
        }
        self.clients.insert(w, c);
        self.updatetitle(w);
        if !has_trans {
            self.applyrules(w);
        }
        // restore tags after `restart`
        if let Some((tags, monnum)) = self.scanning.then(|| self.load_client_tags(w)).flatten() {
            let mask = self.tagmask();
            let mon = self.mons.iter().position(|m| m.num == monnum);
            let c = self.clients.get_mut(&w).unwrap();
            if tags & mask != 0 {
                c.tags = tags & mask;
            }
            if let Some(m) = mon {
                c.mon = m;
            }
        }

        {
            let bw = self.cfg.border_px as i32;
            let c = self.clients.get_mut(&w).unwrap();
            let m = &self.mons[c.mon];
            if c.x + c.width() > m.wx + m.ww {
                c.x = m.wx + m.ww - c.width();
            }
            if c.y + c.height() > m.wy + m.wh {
                c.y = m.wy + m.wh - c.height();
            }
            c.x = c.x.max(m.wx);
            c.y = c.y.max(m.wy);
            c.bw = bw;
        }
        let c = &self.clients[&w];
        let mut wc: xlib::XWindowChanges = std::mem::zeroed();
        wc.border_width = c.bw;
        xlib::XConfigureWindow(self.dpy, w, xlib::CWBorderWidth as c_uint, &mut wc);
        xlib::XSetWindowBorder(self.dpy, w, self.norm.border.pixel);
        self.configure(w); // propagates border_width, if size doesn't change
        self.updatewindowtype(w);
        self.updatesizehints(w);
        self.updatewmhints(w);
        {
            let c = self.clients.get_mut(&w).unwrap();
            // center new floating windows on their monitor
            if c.isfloating || has_trans {
                let m = &self.mons[c.mon];
                c.x = m.wx + (m.ww - c.width()) / 2;
                c.y = m.wy + (m.wh - c.height()) / 2;
            }
        }
        xlib::XSelectInput(self.dpy, w, xlib::EnterWindowMask | xlib::FocusChangeMask | xlib::PropertyChangeMask | xlib::StructureNotifyMask);
        self.grabbuttons(w, false);
        {
            let c = self.clients.get_mut(&w).unwrap();
            if !c.isfloating {
                c.isfloating = has_trans || c.isfixed;
                c.oldstate = c.isfloating;
            }
            if c.isfloating {
                xlib::XRaiseWindow(self.dpy, w);
            }
        }
        let mon = self.clients[&w].mon;
        self.mons[mon].clients.insert(0, w);
        self.mons[mon].stack.insert(0, w);
        xlib::XChangeProperty(self.dpy, self.root, self.na(Net::ClientList), xlib::XA_WINDOW, 32, xlib::PropModeAppend, &w as *const Window as *const c_uchar, 1);
        {
            // some windows require this
            let c = &self.clients[&w];
            xlib::XMoveResizeWindow(self.dpy, w, c.x + 2 * self.sw, c.y, c.w as c_uint, c.h as c_uint);
        }
        self.setclientstate(w, NORMAL_STATE);
        self.save_client_tags(w);
        if mon == self.selmon
            && let Some(s) = self.mons[self.selmon].sel {
                self.unfocus(s, false);
            }
        self.mons[mon].sel = Some(w);
        self.arrange(Some(mon));
        xlib::XMapWindow(self.dpy, w);
        self.focus(None);
    }

    unsafe fn unmanage(&mut self, w: Window, destroyed: bool) {
        let Some(c) = self.clients.get(&w).cloned() else { return };
        let m = c.mon;
        self.detach(w);
        self.detachstack(w);
        if !destroyed {
            let mut wc: xlib::XWindowChanges = std::mem::zeroed();
            wc.border_width = c.oldbw;
            xlib::XGrabServer(self.dpy); // avoid race conditions
            xlib::XSetErrorHandler(Some(xerror_dummy));
            xlib::XSelectInput(self.dpy, w, xlib::NoEventMask);
            xlib::XConfigureWindow(self.dpy, w, xlib::CWBorderWidth as c_uint, &mut wc); // restore border
            xlib::XUngrabButton(self.dpy, xlib::AnyButton as c_uint, xlib::AnyModifier, w);
            self.setclientstate(w, WITHDRAWN_STATE);
            xlib::XSync(self.dpy, xlib::False);
            xlib::XSetErrorHandler(Some(xerror));
            xlib::XUngrabServer(self.dpy);
        }
        self.clients.remove(&w);
        self.focus(None);
        self.updateclientlist();
        self.arrange(Some(m));
    }

    fn detach(&mut self, w: Window) {
        let m = self.clients[&w].mon;
        self.mons[m].clients.retain(|x| *x != w);
    }

    fn detachstack(&mut self, w: Window) {
        let m = self.clients[&w].mon;
        self.mons[m].stack.retain(|x| *x != w);
        if self.mons[m].sel == Some(w) {
            let next = self.mons[m].stack.iter().copied().find(|x| self.is_visible(*x));
            self.mons[m].sel = next;
        }
    }

    fn attach(&mut self, w: Window) {
        let m = self.clients[&w].mon;
        self.mons[m].clients.insert(0, w);
    }

    fn attachstack(&mut self, w: Window) {
        let m = self.clients[&w].mon;
        self.mons[m].stack.insert(0, w);
    }

    // -----------------------------------------------------------------------
    // focus
    // -----------------------------------------------------------------------

    unsafe fn focus(&mut self, c: Option<Window>) {
        let sm = self.selmon;
        let c = match c {
            Some(w) if self.clients.contains_key(&w) && self.is_visible(w) => Some(w),
            _ => self.mons[sm].stack.iter().copied().find(|w| self.is_visible(*w)),
        };
        if let Some(s) = self.mons[self.selmon].sel
            && Some(s) != c {
                self.unfocus(s, false);
            }
        if let Some(w) = c {
            let cm = self.clients[&w].mon;
            if cm != self.selmon {
                self.selmon = cm;
            }
            if self.clients[&w].isurgent {
                self.seturgent(w, false);
            }
            self.detachstack(w);
            self.attachstack(w);
            self.grabbuttons(w, true);
            xlib::XSetWindowBorder(self.dpy, w, self.sel.border.pixel);
            self.setfocus(w);
        } else {
            xlib::XSetInputFocus(self.dpy, self.root, xlib::RevertToPointerRoot, xlib::CurrentTime);
            xlib::XDeleteProperty(self.dpy, self.root, self.na(Net::ActiveWindow));
        }
        let sm = self.selmon;
        self.mons[sm].sel = c;
        self.drawbars();
    }

    unsafe fn unfocus(&mut self, w: Window, setfocus: bool) {
        if !self.clients.contains_key(&w) {
            return;
        }
        self.grabbuttons(w, false);
        xlib::XSetWindowBorder(self.dpy, w, self.norm.border.pixel);
        if setfocus {
            xlib::XSetInputFocus(self.dpy, self.root, xlib::RevertToPointerRoot, xlib::CurrentTime);
            xlib::XDeleteProperty(self.dpy, self.root, self.na(Net::ActiveWindow));
        }
    }

    unsafe fn setfocus(&mut self, w: Window) {
        if !self.clients[&w].neverfocus {
            xlib::XSetInputFocus(self.dpy, w, xlib::RevertToPointerRoot, xlib::CurrentTime);
            xlib::XChangeProperty(self.dpy, self.root, self.na(Net::ActiveWindow), xlib::XA_WINDOW, 32, xlib::PropModeReplace, &w as *const Window as *const c_uchar, 1);
        }
        self.sendevent(w, self.wa(Wm::TakeFocus));
    }

    unsafe fn seturgent(&mut self, w: Window, urg: bool) {
        self.clients.get_mut(&w).unwrap().isurgent = urg;
        let wmh = xlib::XGetWMHints(self.dpy, w);
        if wmh.is_null() {
            return;
        }
        (*wmh).flags = if urg { (*wmh).flags | xlib::XUrgencyHint } else { (*wmh).flags & !xlib::XUrgencyHint };
        xlib::XSetWMHints(self.dpy, w, wmh);
        xlib::XFree(wmh as *mut _);
    }

    unsafe fn focusstack(&mut self, dir: i32) {
        let m = self.selmon;
        let Some(sel) = self.mons[m].sel else { return };
        if self.clients[&sel].isfullscreen && self.cfg.lock_fullscreen {
            return;
        }
        let vis: Vec<Window> = self.mons[m].clients.iter().copied().filter(|w| self.is_visible(*w)).collect();
        let Some(i) = vis.iter().position(|w| *w == sel) else { return };
        let n = vis.len() as i32;
        let next = vis[(i as i32 + dir).rem_euclid(n) as usize];
        self.focus(Some(next));
        self.restack(m);
    }

    // -----------------------------------------------------------------------
    // geometry
    // -----------------------------------------------------------------------

    unsafe fn applysizehints(&mut self, w: Window, x: &mut i32, y: &mut i32, ww: &mut i32, hh: &mut i32, interact: bool) -> bool {
        let bh = self.bh;
        let (sw, sh) = (self.sw, self.sh);
        let arranges = self.arranges(self.clients[&w].mon);
        let resize_hints = self.cfg.resize_hints;
        if !self.clients[&w].hintsvalid {
            self.updatesizehints(w);
        }
        let c = &self.clients[&w];
        let m = &self.mons[c.mon];
        // set minimum possible
        *ww = (*ww).max(1);
        *hh = (*hh).max(1);
        if interact {
            if *x > sw {
                *x = sw - c.width();
            }
            if *y > sh {
                *y = sh - c.height();
            }
            if *x + *ww + 2 * c.bw < 0 {
                *x = 0;
            }
            if *y + *hh + 2 * c.bw < 0 {
                *y = 0;
            }
        } else {
            if *x >= m.wx + m.ww {
                *x = m.wx + m.ww - c.width();
            }
            if *y >= m.wy + m.wh {
                *y = m.wy + m.wh - c.height();
            }
            if *x + *ww + 2 * c.bw <= m.wx {
                *x = m.wx;
            }
            if *y + *hh + 2 * c.bw <= m.wy {
                *y = m.wy;
            }
        }
        *hh = (*hh).max(bh);
        *ww = (*ww).max(bh);
        if resize_hints || c.isfloating || !arranges {
            // see last two sentences in ICCCM 4.1.2.3
            let baseismin = c.basew == c.minw && c.baseh == c.minh;
            if !baseismin {
                // temporarily remove base dimensions
                *ww -= c.basew;
                *hh -= c.baseh;
            }
            // adjust for aspect limits
            if c.mina > 0.0 && c.maxa > 0.0 {
                if c.maxa < *ww as f32 / *hh as f32 {
                    *ww = (*hh as f32 * c.maxa + 0.5) as i32;
                } else if c.mina < *hh as f32 / *ww as f32 {
                    *hh = (*ww as f32 * c.mina + 0.5) as i32;
                }
            }
            if baseismin {
                // increment calculation requires this
                *ww -= c.basew;
                *hh -= c.baseh;
            }
            // adjust for increment value
            if c.incw != 0 {
                *ww -= *ww % c.incw;
            }
            if c.inch != 0 {
                *hh -= *hh % c.inch;
            }
            // restore base dimensions
            *ww = (*ww + c.basew).max(c.minw);
            *hh = (*hh + c.baseh).max(c.minh);
            if c.maxw != 0 {
                *ww = (*ww).min(c.maxw);
            }
            if c.maxh != 0 {
                *hh = (*hh).min(c.maxh);
            }
        }
        *x != c.x || *y != c.y || *ww != c.w || *hh != c.h
    }

    unsafe fn resize(&mut self, w: Window, mut x: i32, mut y: i32, mut ww: i32, mut hh: i32, interact: bool) {
        if self.applysizehints(w, &mut x, &mut y, &mut ww, &mut hh, interact) {
            self.resizeclient(w, x, y, ww, hh);
        }
    }

    unsafe fn resizeclient(&mut self, w: Window, x: i32, y: i32, ww: i32, hh: i32) {
        let c = self.clients.get_mut(&w).unwrap();
        c.oldx = c.x;
        c.x = x;
        c.oldy = c.y;
        c.y = y;
        c.oldw = c.w;
        c.w = ww;
        c.oldh = c.h;
        c.h = hh;
        let mut wc = xlib::XWindowChanges { x, y, width: ww, height: hh, border_width: c.bw, sibling: 0, stack_mode: 0 };
        xlib::XConfigureWindow(self.dpy, w, (xlib::CWX | xlib::CWY | xlib::CWWidth | xlib::CWHeight | xlib::CWBorderWidth) as c_uint, &mut wc);
        self.configure(w);
        xlib::XSync(self.dpy, xlib::False);
    }

    unsafe fn setfullscreen(&mut self, w: Window, fullscreen: bool) {
        let isfs = self.clients[&w].isfullscreen;
        if fullscreen && !isfs {
            let fs = self.na(Net::WMFullscreen);
            xlib::XChangeProperty(self.dpy, w, self.na(Net::WMState), xlib::XA_ATOM, 32, xlib::PropModeReplace, &fs as *const Atom as *const c_uchar, 1);
            let c = self.clients.get_mut(&w).unwrap();
            c.isfullscreen = true;
            c.oldstate = c.isfloating;
            c.oldbw = c.bw;
            c.bw = 0;
            c.isfloating = true;
            let m = &self.mons[c.mon];
            let (x, y, mw, mh) = (m.mx, m.my, m.mw, m.mh);
            self.resizeclient(w, x, y, mw, mh);
            xlib::XRaiseWindow(self.dpy, w);
        } else if !fullscreen && isfs {
            xlib::XChangeProperty(self.dpy, w, self.na(Net::WMState), xlib::XA_ATOM, 32, xlib::PropModeReplace, ptr::null(), 0);
            let c = self.clients.get_mut(&w).unwrap();
            c.isfullscreen = false;
            c.isfloating = c.oldstate;
            c.bw = c.oldbw;
            c.x = c.oldx;
            c.y = c.oldy;
            c.w = c.oldw;
            c.h = c.oldh;
            let (x, y, ww, hh, m) = (c.x, c.y, c.w, c.h, c.mon);
            self.resizeclient(w, x, y, ww, hh);
            self.arrange(Some(m));
        }
    }

    // -----------------------------------------------------------------------
    // arranging
    // -----------------------------------------------------------------------

    unsafe fn arrange(&mut self, m: Option<usize>) {
        let range: Vec<usize> = match m {
            Some(m) => vec![m],
            None => (0..self.mons.len()).collect(),
        };
        for &i in &range {
            self.showhide(i);
        }
        for &i in &range {
            self.arrangemon(i);
        }
        match m {
            Some(m) => self.restack(m),
            None => {
                self.drawbars();
            }
        }
    }

    unsafe fn arrangemon(&mut self, m: usize) {
        let l = self.mons[m].lt[self.mons[m].sellt];
        let kind = self.cfg.layouts[l].kind;
        self.mons[m].ltsymbol = self.cfg.layouts[l].symbol.clone();
        match kind {
            LayoutKind::Tile => self.tile(m),
            LayoutKind::Monocle => self.monocle(m),
            LayoutKind::BStack => self.bstack(m),
            LayoutKind::Grid => self.grid(m),
            LayoutKind::Deck => self.deck(m),
            LayoutKind::Float => {}
        }
    }

    unsafe fn showhide(&mut self, m: usize) {
        let stack = self.mons[m].stack.clone();
        let arranges = self.arranges(m);
        // show clients top down
        for &w in &stack {
            if self.is_visible(w) {
                let c = &self.clients[&w];
                xlib::XMoveWindow(self.dpy, w, c.x, c.y);
                if (!arranges || c.isfloating) && !c.isfullscreen {
                    let (x, y, ww, hh) = (c.x, c.y, c.w, c.h);
                    self.resize(w, x, y, ww, hh, false);
                }
            }
        }
        // hide clients bottom up
        for &w in stack.iter().rev() {
            if !self.is_visible(w) {
                let c = &self.clients[&w];
                xlib::XMoveWindow(self.dpy, w, c.width() * -2, c.y);
            }
        }
    }

    unsafe fn restack(&mut self, m: usize) {
        self.drawbar(m);
        let Some(sel) = self.mons[m].sel else { return };
        if self.clients[&sel].isfloating || !self.arranges(m) {
            xlib::XRaiseWindow(self.dpy, sel);
        }
        if self.arranges(m) {
            let mut wc: xlib::XWindowChanges = std::mem::zeroed();
            wc.stack_mode = xlib::Below;
            wc.sibling = self.mons[m].barwin;
            for w in self.mons[m].stack.clone() {
                if !self.clients[&w].isfloating && self.is_visible(w) {
                    xlib::XConfigureWindow(self.dpy, w, (xlib::CWSibling | xlib::CWStackMode) as c_uint, &mut wc);
                    wc.sibling = w;
                }
            }
        }
        xlib::XSync(self.dpy, xlib::False);
        let mut ev: xlib::XEvent = std::mem::zeroed();
        while xlib::XCheckMaskEvent(self.dpy, xlib::EnterWindowMask, &mut ev) != 0 {}
    }

    /// outer h, outer v, inner h, inner v, number of tiled clients
    fn getgaps(&self, m: usize) -> (i32, i32, i32, i32, usize) {
        let mon = &self.mons[m];
        let n = self.tiled(m).len();
        let g = mon.gaps;
        let on = g.enabled;
        let mut oe = on;
        if g.smart && n == 1 {
            oe = false; // outer gaps disabled when only one client
        }
        let o = |v: i32, e: bool| if e { v } else { 0 };
        (o(g.oh, oe), o(g.ov, oe), o(g.ih, on), o(g.iv, on), n)
    }

    unsafe fn place(&mut self, w: Window, x: i32, y: i32, ww: i32, hh: i32) {
        let bw = self.clients[&w].bw;
        self.resize(w, x, y, ww - 2 * bw, hh - 2 * bw, false);
    }

    unsafe fn tile(&mut self, m: usize) {
        let (oh, ov, ih, iv, n) = self.getgaps(m);
        if n == 0 {
            return;
        }
        let tiled = self.tiled(m);
        let mon = &self.mons[m];
        let nm = (mon.nmaster.max(0) as usize).min(n);
        let (mx, mut my) = (mon.wx + ov, mon.wy + oh);
        let mut sx = mx;
        let mut sy = my;
        let mh = mon.wh - 2 * oh - ih * (nm as i32 - 1).max(0);
        let sh = mon.wh - 2 * oh - ih * (n as i32 - nm as i32 - 1).max(0);
        let mut mw = mon.ww - 2 * ov;
        let mut sw = mw;
        if nm > 0 && n > nm {
            sw = ((mw - iv) as f32 * (1.0 - mon.mfact)) as i32;
            mw = mw - iv - sw;
            sx = mx + mw + iv;
        }
        let (mfacts, sfacts) = (nm.max(1) as i32, (n - nm).max(1) as i32);
        let (mrest, srest) = (mh - (mh / mfacts) * mfacts, sh - (sh / sfacts) * sfacts);
        for (i, w) in tiled.into_iter().enumerate() {
            if i < nm {
                let hh = mh / mfacts + if (i as i32) < mrest { 1 } else { 0 };
                self.place(w, mx, my, mw, hh);
                my += self.clients[&w].height() + ih;
            } else {
                let j = (i - nm) as i32;
                let hh = sh / sfacts + if j < srest { 1 } else { 0 };
                self.place(w, sx, sy, sw, hh);
                sy += self.clients[&w].height() + ih;
            }
        }
    }

    unsafe fn bstack(&mut self, m: usize) {
        let (oh, ov, ih, iv, n) = self.getgaps(m);
        if n == 0 {
            return;
        }
        let tiled = self.tiled(m);
        let mon = &self.mons[m];
        let nm = (mon.nmaster.max(0) as usize).min(n);
        let (mut mx, my) = (mon.wx + ov, mon.wy + oh);
        let mut sx = mx;
        let mut sy = my;
        let mut mh = mon.wh - 2 * oh;
        let mut sh = mh;
        let mw = mon.ww - 2 * ov - iv * (nm as i32 - 1).max(0);
        let sw = mon.ww - 2 * ov - iv * (n as i32 - nm as i32 - 1).max(0);
        if nm > 0 && n > nm {
            sh = ((mh - ih) as f32 * (1.0 - mon.mfact)) as i32;
            mh = mh - ih - sh;
            sx = mx;
            sy = my + mh + ih;
        }
        let (mfacts, sfacts) = (nm.max(1) as i32, (n - nm).max(1) as i32);
        let (mrest, srest) = (mw - (mw / mfacts) * mfacts, sw - (sw / sfacts) * sfacts);
        for (i, w) in tiled.into_iter().enumerate() {
            if i < nm {
                let ww = mw / mfacts + if (i as i32) < mrest { 1 } else { 0 };
                self.place(w, mx, my, ww, mh);
                mx += self.clients[&w].width() + iv;
            } else {
                let j = (i - nm) as i32;
                let ww = sw / sfacts + if j < srest { 1 } else { 0 };
                self.place(w, sx, sy, ww, sh);
                sx += self.clients[&w].width() + iv;
            }
        }
    }

    unsafe fn deck(&mut self, m: usize) {
        let (oh, ov, ih, iv, n) = self.getgaps(m);
        if n == 0 {
            return;
        }
        let tiled = self.tiled(m);
        let mon = &self.mons[m];
        let nm = (mon.nmaster.max(0) as usize).min(n);
        if n > nm {
            self.mons[m].ltsymbol = format!("[{}]", n - nm);
        }
        let mon = &self.mons[m];
        let (mx, mut my) = (mon.wx + ov, mon.wy + oh);
        let mh = mon.wh - 2 * oh - ih * (nm as i32 - 1).max(0);
        let sh = mon.wh - 2 * oh;
        let mut mw = mon.ww - 2 * ov;
        let mut sw = mw;
        let mut sx = mx;
        let sy = my;
        if nm > 0 && n > nm {
            sw = ((mw - iv) as f32 * (1.0 - mon.mfact)) as i32;
            mw = mw - iv - sw;
            sx = mx + mw + iv;
        }
        let mfacts = nm.max(1) as i32;
        let mrest = mh - (mh / mfacts) * mfacts;
        for (i, w) in tiled.into_iter().enumerate() {
            if i < nm {
                let hh = mh / mfacts + if (i as i32) < mrest { 1 } else { 0 };
                self.place(w, mx, my, mw, hh);
                my += self.clients[&w].height() + ih;
            } else {
                self.place(w, sx, sy, sw, sh);
            }
        }
    }

    unsafe fn grid(&mut self, m: usize) {
        let (oh, ov, ih, iv, n) = self.getgaps(m);
        if n == 0 {
            return;
        }
        let tiled = self.tiled(m);
        let mon = &self.mons[m];
        let mut cols = 1;
        while cols * cols < n {
            cols += 1;
        }
        let rows = n.div_ceil(cols);
        let (cols, rows) = (cols as i32, rows as i32);
        let cw = (mon.ww - 2 * ov - iv * (cols - 1)) / cols;
        let ch = (mon.wh - 2 * oh - ih * (rows - 1)) / rows;
        let (x0, y0) = (mon.wx + ov, mon.wy + oh);
        for (i, w) in tiled.into_iter().enumerate() {
            let (c, r) = (i as i32 % cols, i as i32 / cols);
            self.place(w, x0 + c * (cw + iv), y0 + r * (ch + ih), cw, ch);
        }
    }

    unsafe fn monocle(&mut self, m: usize) {
        let n = self.mons[m].clients.iter().filter(|w| self.is_visible(**w)).count();
        if n > 0 {
            self.mons[m].ltsymbol = format!("[{n}]");
        }
        let mon = &self.mons[m];
        let (x, y, ww, hh) = (mon.wx, mon.wy, mon.ww, mon.wh);
        for w in self.tiled(m) {
            self.place(w, x, y, ww, hh);
        }
    }

    // -----------------------------------------------------------------------
    // tag / layout commands
    // -----------------------------------------------------------------------

    unsafe fn view(&mut self, t: Option<u32>) {
        let m = self.selmon;
        let mask = self.tagmask();
        let mon = &mut self.mons[m];
        if let Some(t) = t
            && t & mask == mon.tagset[mon.seltags] {
                return;
            }
        mon.seltags ^= 1; // toggle sel tagset
        if let Some(t) = t
            && t & mask != 0 {
                mon.tagset[mon.seltags] = t & mask;
            }
        self.update_desktop_props();
        self.focus(None);
        self.arrange(Some(m));
    }

    unsafe fn toggleview(&mut self, t: u32) {
        let m = self.selmon;
        let mask = self.tagmask();
        let mon = &mut self.mons[m];
        let new = mon.tagset[mon.seltags] ^ (t & mask);
        if new != 0 {
            mon.tagset[mon.seltags] = new;
            self.update_desktop_props();
            self.focus(None);
            self.arrange(Some(m));
        }
    }

    unsafe fn tag(&mut self, t: u32) {
        let mask = self.tagmask();
        if let Some(s) = self.mons[self.selmon].sel
            && t & mask != 0 {
                self.clients.get_mut(&s).unwrap().tags = t & mask;
                self.save_client_tags(s);
                self.focus(None);
                self.arrange(Some(self.selmon));
            }
    }

    unsafe fn toggletag(&mut self, t: u32) {
        let mask = self.tagmask();
        if let Some(s) = self.mons[self.selmon].sel {
            let new = self.clients[&s].tags ^ (t & mask);
            if new != 0 {
                self.clients.get_mut(&s).unwrap().tags = new;
                self.save_client_tags(s);
                self.focus(None);
                self.arrange(Some(self.selmon));
            }
        }
    }

    unsafe fn sendmon(&mut self, w: Window, m: usize) {
        if self.clients[&w].mon == m {
            return;
        }
        self.unfocus(w, true);
        self.detach(w);
        self.detachstack(w);
        let tags = self.mons[m].tagset[self.mons[m].seltags];
        {
            let c = self.clients.get_mut(&w).unwrap();
            c.mon = m;
            c.tags = tags; // assign tags of target monitor
        }
        self.attach(w);
        self.attachstack(w);
        self.save_client_tags(w);
        self.focus(None);
        self.arrange(None);
    }

    unsafe fn setlayout(&mut self, l: LayoutSel) {
        let m = self.selmon;
        let idx = match l {
            LayoutSel::Toggle => None,
            LayoutSel::Index(i) => Some(i),
            LayoutSel::Name(n) => self.cfg.layouts.iter().position(|l| {
                l.symbol == n
                    || format!("{:?}", l.kind).eq_ignore_ascii_case(&n)
                    || (n.eq_ignore_ascii_case("floating") && l.kind == LayoutKind::Float)
            }),
        };
        let mon = &mut self.mons[m];
        let cur = mon.lt[mon.sellt];
        if idx.is_none() || idx != Some(cur) {
            mon.sellt ^= 1;
        }
        if let Some(i) = idx
            && i < self.cfg.layouts.len() {
                mon.lt[mon.sellt] = i;
            }
        mon.ltsymbol = self.cfg.layouts[mon.lt[mon.sellt]].symbol.clone();
        if mon.sel.is_some() {
            self.arrange(Some(m));
        } else {
            self.drawbar(m);
        }
    }

    unsafe fn setmfact(&mut self, f: f32) {
        let m = self.selmon;
        if !self.arranges(m) {
            return;
        }
        // values >= 1.0 set the factor absolutely (1.55 -> 0.55), like dwm
        let mon = &mut self.mons[m];
        let new = if f < 1.0 { f + mon.mfact } else { f - 1.0 };
        if !(0.05..=0.95).contains(&new) {
            return;
        }
        mon.mfact = new;
        self.arrange(Some(m));
    }

    unsafe fn zoom(&mut self) {
        let m = self.selmon;
        let Some(sel) = self.mons[m].sel else { return };
        if !self.arranges(m) || self.clients[&sel].isfloating {
            return;
        }
        let tiled = self.tiled(m);
        let mut c = sel;
        if tiled.first() == Some(&sel) {
            match tiled.get(1) {
                Some(&next) => c = next,
                None => return,
            }
        }
        // pop: move to the master area
        self.detach(c);
        self.attach(c);
        self.focus(Some(c));
        self.arrange(Some(m));
    }

    unsafe fn killclient(&mut self) {
        let Some(sel) = self.mons[self.selmon].sel else { return };
        if !self.sendevent(sel, self.wa(Wm::Delete)) {
            xlib::XGrabServer(self.dpy);
            xlib::XSetErrorHandler(Some(xerror_dummy));
            xlib::XSetCloseDownMode(self.dpy, xlib::DestroyAll);
            xlib::XKillClient(self.dpy, sel);
            xlib::XSync(self.dpy, xlib::False);
            xlib::XSetErrorHandler(Some(xerror));
            xlib::XUngrabServer(self.dpy);
        }
    }

    unsafe fn togglefloating(&mut self) {
        let m = self.selmon;
        let Some(sel) = self.mons[m].sel else { return };
        let c = self.clients.get_mut(&sel).unwrap();
        if c.isfullscreen {
            return; // no support for fullscreen windows
        }
        c.isfloating = !c.isfloating || c.isfixed;
        if c.isfloating {
            let (x, y, w, h) = (c.x, c.y, c.w, c.h);
            self.resize(sel, x, y, w, h, false);
        }
        self.arrange(Some(m));
    }

    // -----------------------------------------------------------------------
    // mouse move / resize
    // -----------------------------------------------------------------------

    unsafe fn mouse_loop(&mut self, cursor: usize, mut on_motion: impl FnMut(&mut Self, &xlib::XMotionEvent)) {
        if xlib::XGrabPointer(self.dpy, self.root, xlib::False, MOUSE_MASK as c_uint, xlib::GrabModeAsync, xlib::GrabModeAsync, 0, self.cursors[cursor], xlib::CurrentTime)
            != xlib::GrabSuccess
        {
            return;
        }
        let frame = 1000 / self.cfg.refresh_rate.max(1) as xlib::Time;
        let mut last: xlib::Time = 0;
        loop {
            let mut ev: xlib::XEvent = std::mem::zeroed();
            xlib::XMaskEvent(self.dpy, MOUSE_MASK | xlib::ExposureMask | xlib::SubstructureRedirectMask, &mut ev);
            match ev.get_type() {
                xlib::ConfigureRequest | xlib::Expose | xlib::MapRequest => self.handle(&mut ev),
                xlib::MotionNotify => {
                    if ev.motion.time - last <= frame {
                        continue;
                    }
                    last = ev.motion.time;
                    on_motion(self, &ev.motion);
                }
                xlib::ButtonRelease => break,
                _ => {}
            }
        }
        xlib::XUngrabPointer(self.dpy, xlib::CurrentTime);
    }

    unsafe fn after_mouse(&mut self, w: Window) {
        let c = &self.clients[&w];
        let m = self.recttomon(c.x, c.y, c.w, c.h);
        if m != self.selmon {
            self.sendmon(w, m);
            self.selmon = m;
            self.focus(None);
        }
    }

    unsafe fn movemouse(&mut self) {
        let Some(sel) = self.mons[self.selmon].sel else { return };
        if self.clients[&sel].isfullscreen {
            return; // no support moving fullscreen windows by mouse
        }
        self.restack(self.selmon);
        let (ocx, ocy) = (self.clients[&sel].x, self.clients[&sel].y);
        let Some((px, py)) = self.getrootptr() else { return };
        self.mouse_loop(2, |wm, e| {
            let mut nx = ocx + (e.x_root - px);
            let mut ny = ocy + (e.y_root - py);
            let snap = wm.cfg.snap as i32;
            let m = &wm.mons[wm.selmon];
            let c = &wm.clients[&sel];
            if (m.wx - nx).abs() < snap {
                nx = m.wx;
            } else if ((m.wx + m.ww) - (nx + c.width())).abs() < snap {
                nx = m.wx + m.ww - c.width();
            }
            if (m.wy - ny).abs() < snap {
                ny = m.wy;
            } else if ((m.wy + m.wh) - (ny + c.height())).abs() < snap {
                ny = m.wy + m.wh - c.height();
            }
            let (cw, ch, cx, cy, floating) = (c.w, c.h, c.x, c.y, c.isfloating);
            if !floating && wm.arranges(wm.selmon) && ((nx - cx).abs() > snap || (ny - cy).abs() > snap) {
                wm.togglefloating();
            }
            if !wm.arranges(wm.selmon) || wm.clients[&sel].isfloating {
                wm.resize(sel, nx, ny, cw, ch, true);
            }
        });
        self.after_mouse(sel);
    }

    unsafe fn resizemouse(&mut self) {
        let Some(sel) = self.mons[self.selmon].sel else { return };
        if self.clients[&sel].isfullscreen {
            return; // no support resizing fullscreen windows by mouse
        }
        self.restack(self.selmon);
        let (ocx, ocy) = (self.clients[&sel].x, self.clients[&sel].y);
        {
            let c = &self.clients[&sel];
            xlib::XWarpPointer(self.dpy, 0, sel, 0, 0, 0, 0, c.w + c.bw - 1, c.h + c.bw - 1);
        }
        self.mouse_loop(1, |wm, e| {
            let c = &wm.clients[&sel];
            let nw = (e.x_root - ocx - 2 * c.bw + 1).max(1);
            let nh = (e.y_root - ocy - 2 * c.bw + 1).max(1);
            let snap = wm.cfg.snap as i32;
            let cm = &wm.mons[c.mon];
            let sm = &wm.mons[wm.selmon];
            let inside = cm.wx + nw >= sm.wx && cm.wx + nw <= sm.wx + sm.ww && cm.wy + nh >= sm.wy && cm.wy + nh <= sm.wy + sm.wh;
            let (cx, cy, cw, ch, floating) = (c.x, c.y, c.w, c.h, c.isfloating);
            if inside && !floating && wm.arranges(wm.selmon) && ((nw - cw).abs() > snap || (nh - ch).abs() > snap) {
                wm.togglefloating();
            }
            if !wm.arranges(wm.selmon) || wm.clients[&sel].isfloating {
                wm.resize(sel, cx, cy, nw, nh, true);
            }
        });
        {
            let c = &self.clients[&sel];
            xlib::XWarpPointer(self.dpy, 0, sel, 0, 0, 0, 0, c.w + c.bw - 1, c.h + c.bw - 1);
        }
        let mut ev: xlib::XEvent = std::mem::zeroed();
        while xlib::XCheckMaskEvent(self.dpy, xlib::EnterWindowMask, &mut ev) != 0 {}
        self.after_mouse(sel);
    }
}

unsafe extern "C" fn xerror_dummy(_: *mut xlib::Display, _: *mut xlib::XErrorEvent) -> c_int {
    0
}

/// `trwm msg <lua>`: send a command to the running window manager.
pub fn send_message(msg: &str) -> Result<(), String> {
    unsafe {
        let dpy = xlib::XOpenDisplay(ptr::null());
        if dpy.is_null() {
            return Err("cannot open display".into());
        }
        let root = xlib::XDefaultRootWindow(dpy);
        let name = CString::new(CMD_ATOM).unwrap();
        let atom = xlib::XInternAtom(dpy, name.as_ptr(), xlib::False);
        let utf8n = CString::new("UTF8_STRING").unwrap();
        let utf8 = xlib::XInternAtom(dpy, utf8n.as_ptr(), xlib::False);
        let mut data = msg.as_bytes().to_vec();
        data.push(0);
        xlib::XChangeProperty(dpy, root, atom, utf8, 8, xlib::PropModeAppend, data.as_ptr(), data.len() as c_int);
        xlib::XSync(dpy, xlib::False);
        xlib::XCloseDisplay(dpy);
    }
    Ok(())
}

/// Path of the running binary. After `cargo install` replaces it, Linux reports
/// "/path/trwm (deleted)"; strip that so `restart` runs the new build.
pub fn exe_path() -> PathBuf {
    match std::env::current_exe() {
        Ok(p) => {
            let s = p.to_string_lossy();
            match s.strip_suffix(" (deleted)") {
                Some(stripped) => PathBuf::from(stripped),
                None => p,
            }
        }
        Err(_) => PathBuf::from("trwm"),
    }
}
