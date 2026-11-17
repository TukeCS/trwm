//! Mouse and touchpad settings (what `xinput set-prop` does), through the
//! libinput driver's device properties.

use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_uchar, c_ulong};
use std::ptr;

use x11::{xinput2, xlib};

/// `mouse = { ... }` / `touchpad = { ... }` in the config. `None` = leave it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerCfg {
    /// -1.0 (slowest) .. 1.0 (fastest), 0 = default
    pub speed: Option<f32>,
    /// "adaptive" (acceleration) or "flat" (none)
    pub accel: Option<String>,
    pub natural_scroll: Option<bool>,
    pub left_handed: Option<bool>,
    pub middle_emulation: Option<bool>,
    /// touchpads: tap to click
    pub tap: Option<bool>,
    /// touchpads: ignore the touchpad while typing
    pub disable_while_typing: Option<bool>,
}

impl PointerCfg {
    pub fn is_empty(&self) -> bool {
        *self == PointerCfg::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputCfg {
    pub mouse: PointerCfg,
    pub touchpad: PointerCfg,
}

const P_TAP: &str = "libinput Tapping Enabled";
const P_NATURAL: &str = "libinput Natural Scrolling Enabled";
const P_SPEED: &str = "libinput Accel Speed";
const P_PROFILE: &str = "libinput Accel Profile Enabled";
const P_LEFT: &str = "libinput Left Handed Enabled";
const P_MIDDLE: &str = "libinput Middle Emulation Enabled";
const P_DWT: &str = "libinput Disable While Typing Enabled";

pub struct Device {
    pub id: c_int,
    pub name: String,
    pub touchpad: bool,
    /// handled by libinput (otherwise the settings can't be applied)
    pub libinput: bool,
}

fn atom(dpy: *mut xlib::Display, name: &str, only_if_exists: bool) -> xlib::Atom {
    let c = CString::new(name).unwrap();
    unsafe { xlib::XInternAtom(dpy, c.as_ptr(), if only_if_exists { xlib::True } else { xlib::False }) }
}

struct Prop {
    kind: xlib::Atom,
    format: c_int,
    items: Vec<u32>,
}

/// Read a device property (items widened to u32).
unsafe fn get(dpy: *mut xlib::Display, dev: c_int, name: &str) -> Option<Prop> {
    let a = atom(dpy, name, true);
    if a == 0 {
        return None;
    }
    let (mut kind, mut format, mut n, mut after) = (0 as c_ulong, 0 as c_int, 0 as c_ulong, 0 as c_ulong);
    let mut data: *mut c_uchar = ptr::null_mut();
    let r = xinput2::XIGetProperty(dpy, dev, a, 0, 64, xlib::False, xlib::AnyPropertyType as c_ulong, &mut kind, &mut format, &mut n, &mut after, &mut data);
    if r != xlib::Success as c_int || data.is_null() || kind == 0 {
        if !data.is_null() {
            xlib::XFree(data as *mut _);
        }
        return None;
    }
    let n = n as usize;
    let items = match format {
        8 => std::slice::from_raw_parts(data, n).iter().map(|&b| b as u32).collect(),
        16 => std::slice::from_raw_parts(data as *const u16, n).iter().map(|&b| b as u32).collect(),
        _ => std::slice::from_raw_parts(data as *const u32, n).to_vec(),
    };
    xlib::XFree(data as *mut _);
    Some(Prop { kind, format, items })
}

/// Write a device property back with the same type and format.
unsafe fn put(dpy: *mut xlib::Display, dev: c_int, name: &str, p: &Prop) {
    let a = atom(dpy, name, true);
    let n = p.items.len() as c_int;
    match p.format {
        8 => {
            let mut v: Vec<u8> = p.items.iter().map(|&x| x as u8).collect();
            xinput2::XIChangeProperty(dpy, dev, a, p.kind, 8, xlib::PropModeReplace, v.as_mut_ptr(), n);
        }
        16 => {
            let mut v: Vec<u16> = p.items.iter().map(|&x| x as u16).collect();
            xinput2::XIChangeProperty(dpy, dev, a, p.kind, 16, xlib::PropModeReplace, v.as_mut_ptr() as *mut c_uchar, n);
        }
        _ => {
            let mut v = p.items.clone();
            xinput2::XIChangeProperty(dpy, dev, a, p.kind, 32, xlib::PropModeReplace, v.as_mut_ptr() as *mut c_uchar, n);
        }
    }
}

/// All real mice and touchpads.
pub fn devices(dpy: *mut xlib::Display) -> Vec<Device> {
    let mut out = vec![];
    unsafe {
        let mut n = 0;
        let info = xinput2::XIQueryDevice(dpy, xinput2::XIAllDevices, &mut n);
        if info.is_null() {
            return out;
        }
        for i in 0..n as usize {
            let d = &*info.add(i);
            if d._use != xinput2::XISlavePointer || d.name.is_null() {
                continue;
            }
            let name = CStr::from_ptr(d.name).to_string_lossy().into_owned();
            if name.contains("XTEST") {
                continue; // the virtual pointer used by xdotool & co
            }
            out.push(Device {
                id: d.deviceid,
                touchpad: get(dpy, d.deviceid, P_TAP).is_some(),
                libinput: get(dpy, d.deviceid, P_NATURAL).is_some() || get(dpy, d.deviceid, P_SPEED).is_some(),
                name,
            });
        }
        xinput2::XIFreeDeviceInfo(info);
    }
    out
}

unsafe fn set_bool(dpy: *mut xlib::Display, dev: c_int, name: &str, on: Option<bool>) {
    let Some(on) = on else { return };
    if let Some(mut p) = get(dpy, dev, name)
        && p.items.first() != Some(&(on as u32))
    {
        if p.items.is_empty() {
            p.items.push(0);
        }
        p.items[0] = on as u32;
        put(dpy, dev, name, &p);
    }
}

unsafe fn apply_device(dpy: *mut xlib::Display, d: &Device, cfg: &PointerCfg) {
    set_bool(dpy, d.id, P_NATURAL, cfg.natural_scroll);
    set_bool(dpy, d.id, P_LEFT, cfg.left_handed);
    set_bool(dpy, d.id, P_MIDDLE, cfg.middle_emulation);
    set_bool(dpy, d.id, P_TAP, cfg.tap);
    set_bool(dpy, d.id, P_DWT, cfg.disable_while_typing);
    if let Some(speed) = cfg.speed
        && let Some(mut p) = get(dpy, d.id, P_SPEED)
        && !p.items.is_empty()
    {
        let bits = speed.clamp(-1.0, 1.0).to_bits();
        if p.items[0] != bits {
            p.items[0] = bits;
            put(dpy, d.id, P_SPEED, &p);
        }
    }
    if let Some(accel) = &cfg.accel
        && let Some(mut p) = get(dpy, d.id, P_PROFILE)
    {
        // one flag per profile: adaptive, flat (, custom)
        let idx = if accel == "flat" { 1 } else { 0 };
        if idx < p.items.len() {
            let want: Vec<u32> = (0..p.items.len()).map(|i| (i == idx) as u32).collect();
            if p.items != want {
                p.items = want;
                put(dpy, d.id, P_PROFILE, &p);
            }
        }
    }
}

/// Apply the mouse settings to every mouse and the touchpad settings to every
/// touchpad. Settings a device doesn't support are skipped.
pub fn apply(dpy: *mut xlib::Display, cfg: &InputCfg) {
    if cfg.mouse.is_empty() && cfg.touchpad.is_empty() {
        return;
    }
    unsafe {
        for d in devices(dpy) {
            let c = if d.touchpad { &cfg.touchpad } else { &cfg.mouse };
            if !c.is_empty() && d.libinput {
                apply_device(dpy, &d, c);
            }
        }
        xlib::XSync(dpy, xlib::False);
    }
}

/// Current settings of a device, for `trwm input`.
pub fn describe(dpy: *mut xlib::Display, d: &Device) -> String {
    unsafe {
        let b = |name: &str| get(dpy, d.id, name).and_then(|p| p.items.first().copied()).map(|v| if v != 0 { "on" } else { "off" });
        let mut parts = vec![];
        if let Some(p) = get(dpy, d.id, P_SPEED).filter(|p| !p.items.is_empty()) {
            parts.push(format!("speed={:.2}", f32::from_bits(p.items[0])));
        }
        if let Some(p) = get(dpy, d.id, P_PROFILE) {
            parts.push(format!("accel={}", if p.items.get(1) == Some(&1) { "flat" } else { "adaptive" }));
        }
        for (label, prop) in
            [("natural_scroll", P_NATURAL), ("left_handed", P_LEFT), ("middle_emulation", P_MIDDLE), ("tap", P_TAP), ("disable_while_typing", P_DWT)]
        {
            if let Some(v) = b(prop) {
                parts.push(format!("{label}={v}"));
            }
        }
        if !d.libinput {
            parts.push("(not using the libinput driver: settings can't be changed)".into());
        }
        parts.join(" ")
    }
}

/// Ask for an event when devices are plugged in or removed.
pub fn watch_hotplug(dpy: *mut xlib::Display, root: xlib::Window) -> Option<c_int> {
    unsafe {
        let name = CString::new("XInputExtension").unwrap();
        let (mut opcode, mut ev, mut err) = (0, 0, 0);
        if xlib::XQueryExtension(dpy, name.as_ptr(), &mut opcode, &mut ev, &mut err) == 0 {
            return None;
        }
        let mut mask_bits = [0u8; 4];
        mask_bits[(xinput2::XI_HierarchyChanged / 8) as usize] |= 1 << (xinput2::XI_HierarchyChanged % 8);
        let mut mask = xinput2::XIEventMask { deviceid: xinput2::XIAllDevices, mask_len: mask_bits.len() as c_int, mask: mask_bits.as_mut_ptr() };
        xinput2::XISelectEvents(dpy, root, &mut mask, 1);
        Some(opcode)
    }
}

/// Lua table -> PointerCfg (shared by `mouse` and `touchpad`).
pub fn parse(t: Option<mlua::Table>, what: &str) -> mlua::Result<PointerCfg> {
    let Some(t) = t else { return Ok(PointerCfg::default()) };
    let accel: Option<String> = t.get("accel")?;
    if let Some(a) = &accel
        && a != "flat"
        && a != "adaptive"
    {
        return Err(mlua::Error::RuntimeError(format!("{what}.accel: expected \"flat\" or \"adaptive\", got '{a}'")));
    }
    let speed: Option<f32> = t.get("speed")?;
    if let Some(s) = speed
        && !(-1.0..=1.0).contains(&s)
    {
        return Err(mlua::Error::RuntimeError(format!("{what}.speed: {s} is out of range (-1.0 to 1.0)")));
    }
    Ok(PointerCfg {
        speed,
        accel,
        natural_scroll: t.get("natural_scroll")?,
        left_handed: t.get("left_handed")?,
        middle_emulation: t.get("middle_emulation")?,
        tap: t.get("tap")?,
        disable_while_typing: t.get("disable_while_typing")?,
    })
}
