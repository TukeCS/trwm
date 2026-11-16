//! Monitor arrangement (what `xrandr --output ... --mode ... --right-of ...`
//! does), through RandR.
//!
//! The arrangement is worked out by [`plan`], a pure function, and then
//! applied by [`apply`], which changes nothing when everything already
//! matches (so saving the config never makes the screen flicker).

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr;

use x11::{xlib, xrandr};

#[derive(Clone, Debug, PartialEq)]
pub enum Pos {
    /// keep the current position, or go right of everything else
    Auto,
    At(i32, i32),
    RightOf(String),
    LeftOf(String),
    Above(String),
    Below(String),
    SameAs(String),
}

/// One entry of `monitors = { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorCfg {
    /// the output's name, or several (the first one that exists is used: the
    /// same port can have a different name depending on the GPU mode)
    pub names: Vec<String>,
    /// None = the monitor's preferred mode
    pub mode: Option<(u32, u32)>,
    /// None = the highest rate for the mode (or the preferred mode's rate)
    pub rate: Option<f64>,
    pub pos: Pos,
    pub primary: bool,
    /// RR_Rotate_0 / 90 / 180 / 270
    pub rotation: u16,
    pub off: bool,
}

#[derive(Clone, Debug)]
pub struct Mode {
    pub id: u64,
    pub w: u32,
    pub h: u32,
    pub rate: f64,
}

/// What the X server says about one output.
#[derive(Clone, Debug)]
pub struct Output {
    pub id: u64,
    pub name: String,
    pub connected: bool,
    /// modes this output supports, best first; the first `npreferred` are preferred
    pub modes: Vec<Mode>,
    pub npreferred: usize,
    pub possible_crtcs: Vec<u64>,
    /// current state: crtc (0 = off), mode, position, rotation, size on screen
    pub crtc: u64,
    pub mode: u64,
    pub x: i32,
    pub y: i32,
    pub rotation: u16,
    pub w: u32,
    pub h: u32,
}

/// The desired state of one crtc.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub output: u64,
    pub crtc: u64,
    /// 0 = switch off
    pub mode: u64,
    pub x: i32,
    pub y: i32,
    pub rotation: u16,
    pub w: u32,
    pub h: u32,
}

pub const ROT_0: u16 = 1;
pub const ROT_90: u16 = 2;
pub const ROT_180: u16 = 4;
pub const ROT_270: u16 = 8;

fn rotated(w: u32, h: u32, rot: u16) -> (u32, u32) {
    if rot == ROT_90 || rot == ROT_270 { (h, w) } else { (w, h) }
}

pub fn parse_rotation(s: &str) -> Option<u16> {
    Some(match s {
        "normal" | "none" | "0" => ROT_0,
        "left" | "90" => ROT_90,
        "inverted" | "180" => ROT_180,
        "right" | "270" => ROT_270,
        _ => return None,
    })
}

fn pick_mode(o: &Output, c: &MonitorCfg) -> Result<Mode, String> {
    let preferred = o.modes.iter().take(o.npreferred.max(1)).next().cloned();
    let (w, h) = match (c.mode, &preferred) {
        (Some(s), _) => s,
        (None, Some(p)) => (p.w, p.h),
        (None, None) => return Err(format!("monitors: {} reports no modes", o.name)),
    };
    let same: Vec<&Mode> = o.modes.iter().filter(|m| m.w == w && m.h == h).collect();
    if same.is_empty() {
        let mut sizes: Vec<String> = o.modes.iter().map(|m| format!("{}x{}", m.w, m.h)).collect();
        sizes.dedup();
        return Err(format!("monitors: {} has no mode {w}x{h} (it has: {})", o.name, sizes.join(", ")));
    }
    let m = match c.rate {
        Some(r) => same.iter().min_by(|a, b| (a.rate - r).abs().total_cmp(&(b.rate - r).abs())).copied(),
        None => match &preferred {
            Some(p) if p.w == w && p.h == h && c.mode.is_none() => same.iter().find(|m| m.id == p.id).copied(),
            _ => same.iter().max_by(|a, b| a.rate.total_cmp(&b.rate)).copied(),
        },
    };
    Ok(m.unwrap().clone())
}

