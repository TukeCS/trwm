//! Minimal PAM bindings: just enough to check the user's password.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_REFRESH_CRED: c_int = 0x0010;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut c_void) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
}

extern "C" fn conversation(n: c_int, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msg.is_null() || resp.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    unsafe {
        let out = libc::calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if out.is_null() {
            return PAM_BUF_ERR;
        }
        let password = data as *const c_char;
        for i in 0..n as usize {
            let m = *msg.add(i);
            let style = if m.is_null() { 0 } else { (*m).msg_style };
            if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
                (*out.add(i)).resp = libc::strdup(password);
            }
        }
        *resp = out;
    }
    PAM_SUCCESS
}

pub fn username() -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return std::env::var("USER").ok();
        }
        Some(CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
    }
}

/// Returns true if `password` is the current user's password.
pub fn authenticate(service: &str, user: &str, password: &[u8]) -> bool {
    let (Ok(service), Ok(user)) = (CString::new(service), CString::new(user)) else { return false };
    let mut pw = password.to_vec();
    pw.push(0);
    if pw[..pw.len() - 1].contains(&0) {
        wipe(&mut pw);
        return false;
    }
    let conv = PamConv { conv: conversation, appdata_ptr: pw.as_ptr() as *mut c_void };
    let mut pamh = ptr::null_mut();
    let ok = unsafe {
        let mut r = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut pamh);
        if r == PAM_SUCCESS {
            r = pam_authenticate(pamh, 0);
            if r == PAM_SUCCESS {
                pam_setcred(pamh, PAM_REFRESH_CRED);
            }
        } else {
            eprintln!("trwm lock: pam_start failed ({r})");
        }
        if !pamh.is_null() {
            pam_end(pamh, r);
        }
        r == PAM_SUCCESS
    };
    wipe(&mut pw);
    ok
}

/// Overwrite a buffer in a way the optimiser can't elide.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_answers_password_prompts() {
        let pw = CString::new("hunter2").unwrap();
        let prompt = CString::new("Password: ").unwrap();
        let msgs = [PamMessage { msg_style: PAM_PROMPT_ECHO_OFF, msg: prompt.as_ptr() }, PamMessage { msg_style: 4 /* PAM_TEXT_INFO */, msg: prompt.as_ptr() }];
        let mut ptrs: Vec<*const PamMessage> = msgs.iter().map(|m| m as *const _).collect();
        let mut resp: *mut PamResponse = ptr::null_mut();
        let r = conversation(2, ptrs.as_mut_ptr(), &mut resp, pw.as_ptr() as *mut c_void);
        assert_eq!(r, PAM_SUCCESS);
        unsafe {
            let r0 = &*resp;
            assert_eq!(CStr::from_ptr(r0.resp).to_str().unwrap(), "hunter2");
            assert!((*resp.add(1)).resp.is_null());
            libc::free(r0.resp as *mut c_void);
            libc::free(resp as *mut c_void);
        }
    }
}
