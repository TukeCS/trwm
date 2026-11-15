//! Keyboard settings: layout (what `setxkbmap` does), key repeat (`xset r
//! rate`) and num lock (`numlockx`), done directly through XKB so none of
//! those tools need to be installed.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_ushort};
use std::ptr;

use x11::xlib;

const XKB_USE_CORE_KBD: c_uint = 0x0100;
const XKB_GBN_ALL: c_uint = 0xff;
const XKB_GBN_GEOMETRY: c_uint = 1 << 6;
const XKB_ALL_COMPONENTS: c_uint = 0x7f;

/// Where the XKB data (rules, symbols, ...) lives.
const XKB_ROOTS: &[&str] = &["/usr/share/X11/xkb", "/usr/lib/X11/xkb", "/usr/X11R6/lib/X11/xkb"];

#[repr(C)]
struct VarDefs {
    model: *mut c_char,
    layout: *mut c_char,
    variant: *mut c_char,
    options: *mut c_char,
    sz_extra: c_ushort,
    num_extra: c_ushort,
    extra_names: *mut c_char,
    extra_values: *mut *mut c_char,
}

#[repr(C)]
struct ComponentNames {
    keymap: *mut c_char,
    keycodes: *mut c_char,
    types: *mut c_char,
    compat: *mut c_char,
    symbols: *mut c_char,
    geometry: *mut c_char,
}

enum Rules {}

#[link(name = "xkbfile")]
unsafe extern "C" {
    fn XkbRF_Load(base: *mut c_char, locale: *mut c_char, want_desc: c_int, want_rules: c_int) -> *mut Rules;
    fn XkbRF_GetComponents(rules: *mut Rules, defs: *mut VarDefs, names: *mut ComponentNames) -> c_int;
    fn XkbRF_Free(rules: *mut Rules, free_rules: c_int);
    fn XkbRF_GetNamesProp(dpy: *mut xlib::Display, rules_file: *mut *mut c_char, defs: *mut VarDefs) -> c_int;
    fn XkbRF_SetNamesProp(dpy: *mut xlib::Display, rules_file: *mut c_char, defs: *mut VarDefs) -> c_int;
}

/// `keyboard = { ... }` in the config. `None` = leave as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyboardCfg {
    /// comma-separated, e.g. "us,fr"
    pub layout: Option<String>,
    /// comma-separated, one per layout, e.g. ",azerty"
    pub variant: Option<String>,
    pub model: Option<String>,
    /// comma-separated XKB options, e.g. "caps:escape,grp:alt_shift_toggle"
    pub options: Option<String>,
    /// ms before a held key starts repeating
    pub repeat_delay: Option<u32>,
    /// repeats per second
    pub repeat_rate: Option<u32>,
    pub numlock: Option<bool>,
}

impl KeyboardCfg {
    pub fn sets_layout(&self) -> bool {
        self.layout.is_some() || self.variant.is_some() || self.model.is_some() || self.options.is_some()
    }
}

/// The layout as the X server knows it: (rules, model, layout, variant, options).
pub struct Current {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
}

fn take(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { libc::free(p as *mut _) };
    s
}

pub fn current(dpy: *mut xlib::Display) -> Option<Current> {
    unsafe {
        let mut rules: *mut c_char = ptr::null_mut();
        let mut defs: VarDefs = std::mem::zeroed();
        if XkbRF_GetNamesProp(dpy, &mut rules, &mut defs) == 0 {
            return None;
        }
        Some(Current {
            rules: take(rules),
            model: take(defs.model),
            layout: take(defs.layout),
            variant: take(defs.variant),
            options: take(defs.options),
        })
    }
}

/// Apply everything in `cfg` that is set.
pub fn apply(dpy: *mut xlib::Display, cfg: &KeyboardCfg) -> Result<(), String> {
    if cfg.sets_layout() {
        set_layout(dpy, cfg)?;
    }
    unsafe {
        if cfg.repeat_delay.is_some() || cfg.repeat_rate.is_some() {
            let (mut delay, mut interval) = (0, 0);
            xlib::XkbGetAutoRepeatRate(dpy, XKB_USE_CORE_KBD, &mut delay, &mut interval);
            if let Some(d) = cfg.repeat_delay {
                delay = d.clamp(1, 5000);
            }
            if let Some(r) = cfg.repeat_rate {
                interval = 1000 / r.clamp(1, 1000);
            }
            xlib::XAutoRepeatOn(dpy);
            xlib::XkbSetAutoRepeatRate(dpy, XKB_USE_CORE_KBD, delay, interval);
        }
        if let Some(on) = cfg.numlock {
            let mask = xlib::XkbKeysymToModifiers(dpy, x11::keysym::XK_Num_Lock as xlib::KeySym);
            if mask != 0 {
                xlib::XkbLockModifiers(dpy, XKB_USE_CORE_KBD, mask, if on { mask } else { 0 });
            }
        }
        xlib::XSync(dpy, xlib::False);
    }
    Ok(())
}