/// Work out the crtc settings and the screen size for `cfg`.
/// Returns (plans for every crtc that should be on or turned off, screen size,
/// primary output).
/// (crtc plans, screen size, primary output)
pub type Arrangement = (Vec<Plan>, (u32, u32), Option<u64>);

/// The name of the output `c` refers to on this machine right now: the first
/// of its names that is connected, else the first that exists.
fn resolve<'a>(outputs: &'a [Output], c: &MonitorCfg) -> Option<&'a str> {
    let find = |connected: bool| c.names.iter().find_map(|n| outputs.iter().find(|o| &o.name == n && (o.connected || !connected)));
    find(true).or_else(|| find(false)).map(|o| o.name.as_str())
}

/// Config entries whose output doesn't exist (they are skipped).
pub fn missing(outputs: &[Output], cfg: &[MonitorCfg]) -> Vec<String> {
    cfg.iter().filter(|c| resolve(outputs, c).is_none()).map(|c| c.names.join(" / ")).collect()
}

pub fn plan(outputs: &[Output], cfg: &[MonitorCfg]) -> Result<Arrangement, String> {
    // entries for outputs that don't exist (a GPU mode with other output names,
    // a dock that isn't attached, ...) are skipped; the rest use the real names,
    // also in right_of = ... and friends
    let real = |n: &String| -> String {
        cfg.iter()
            .find(|c| c.names.contains(n))
            .and_then(|c| resolve(outputs, c))
            .map_or_else(|| n.clone(), str::to_string)
    };
    let cfg: Vec<(String, MonitorCfg)> = cfg
        .iter()
        .filter_map(|c| {
            let name = resolve(outputs, c)?.to_string();
            let mut c = c.clone();
            c.pos = match c.pos {
                Pos::RightOf(r) => Pos::RightOf(real(&r)),
                Pos::LeftOf(r) => Pos::LeftOf(real(&r)),
                Pos::Above(r) => Pos::Above(real(&r)),
                Pos::Below(r) => Pos::Below(real(&r)),
                Pos::SameAs(r) => Pos::SameAs(real(&r)),
                p => p,
            };
            Some((name, c))
        })
        .collect();
    let cfg_of = |o: &Output| cfg.iter().find(|(n, _)| *n == o.name).map(|(_, c)| c);

    // what each output should do
    struct Want {
        idx: usize,
        mode: Mode,
        rotation: u16,
        pos: Pos,
        w: u32,
        h: u32,
    }
    let mut wants: Vec<Want> = vec![]; // outputs that should be on
    let mut offs: Vec<usize> = vec![];
    for (i, o) in outputs.iter().enumerate() {
        match cfg_of(o) {
            Some(c) if c.off || !o.connected => {
                if o.crtc != 0 {
                    offs.push(i);
                }
            }
            Some(c) => {
                let mode = pick_mode(o, c)?;
                let (w, h) = rotated(mode.w, mode.h, c.rotation);
                wants.push(Want { idx: i, mode, rotation: c.rotation, pos: c.pos.clone(), w, h });
            }
            None if o.crtc != 0 && !o.connected => offs.push(i), // unplugged but still on
            None if o.crtc != 0 => {
                // not in the config: leave it as it is
                let mode = o.modes.iter().find(|m| m.id == o.mode).cloned().unwrap_or(Mode { id: o.mode, w: o.w, h: o.h, rate: 0.0 });
                wants.push(Want { idx: i, mode, rotation: o.rotation, pos: Pos::At(o.x, o.y), w: o.w, h: o.h });
            }
            None => {}
        }
    }

    // positions: resolve explicit ones, then the ones relative to them
    let mut placed: HashMap<String, (i32, i32, u32, u32)> = HashMap::new();
    let name = |w: &Want| outputs[w.idx].name.clone();
    for _ in 0..=wants.len() {
        for w in &wants {
            if placed.contains_key(&name(w)) {
                continue;
            }
            let rel = |r: &String, f: &dyn Fn((i32, i32, u32, u32)) -> (i32, i32)| placed.get(r).map(|&g| f(g));
            let at = match &w.pos {
                Pos::At(x, y) => Some((*x, *y)),
                Pos::RightOf(r) => rel(r, &|(x, y, rw, _)| (x + rw as i32, y)),
                Pos::LeftOf(r) => rel(r, &|(x, y, _, _)| (x - w.w as i32, y)),
                Pos::Below(r) => rel(r, &|(x, y, _, rh)| (x, y + rh as i32)),
                Pos::Above(r) => rel(r, &|(x, y, _, _)| (x, y - w.h as i32)),
                Pos::SameAs(r) => rel(r, &|(x, y, _, _)| (x, y)),
                Pos::Auto => None,
            };
            if let Some((x, y)) = at {
                placed.insert(name(w), (x, y, w.w, w.h));
            }
        }
    }
    // Auto (and references to monitors that aren't on): keep the current
    // position if it was on, else go to the right of everything
    for w in &wants {
        if placed.contains_key(&name(w)) {
            continue;
        }
        let o = &outputs[w.idx];
        let (x, y) = if o.crtc != 0 {
            (o.x, o.y)
        } else {
            (placed.values().map(|&(x, _, w, _)| x + w as i32).max().unwrap_or(0), 0)
        };
        placed.insert(name(w), (x, y, w.w, w.h));
    }
    // the screen starts at 0,0
    let minx = placed.values().map(|g| g.0).min().unwrap_or(0);
    let miny = placed.values().map(|g| g.1).min().unwrap_or(0);
    for g in placed.values_mut() {
        g.0 -= minx;
        g.1 -= miny;
    }
    let sw = placed.values().map(|&(x, _, w, _)| (x + w as i32) as u32).max().unwrap_or(0);
    let sh = placed.values().map(|&(_, y, _, h)| (y + h as i32) as u32).max().unwrap_or(0);

    // crtcs: keep the current one, else take a free one this output can use
    let mut taken: Vec<u64> = wants.iter().map(|w| outputs[w.idx].crtc).filter(|&c| c != 0).collect();
    let mut plans = vec![];
    for w in &wants {
        let o = &outputs[w.idx];
        let crtc = if o.crtc != 0 {
            o.crtc
        } else {
            let free = o.possible_crtcs.iter().copied().find(|c| !taken.contains(c));
            let free = free.ok_or_else(|| format!("monitors: no free display controller for {} (too many monitors on?)", o.name))?;
            taken.push(free);
            free
        };
        let (x, y, _, _) = placed[&o.name];
        plans.push(Plan { output: o.id, crtc, mode: w.mode.id, x, y, rotation: w.rotation, w: w.w, h: w.h });
    }
    for &i in &offs {
        let o = &outputs[i];
        plans.push(Plan { output: o.id, crtc: o.crtc, mode: 0, x: 0, y: 0, rotation: ROT_0, w: 0, h: 0 });
    }
    let primary = cfg
        .iter()
        .filter(|(_, c)| c.primary && !c.off)
        .find_map(|(n, _)| outputs.iter().find(|o| &o.name == n && o.connected).map(|o| o.id));
    Ok((plans, (sw, sh), primary))
}

