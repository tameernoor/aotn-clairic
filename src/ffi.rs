//! C ABI, so the library can be loaded as a DLL / .so / .dylib from C#, Python or anything
//! else that can call C. Credentials come from CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN.

use std::ffi::{CString, c_char};

use anyhow::Result;
use serde::Serialize;
use serde_json::json;

use crate::Client;

/// Flag one raw email, given as its bytes exactly as stored (an .eml file or an IMAP fetch).
/// Returns a JSON string, either the result or `{"error": "..."}`. Free it with `clairic_free`.
///
/// # Safety
/// `raw` must point to `len` readable bytes, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clairic_flag_dispute(raw: *const u8, len: usize) -> *mut c_char {
    unsafe { call(raw, len, |c, b| c.triage_eml(b)) }
}

/// Read one photo of an invoice (PNG, JPEG or WebP bytes). Returns a JSON string, either the
/// result or `{"error": "..."}`. Free it with `clairic_free`.
///
/// # Safety
/// `raw` must point to `len` readable bytes, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clairic_read_invoice(raw: *const u8, len: usize) -> *mut c_char {
    unsafe { call(raw, len, |c, b| c.read_invoice(b)) }
}

/// Free a string returned by this library.
///
/// # Safety
/// `s` must come from this library and not be freed twice, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clairic_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: `s` came from `CString::into_raw` in this library.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// # Safety
/// `raw` must point to `len` readable bytes, or be null.
unsafe fn call<T: Serialize>(raw: *const u8, len: usize, f: impl Fn(&Client, &[u8]) -> Result<T>) -> *mut c_char {
    let out = if raw.is_null() {
        json!({ "error": "null input" })
    } else {
        // SAFETY: the caller passes `len` readable bytes at `raw`.
        let bytes = unsafe { std::slice::from_raw_parts(raw, len) };
        // A panic must not cross into the calling program, so it becomes an error instead.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Client::from_env().and_then(|c| f(&c, bytes))));
        match result {
            Ok(Ok(t)) => serde_json::to_value(t).unwrap_or_else(|e| json!({ "error": e.to_string() })),
            Ok(Err(e)) => json!({ "error": format!("{e:#}") }),
            Err(_) => json!({ "error": "internal error" }),
        }
    };
    CString::new(out.to_string()).unwrap_or_default().into_raw()
}