/// A layout resolved through the XKB rules into the components the X server loads.
#[derive(Debug)]
pub struct Resolved {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub keycodes: String,
    pub types: String,
    pub compat: String,
    pub symbols: String,
}

fn cstr(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| "keyboard: settings must not contain NUL bytes".to_string())
}

/// Work out what `cfg` means (filling in unset parts from the current
/// settings) without changing anything.
pub fn resolve(dpy: *mut xlib::Display, cfg: &KeyboardCfg) -> Result<Resolved, String> {
    let cur = current(dpy).unwrap_or(Current {
        rules: "evdev".into(),
        model: "pc105".into(),
        layout: "us".into(),
        variant: String::new(),
        options: String::new(),
    });
    let rules_name = if cur.rules.is_empty() { "evdev".to_string() } else { cur.rules.clone() };
    let layout = cfg.layout.clone().unwrap_or(cur.layout);
    // a variant belongs to its layout: a new layout without variant gets none
    let variant = match (&cfg.variant, &cfg.layout) {
        (Some(v), _) => v.clone(),
        (None, Some(_)) => String::new(),
        (None, None) => cur.variant,
    };
    let model = cfg.model.clone().unwrap_or(if cur.model.is_empty() { "pc105".into() } else { cur.model });
    let options = cfg.options.clone().unwrap_or(cur.options);

    let root = XKB_ROOTS
        .iter()
        .find(|r| std::path::Path::new(&format!("{r}/rules/{rules_name}")).exists())
        .ok_or_else(|| format!("keyboard: XKB rules '{rules_name}' not found (is xkeyboard-config installed?)"))?;
    check_names(root, &layout, &variant)?;
    let path = cstr(&format!("{root}/rules/{rules_name}"))?;
    let locale = cstr("C")?;
    let (c_layout, c_variant, c_model, c_options) = (cstr(&layout)?, cstr(&variant)?, cstr(&model)?, cstr(&options)?);

    unsafe {
        let rules = XkbRF_Load(path.as_ptr() as *mut _, locale.as_ptr() as *mut _, xlib::False, xlib::True);
        if rules.is_null() {
            return Err(format!("keyboard: cannot load XKB rules {}", path.to_string_lossy()));
        }
        let mut defs = VarDefs {
            model: c_model.as_ptr() as *mut _,
            layout: c_layout.as_ptr() as *mut _,
            variant: if variant.is_empty() { ptr::null_mut() } else { c_variant.as_ptr() as *mut _ },
            options: if options.is_empty() { ptr::null_mut() } else { c_options.as_ptr() as *mut _ },
            sz_extra: 0,
            num_extra: 0,
            extra_names: ptr::null_mut(),
            extra_values: ptr::null_mut(),
        };
        let mut names: ComponentNames = std::mem::zeroed();
        let ok = XkbRF_GetComponents(rules, &mut defs, &mut names) != 0;
        XkbRF_Free(rules, xlib::True);
        let r = Resolved {
            rules: rules_name,
            model,
            layout,
            variant,
            options,
            keycodes: take(names.keycodes),
            types: take(names.types),
            compat: take(names.compat),
            symbols: take(names.symbols),
        };
        take(names.keymap);
        take(names.geometry);
        if !ok || r.symbols.is_empty() || r.keycodes.is_empty() {
            return Err(format!("keyboard: unknown layout '{}' / variant '{}' / options '{}'", r.layout, r.variant, r.options));
        }
        Ok(r)
    }
}