/// Does this plan differ from what the output is doing now?
fn changes(p: &Plan, o: &Output) -> bool {
    if p.mode == 0 {
        return o.crtc != 0;
    }
    o.crtc != p.crtc || o.mode != p.mode || o.x != p.x || o.y != p.y || o.rotation != p.rotation
}

/// Read the outputs from the X server.
pub fn query(dpy: *mut xlib::Display, root: xlib::Window) -> Option<Vec<Output>> {
    unsafe {
        let res = xrandr::XRRGetScreenResourcesCurrent(dpy, root);
        if res.is_null() {
            return None;
        }
        let r = &*res;
        let mode_list: Vec<Mode> = (0..r.nmode as usize)
            .map(|i| {
                let m = &*r.modes.add(i);
                let mut v = m.vTotal as f64;
                if m.modeFlags & xrandr::RR_DoubleScan as u64 != 0 {
                    v *= 2.0;
                }
                if m.modeFlags & xrandr::RR_Interlace as u64 != 0 {
                    v /= 2.0;
                }
                let rate = if m.hTotal > 0 && v > 0.0 { m.dotClock as f64 / (m.hTotal as f64 * v) } else { 0.0 };
                Mode { id: m.id, w: m.width, h: m.height, rate }
            })
            .collect();
        let mut out = vec![];
        for i in 0..r.noutput as usize {
            let id = *r.outputs.add(i);
            let info = xrandr::XRRGetOutputInfo(dpy, res, id);
            if info.is_null() {
                continue;
            }
            let oi = &*info;
            let name = CStr::from_ptr(oi.name).to_string_lossy().into_owned();
            let modes: Vec<Mode> = (0..oi.nmode as usize)
                .filter_map(|j| {
                    let mid = *oi.modes.add(j);
                    mode_list.iter().find(|m| m.id == mid).cloned()
                })
                .collect();
            let possible_crtcs = (0..oi.ncrtc as usize).map(|j| *oi.crtcs.add(j)).collect();
            let mut o = Output {
                id,
                name,
                connected: oi.connection == xrandr::RR_Connected as u16,
                modes,
                npreferred: oi.npreferred.max(0) as usize,
                possible_crtcs,
                crtc: oi.crtc,
                mode: 0,
                x: 0,
                y: 0,
                rotation: ROT_0,
                w: 0,
                h: 0,
            };
            if oi.crtc != 0 {
                let ci = xrandr::XRRGetCrtcInfo(dpy, res, oi.crtc);
                if !ci.is_null() {
                    let c = &*ci;
                    (o.mode, o.x, o.y, o.rotation, o.w, o.h) = (c.mode, c.x, c.y, c.rotation & 0xf, c.width, c.height);
                    xrandr::XRRFreeCrtcInfo(ci);
                }
                if o.mode == 0 {
                    o.crtc = 0;
                }
            }
            xrandr::XRRFreeOutputInfo(info);
            out.push(o);
        }
        xrandr::XRRFreeScreenResources(res);
        Some(out)
    }
}

