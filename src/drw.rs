//! Tiny Xft drawing helper (the trwm take on dwm's drw.c).
//!
//! Holds an offscreen pixmap the bar (and the locker) render into, a list of
//! fonts with per-glyph fallback, and colour allocation.

use std::ffi::CString;
use std::os::raw::{c_int, c_uint};
use std::ptr;

use x11::xft;
use x11::xlib;
use x11::xrender::XGlyphInfo;

pub type Color = xft::XftColor;

/// fg, bg, border
#[derive(Clone, Copy)]
pub struct Scheme {
    pub fg: Color,
    pub bg: Color,
    pub border: Color,
}

pub struct Font {
    xfont: *mut xft::XftFont,
    pub h: u32,
}

pub struct Drw {
    pub dpy: *mut xlib::Display,
    pub screen: c_int,
    pub root: xlib::Window,
    pub w: u32,
    pub h: u32,
    pub drawable: xlib::Drawable,
    gc: xlib::GC,
    pub fonts: Vec<Font>,
    pub fg: Color,
    pub bg: Color,
}

impl Drw {
    pub fn new(dpy: *mut xlib::Display, screen: c_int, root: xlib::Window, w: u32, h: u32) -> Drw {
        unsafe {
            let depth = xlib::XDefaultDepth(dpy, screen) as c_uint;
            let drawable = xlib::XCreatePixmap(dpy, root, w, h, depth);
            let gc = xlib::XCreateGC(dpy, root, 0, ptr::null_mut());
            xlib::XSetLineAttributes(dpy, gc, 1, xlib::LineSolid, xlib::CapButt, xlib::JoinMiter);
            let zero: Color = std::mem::zeroed();
            Drw { dpy, screen, root, w, h, drawable, gc, fonts: Vec::new(), fg: zero, bg: zero }
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        unsafe {
            self.w = w;
            self.h = h;
            if self.drawable != 0 {
                xlib::XFreePixmap(self.dpy, self.drawable);
            }
            let depth = xlib::XDefaultDepth(self.dpy, self.screen) as c_uint;
            self.drawable = xlib::XCreatePixmap(self.dpy, self.root, w, h, depth);
        }
    }

    /// Load a list of fontconfig names. Returns false if none could be loaded.
    pub fn set_fonts(&mut self, names: &[String]) -> bool {
        let mut new = Vec::new();
        for name in names {
            let Ok(c) = CString::new(name.as_str()) else { continue };
            unsafe {
                let f = xft::XftFontOpenName(self.dpy, self.screen, c.as_ptr());
                if f.is_null() {
                    eprintln!("trwm: cannot load font '{name}'");
                    continue;
                }
                let h = ((*f).ascent + (*f).descent) as u32;
                new.push(Font { xfont: f, h });
            }
        }
        if new.is_empty() {
            return false;
        }
        self.free_fonts();
        self.fonts = new;
        true
    }

    fn free_fonts(&mut self) {
        for f in self.fonts.drain(..) {
            unsafe { xft::XftFontClose(self.dpy, f.xfont) };
        }
    }

    pub fn font_h(&self) -> u32 {
        self.fonts.first().map(|f| f.h).unwrap_or(12)
    }

    pub fn color(&self, name: &str) -> Color {
        unsafe {
            let mut c: Color = std::mem::zeroed();
            let cname = CString::new(name).unwrap_or_else(|_| CString::new("#000000").unwrap());
            let vis = xlib::XDefaultVisual(self.dpy, self.screen);
            let cmap = xlib::XDefaultColormap(self.dpy, self.screen);
            if xft::XftColorAllocName(self.dpy, vis, cmap, cname.as_ptr(), &mut c) == 0 {
                eprintln!("trwm: cannot allocate color '{name}'");
                let black = CString::new("#000000").unwrap();
                xft::XftColorAllocName(self.dpy, vis, cmap, black.as_ptr(), &mut c);
            }
            // make sure the colour is fully opaque even on 32-bit visuals
            c.pixel |= 0xff << 24;
            c
        }
    }

    pub fn scheme(&self, fg: &str, bg: &str, border: &str) -> Scheme {
        Scheme { fg: self.color(fg), bg: self.color(bg), border: self.color(border) }
    }

    pub fn set_scheme(&mut self, s: &Scheme) {
        self.fg = s.fg;
        self.bg = s.bg;
    }

    /// Index of the first font that has a glyph for `ch`.
    fn font_for(&self, ch: char) -> usize {
        for (i, f) in self.fonts.iter().enumerate() {
            if unsafe { xft::XftCharExists(self.dpy, f.xfont, ch as u32) } != 0 {
                return i;
            }
        }
        0
    }

    /// Split text into runs that use the same font.
    fn runs<'a>(&self, text: &'a str) -> Vec<(usize, &'a str)> {
        let mut out: Vec<(usize, &str)> = Vec::new();
        let mut start = 0;
        let mut cur = None;
        for (i, ch) in text.char_indices() {
            let f = self.font_for(ch);
            match cur {
                None => cur = Some(f),
                Some(c) if c != f => {
                    out.push((c, &text[start..i]));
                    start = i;
                    cur = Some(f);
                }
                _ => {}
            }
        }
        if let Some(c) = cur {
            out.push((c, &text[start..]));
        }
        out
    }