/// Make sure every layout (and variant) exists, so a typo gives a clear
/// error instead of a vague one from the X server.
fn check_names(root: &str, layout: &str, variant: &str) -> Result<(), String> {
    let variants: Vec<&str> = variant.split(',').collect();
    for (i, l) in layout.split(',').enumerate() {
        let l = l.trim();
        if l.is_empty() || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("keyboard: '{l}' is not a valid layout name"));
        }
        let file = format!("{root}/symbols/{l}");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return Err(format!("keyboard: unknown layout '{l}' (all layouts are listed in {root}/rules/base.lst)"));
        };
        let v = variants.get(i).map(|v| v.trim()).unwrap_or("");
        if !v.is_empty() && !text.contains(&format!("xkb_symbols \"{v}\"")) {
            return Err(format!("keyboard: layout '{l}' has no variant '{v}' (its variants are listed in {root}/rules/base.lst)"));
        }
    }
    Ok(())
}

/// What `setxkbmap -layout .. -variant .. -model .. -option ..` does.
fn set_layout(dpy: *mut xlib::Display, cfg: &KeyboardCfg) -> Result<(), String> {
    let r = resolve(dpy, cfg)?;
    // symbols must at least name one of the requested layouts, or something is off
    let first = r.layout.split(',').next().unwrap_or("");
    if !r.symbols.contains(first) {
        return Err(format!("keyboard: XKB resolved '{}' to unexpected symbols '{}'", r.layout, r.symbols));
    }
    let (kc, ty, co, sy) = (cstr(&r.keycodes)?, cstr(&r.types)?, cstr(&r.compat)?, cstr(&r.symbols)?);
    let mut names = ComponentNames {
        keymap: ptr::null_mut(),
        keycodes: kc.as_ptr() as *mut _,
        types: ty.as_ptr() as *mut _,
        compat: co.as_ptr() as *mut _,
        symbols: sy.as_ptr() as *mut _,
        geometry: ptr::null_mut(),
    };
    unsafe {
        let xkb = xlib::XkbGetKeyboardByName(
            dpy,
            XKB_USE_CORE_KBD,
            &mut names as *mut ComponentNames as xlib::XkbComponentNamesPtr,
            XKB_GBN_ALL,
            XKB_GBN_ALL & !XKB_GBN_GEOMETRY,
            xlib::True,
        );
        if xkb.is_null() {
            return Err(format!("keyboard: the X server rejected layout '{}' / variant '{}'", r.layout, r.variant));
        }
        xlib::XkbFreeKeyboard(xkb, XKB_ALL_COMPONENTS, xlib::True);
        // tell other programs (and `setxkbmap -query`) what is set
        let (c_rules, c_model, c_layout, c_variant, c_options) = (cstr(&r.rules)?, cstr(&r.model)?, cstr(&r.layout)?, cstr(&r.variant)?, cstr(&r.options)?);
        let mut defs = VarDefs {
            model: c_model.as_ptr() as *mut _,
            layout: c_layout.as_ptr() as *mut _,
            variant: c_variant.as_ptr() as *mut _,
            options: c_options.as_ptr() as *mut _,
            sz_extra: 0,
            num_extra: 0,
            extra_names: ptr::null_mut(),
            extra_values: ptr::null_mut(),
        };
        XkbRF_SetNamesProp(dpy, c_rules.as_ptr() as *mut _, &mut defs);
        xlib::XSync(dpy, xlib::False);
    }
    Ok(())
}

/// The active layout group (0 = first layout in the list).
pub fn group(dpy: *mut xlib::Display) -> usize {
    unsafe {
        let mut st: xlib::_XkbStateRec = std::mem::zeroed();
        xlib::XkbGetState(dpy, XKB_USE_CORE_KBD, &mut st);
        st.group as usize
    }
}

pub fn set_group(dpy: *mut xlib::Display, g: usize) {
    unsafe {
        xlib::XkbLockGroup(dpy, XKB_USE_CORE_KBD, g as c_uint);
        xlib::XSync(dpy, xlib::False);
    }
}

/// The layouts in the current keymap, e.g. ["us", "fr"].
pub fn layouts(dpy: *mut xlib::Display) -> Vec<String> {
    current(dpy).map(|c| c.layout.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

/// Name of the active layout, e.g. "fr".
pub fn active_name(dpy: *mut xlib::Display) -> Option<String> {
    let l = layouts(dpy);
    l.get(group(dpy)).or(l.first()).cloned().filter(|s| !s.is_empty())
}

/// Split "us, fr" or a Lua list into the comma form XKB wants.
pub fn join(parts: &[String]) -> String {
    parts.iter().map(|s| s.trim()).collect::<Vec<_>>().join(",")
}