/// Arrange the monitors. Does nothing if they already match `cfg`.
/// Returns true if anything was changed.
pub fn apply(dpy: *mut xlib::Display, root: xlib::Window, screen: c_int, cfg: &[MonitorCfg], dpi: Option<u32>) -> Result<bool, String> {
    if cfg.is_empty() {
        return Ok(false);
    }
    let outputs = query(dpy, root).ok_or("monitors: RandR is not available")?;
    let (plans, (sw, sh), primary) = plan(&outputs, cfg)?;
    let by_id = |id: u64| outputs.iter().find(|o| o.id == id).unwrap();
    let cur_w = unsafe { xlib::XDisplayWidth(dpy, screen) } as u32;
    let cur_h = unsafe { xlib::XDisplayHeight(dpy, screen) } as u32;
    let changed: Vec<&Plan> = plans.iter().filter(|p| changes(p, by_id(p.output))).collect();
    let resize = (sw, sh) != (cur_w, cur_h) && sw > 0 && sh > 0;
    unsafe {
        let cur_primary = xrandr::XRRGetOutputPrimary(dpy, root);
        let primary_change = primary.is_some_and(|p| p != cur_primary);
        if changed.is_empty() && !resize && !primary_change {
            return Ok(false);
        }
        let (mut minw, mut minh, mut maxw, mut maxh) = (0, 0, 0, 0);
        xrandr::XRRGetScreenSizeRange(dpy, root, &mut minw, &mut minh, &mut maxw, &mut maxh);
        if maxw > 0 && (sw as c_int > maxw || sh as c_int > maxh) {
            return Err(format!("monitors: the arrangement needs {sw}x{sh} pixels, but the graphics card allows at most {maxw}x{maxh}"));
        }
        let res = xrandr::XRRGetScreenResourcesCurrent(dpy, root);
        if res.is_null() {
            return Err("monitors: RandR is not available".into());
        }
        xlib::XGrabServer(dpy);
        let mut err = None;
        // switch off what changes first, so the screen can be resized
        for p in &changed {
            let o = by_id(p.output);
            if o.crtc != 0 {
                xrandr::XRRSetCrtcConfig(dpy, res, o.crtc, xlib::CurrentTime, 0, 0, 0, ROT_0, ptr::null_mut(), 0);
            }
        }
        if resize {
            // keep the DPI (the configured one, or the current one)
            let mm = |px: u32, cur_px: c_int, cur_mm: c_int| match dpi {
                Some(d) => ((px as f64 * 25.4 / d as f64).round() as c_int).max(1),
                None if cur_px > 0 && cur_mm > 0 => ((px as f64 * cur_mm as f64 / cur_px as f64).round() as c_int).max(1),
                None => ((px as f64 * 25.4 / 96.0).round() as c_int).max(1),
            };
            let mmw = mm(sw, xlib::XDisplayWidth(dpy, screen), xlib::XDisplayWidthMM(dpy, screen));
            let mmh = mm(sh, xlib::XDisplayHeight(dpy, screen), xlib::XDisplayHeightMM(dpy, screen));
            xrandr::XRRSetScreenSize(dpy, root, sw as c_int, sh as c_int, mmw, mmh);
        }
        for p in changed.iter().filter(|p| p.mode != 0) {
            let mut out = p.output;
            let st = xrandr::XRRSetCrtcConfig(dpy, res, p.crtc, xlib::CurrentTime, p.x, p.y, p.mode, p.rotation, &mut out, 1);
            if st != 0 && err.is_none() {
                err = Some(format!("monitors: the X server refused the settings for {}", by_id(p.output).name));
            }
        }
        if let Some(pr) = primary {
            xrandr::XRRSetOutputPrimary(dpy, root, pr);
        }
        xlib::XUngrabServer(dpy);
        xlib::XSync(dpy, xlib::False);
        xrandr::XRRFreeScreenResources(res);
        match err {
            Some(e) => Err(e),
            None => Ok(true),
        }
    }
}