    fn run_width(&self, font: usize, s: &str) -> u32 {
        if s.is_empty() || self.fonts.is_empty() {
            return 0;
        }
        unsafe {
            let mut ext: XGlyphInfo = std::mem::zeroed();
            xft::XftTextExtentsUtf8(self.dpy, self.fonts[font].xfont, s.as_ptr(), s.len() as c_int, &mut ext);
            ext.xOff.max(0) as u32
        }
    }

    pub fn text_width(&self, text: &str) -> u32 {
        self.runs(text).iter().map(|(f, s)| self.run_width(*f, s)).sum()
    }

    pub fn rect(&self, x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
        unsafe {
            let c = if invert { self.bg.pixel } else { self.fg.pixel };
            xlib::XSetForeground(self.dpy, self.gc, c);
            if filled {
                xlib::XFillRectangle(self.dpy, self.drawable, self.gc, x, y, w, h);
            } else {
                xlib::XDrawRectangle(self.dpy, self.drawable, self.gc, x, y, w.saturating_sub(1), h.saturating_sub(1));
            }
        }
    }

    /// Fill `x,y,w,h` with the background and draw `text` left-padded by `lpad`,
    /// vertically centred. Text that does not fit is cut off with "…".
    /// Returns x + w.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&self, x: i32, y: i32, w: u32, h: u32, lpad: u32, text: &str, invert: bool) -> i32 {
        let (fg, bg) = if invert { (self.bg, self.fg) } else { (self.fg, self.bg) };
        self.text_on(self.drawable, x, y, w, h, lpad, text, &fg, &bg)
    }

    /// Fill a rectangle of any drawable with a colour.
    pub fn fill_on(&self, drawable: xlib::Drawable, x: i32, y: i32, w: u32, h: u32, c: &Color) {
        unsafe {
            xlib::XSetForeground(self.dpy, self.gc, c.pixel);
            xlib::XFillRectangle(self.dpy, drawable, self.gc, x, y, w, h);
        }
    }

    /// Like [`Drw::text`] but onto any drawable with explicit colours.
    #[allow(clippy::too_many_arguments)]
    pub fn text_on(&self, drawable: xlib::Drawable, x: i32, y: i32, w: u32, h: u32, lpad: u32, text: &str, fg: &Color, bg: &Color) -> i32 {
        if w == 0 || self.fonts.is_empty() {
            return x;
        }
        self.fill_on(drawable, x, y, w, h, bg);
        let avail = w.saturating_sub(lpad);
        let mut text = std::borrow::Cow::Borrowed(text);
        if self.text_width(&text) > avail {
            // shrink until "text…" fits
            let mut s: String = text.to_string();
            while !s.is_empty() && self.text_width(&format!("{s}…")) > avail {
                s.pop();
            }
            text = std::borrow::Cow::Owned(if s.is_empty() { String::new() } else { format!("{s}…") });
        }
        unsafe {
            let vis = xlib::XDefaultVisual(self.dpy, self.screen);
            let cmap = xlib::XDefaultColormap(self.dpy, self.screen);
            let d = xft::XftDrawCreate(self.dpy, drawable, vis, cmap);
            let mut cx = x + lpad as i32;
            for (fi, run) in self.runs(&text) {
                let f = &self.fonts[fi];
                let fy = y + (h as i32 - f.h as i32) / 2 + (*f.xfont).ascent;
                xft::XftDrawStringUtf8(d, fg, f.xfont, cx, fy, run.as_ptr(), run.len() as c_int);
                cx += self.run_width(fi, run) as i32;
            }
            xft::XftDrawDestroy(d);
        }
        x + w as i32
    }

    pub fn map(&self, win: xlib::Window, x: i32, y: i32, w: u32, h: u32) {
        unsafe {
            xlib::XCopyArea(self.dpy, self.drawable, win, self.gc, x, y, w, h, x, y);
            xlib::XSync(self.dpy, xlib::False);
        }
    }
}

impl Drop for Drw {
    fn drop(&mut self) {
        self.free_fonts();
        unsafe {
            xlib::XFreePixmap(self.dpy, self.drawable);
            xlib::XFreeGC(self.dpy, self.gc);
        }
    }
}