/// Ask for an event when monitors are plugged in or removed.
/// Returns the RandR event base.
pub fn watch_hotplug(dpy: *mut xlib::Display, root: xlib::Window) -> Option<c_int> {
    unsafe {
        let (mut eb, mut er) = (0, 0);
        if xrandr::XRRQueryExtension(dpy, &mut eb, &mut er) == 0 {
            return None;
        }
        xrandr::XRRSelectInput(dpy, root, xrandr::RROutputChangeNotifyMask);
        Some(eb)
    }
}

/// Text for `trwm monitors`.
pub fn describe(o: &Output) -> String {
    let mut s = format!("{}  {}", o.name, if o.connected { "connected" } else { "disconnected" });
    if o.crtc != 0 {
        let rate = o.modes.iter().find(|m| m.id == o.mode).map(|m| m.rate).unwrap_or(0.0);
        let rot = match o.rotation {
            ROT_90 => " left",
            ROT_180 => " inverted",
            ROT_270 => " right",
            _ => "",
        };
        s += &format!("  {}x{} @ {rate:.2} Hz at {},{}{rot}", o.w, o.h, o.x, o.y);
    } else if o.connected {
        s += "  (off)";
    }
    if o.connected && !o.modes.is_empty() {
        // "2560x1600 (240, 60)  1920x1200 (60)" ...
        let mut sizes: Vec<(u32, u32, Vec<f64>)> = vec![];
        for m in &o.modes {
            match sizes.iter_mut().find(|(w, h, _)| *w == m.w && *h == m.h) {
                Some(e) => e.2.push(m.rate),
                None => sizes.push((m.w, m.h, vec![m.rate])),
            }
        }
        let list: Vec<String> = sizes
            .iter()
            .take(8)
            .map(|(w, h, r)| format!("{w}x{h} ({})", r.iter().map(|r| format!("{r:.0}")).collect::<Vec<_>>().join(", ")))
            .collect();
        s += &format!("\n    modes: {}{}", list.join("  "), if sizes.len() > 8 { "  ..." } else { "" });
    }
    s
}

/// Lua list -> Vec<MonitorCfg>
pub fn parse(t: Option<mlua::Table>) -> mlua::Result<Vec<MonitorCfg>> {
    let err = |m: String| mlua::Error::RuntimeError(m);
    let Some(t) = t else { return Ok(vec![]) };
    let mut out = vec![];
    for (i, e) in t.sequence_values::<mlua::Table>().enumerate() {
        let e = e?;
        let need = || err(format!("monitors[{}]: needs output = \"name\" or {{ \"name\", \"other name\" }} (see `trwm monitors`)", i + 1));
        let names: Vec<String> = match e.get::<mlua::Value>("output")? {
            mlua::Value::String(s) => vec![s.to_str()?.to_string()],
            mlua::Value::Table(t) => t.sequence_values::<String>().collect::<mlua::Result<_>>().map_err(|_| need())?,
            _ => return Err(need()),
        };
        if names.is_empty() {
            return Err(need());
        }
        let mode = match e.get::<Option<String>>("mode")? {
            None => None,
            Some(m) => {
                let (w, h) = m.split_once('x').ok_or_else(|| err(format!("monitors[{}]: mode should look like \"1920x1080\"", i + 1)))?;
                Some((w.trim().parse().map_err(|_| err(format!("monitors[{}]: bad mode '{m}'", i + 1)))?, h.trim().parse().map_err(|_| err(format!("monitors[{}]: bad mode '{m}'", i + 1)))?))
            }
        };
        let rel = |k: &str| e.get::<Option<String>>(k);
        let pos = if let Some(p) = e.get::<Option<mlua::Value>>("pos")? {
            match p {
                mlua::Value::String(s) => {
                    let s = s.to_string_lossy();
                    let (x, y) = s.split_once('x').ok_or_else(|| err(format!("monitors[{}]: pos should look like \"1920x0\" or {{ 1920, 0 }}", i + 1)))?;
                    Pos::At(x.trim().parse().map_err(|_| err(format!("monitors[{}]: bad pos '{s}'", i + 1)))?, y.trim().parse().map_err(|_| err(format!("monitors[{}]: bad pos '{s}'", i + 1)))?)
                }
                mlua::Value::Table(t) => Pos::At(t.get(1)?, t.get(2)?),
                _ => return Err(err(format!("monitors[{}]: pos should look like \"1920x0\" or {{ 1920, 0 }}", i + 1))),
            }
        } else if let Some(r) = rel("right_of")? {
            Pos::RightOf(r)
        } else if let Some(r) = rel("left_of")? {
            Pos::LeftOf(r)
        } else if let Some(r) = rel("above")? {
            Pos::Above(r)
        } else if let Some(r) = rel("below")? {
            Pos::Below(r)
        } else if let Some(r) = rel("same_as")? {
            Pos::SameAs(r)
        } else {
            Pos::Auto
        };
        let rotation = match e.get::<Option<String>>("rotate")? {
            None => ROT_0,
            Some(r) => parse_rotation(&r).ok_or_else(|| err(format!("monitors[{}]: rotate should be normal, left, right or inverted", i + 1)))?,
        };
        out.push(MonitorCfg {
            names,
            mode,
            rate: e.get("rate")?,
            pos,
            primary: e.get::<Option<bool>>("primary")?.unwrap_or(false),
            rotation,
            off: e.get::<Option<bool>>("off")?.unwrap_or(false),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(id: u64, name: &str, connected: bool, on: Option<(i32, i32)>) -> Output {
        Output {
            id,
            name: name.into(),
            connected,
            modes: vec![
                Mode { id: id * 10 + 1, w: 1920, h: 1080, rate: 144.0 },
                Mode { id: id * 10 + 2, w: 1920, h: 1080, rate: 60.0 },
                Mode { id: id * 10 + 3, w: 1280, h: 720, rate: 60.0 },
            ],
            npreferred: 1,
            possible_crtcs: vec![100, 200],
            crtc: if on.is_some() { 100 + (id - 1) * 100 } else { 0 },
            mode: if on.is_some() { id * 10 + 1 } else { 0 },
            x: on.map_or(0, |p| p.0),
            y: on.map_or(0, |p| p.1),
            rotation: ROT_0,
            w: if on.is_some() { 1920 } else { 0 },
            h: if on.is_some() { 1080 } else { 0 },
        }
    }

    fn mc(name: &str, pos: Pos) -> MonitorCfg {
        MonitorCfg { names: vec![name.into()], mode: None, rate: None, pos, primary: false, rotation: ROT_0, off: false }
    }

    #[test]
    fn right_of_and_auto_crtc() {
        let o = vec![out(1, "eDP-1", true, Some((0, 0))), out(2, "HDMI-1", true, None)];
        let (p, size, _) = plan(&o, &[mc("HDMI-1", Pos::RightOf("eDP-1".into()))]).unwrap();
        assert_eq!(size, (3840, 1080));
        let h = p.iter().find(|p| p.output == 2).unwrap();
        assert_eq!((h.crtc, h.x, h.y, h.mode), (200, 1920, 0, 21)); // free crtc, preferred mode
        // eDP-1 is not in the config: left exactly as it is
        let e = p.iter().find(|p| p.output == 1).unwrap();
        assert!(!changes(e, &o[0]));
    }

    #[test]
    fn left_of_normalises_to_zero() {
        let o = vec![out(1, "eDP-1", true, Some((0, 0))), out(2, "HDMI-1", true, None)];
        let (p, size, _) = plan(&o, &[MonitorCfg { mode: Some((1280, 720)), ..mc("HDMI-1", Pos::LeftOf("eDP-1".into())) }]).unwrap();
        assert_eq!(size, (3200, 1080));
        assert_eq!(p.iter().find(|p| p.output == 2).map(|p| (p.x, p.mode)), Some((0, 23)));
        assert_eq!(p.iter().find(|p| p.output == 1).map(|p| p.x), Some(1280)); // laptop moves right
    }

    #[test]
    fn rate_and_rotation() {
        let o = vec![out(1, "eDP-1", true, Some((0, 0)))];
        let c = MonitorCfg { mode: Some((1920, 1080)), rate: Some(59.0), rotation: ROT_90, ..mc("eDP-1", Pos::Auto) };
        let (p, size, _) = plan(&o, &[c]).unwrap();
        assert_eq!((p[0].mode, p[0].w, p[0].h), (12, 1080, 1920));
        assert_eq!(size, (1080, 1920));
    }

    #[test]
    fn same_config_changes_nothing() {
        let o = vec![out(1, "eDP-1", true, Some((0, 0))), out(2, "HDMI-1", true, Some((1920, 0)))];
        let (p, size, prim) = plan(&o, &[MonitorCfg { primary: true, ..mc("eDP-1", Pos::Auto) }, mc("HDMI-1", Pos::RightOf("eDP-1".into()))]).unwrap();
        assert_eq!(size, (3840, 1080));
        assert!(p.iter().all(|pl| !changes(pl, o.iter().find(|x| x.id == pl.output).unwrap())));
        assert_eq!(prim, Some(1));
    }

    #[test]
    fn off_unplugged_and_errors() {
        let o = vec![out(1, "eDP-1", true, Some((0, 0))), out(2, "HDMI-1", false, Some((1920, 0)))];
        // HDMI-1 was unplugged but is still on: switch it off
        let (p, size, _) = plan(&o, &[mc("eDP-1", Pos::Auto)]).unwrap();
        assert_eq!(p.iter().find(|p| p.output == 2).map(|p| p.mode), Some(0));
        assert_eq!(size, (1920, 1080));
        // explicit off
        let (p, _, _) = plan(&o, &[MonitorCfg { off: true, ..mc("eDP-1", Pos::Auto) }]).unwrap();
        assert_eq!(p.iter().find(|p| p.output == 1).map(|p| p.mode), Some(0));
        // outputs that don't exist are skipped (and reported by `missing`)
        assert!(plan(&o, &[mc("HDMI-2", Pos::Auto)]).is_ok());
        assert_eq!(missing(&o, &[mc("HDMI-2", Pos::Auto), mc("eDP-1", Pos::Auto)]), vec!["HDMI-2".to_string()]);
        assert!(plan(&o, &[MonitorCfg { mode: Some((800, 600)), ..mc("eDP-1", Pos::Auto) }]).unwrap_err().contains("has no mode 800x600"));
    }

    #[test]
    fn alternative_names() {
        // hybrid graphics: the same ports get other names
        let o = vec![out(1, "eDP-1-0", true, Some((0, 0))), out(2, "HDMI-A-1-0", true, None)];
        let c = |a: &str, b: &str, pos| MonitorCfg { names: vec![a.into(), b.into()], ..mc(a, pos) };
        let cfg = [
            MonitorCfg { primary: true, ..c("eDP", "eDP-1-0", Pos::Auto) },
            c("HDMI-A-0", "HDMI-A-1-0", Pos::RightOf("eDP".into())),
        ];
        let (p, size, prim) = plan(&o, &cfg).unwrap();
        assert_eq!(size, (3840, 1080));
        assert_eq!(p.iter().find(|p| p.output == 2).map(|p| p.x), Some(1920));
        assert_eq!(prim, Some(1));
        assert!(missing(&o, &cfg).is_empty());
        // a connected name wins over one that only exists
        let o = vec![out(1, "eDP-1-0", false, None), out(2, "eDP-1", true, Some((0, 0)))];
        let (p, _, prim) = plan(&o, &[MonitorCfg { primary: true, ..c("eDP-1-0", "eDP-1", Pos::Auto) }]).unwrap();
        assert!(p.iter().all(|p| p.output == 2));
        assert_eq!(prim, Some(2));
    }
}
