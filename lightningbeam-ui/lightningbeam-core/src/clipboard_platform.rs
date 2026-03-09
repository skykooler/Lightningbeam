//! Platform-native clipboard integration for custom MIME types.
//!
//! > **Temporary shim** — this module exists because arboard does not yet support
//! > custom MIME types.  Once arboard gains that capability (tracked at
//! > <https://github.com/1Password/arboard/issues/14>) this module and all
//! > platform-conditional deps (`objc2*`, `windows-sys`, `wl-clipboard-rs`,
//! > `x11-clipboard`) should be removed and replaced with a single arboard call.
//!
//! Provides [`set`] and [`get`] functions for reading and writing non-text
//! clipboard formats directly via each platform's native clipboard API.
//!
//! # Platform notes
//! - **macOS**: NSPasteboard via objc2 — appends entries to the clipboard that
//!   arboard already opened; must be called *after* `arboard::set_text()` /
//!   `set_image()` since arboard calls `clearContents` internally.
//! - **Windows**: `RegisterClipboardFormat` + `SetClipboardData` — appends to
//!   the clipboard arboard already populated; must be called *after* arboard.
//! - **Linux/Wayland**: `wl-clipboard-rs` — creates its own Wayland connection
//!   and spawns a background thread to serve clipboard requests; no external
//!   tools required.
//! - **Linux/X11**: `x11-clipboard` — serves custom-atom requests via its
//!   background thread; only the first entry is set (X11 single-target
//!   limitation per selection).

/// Set one or more `(mime_type, data)` pairs on the platform clipboard.
///
/// On macOS and Windows this must be called **after** `arboard::Clipboard::set_text()` /
/// `set_image()` because arboard empties the clipboard first.
///
/// On Linux/X11 only the first entry is used.
pub fn set(entries: &[(&str, &[u8])]) {
    platform_impl::set(entries);
}

/// Return the first available `(mime_type, data)` pair from the platform clipboard.
///
/// `preferred` is tried in order; the first MIME type with data wins.
/// Returns `None` when none of the requested types are present.
pub fn get(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
    platform_impl::get(preferred)
}

// ─────────────────────────────────── macOS ──────────────────────────────────

#[cfg(target_os = "macos")]
mod platform_impl {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};

    pub fn set(entries: &[(&str, &[u8])]) {
        let pb = NSPasteboard::generalPasteboard();
        for &(mime, data) in entries {
            let ns_type = NSString::from_str(mime);
            let ns_data = NSData::with_bytes(data);
            // setData:forType: appends to the current clipboard contents
            // (arboard already called clearContents, so no double-clear needed).
            pb.setData_forType(Some(&ns_data), &ns_type);
        }
    }

    pub fn get(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        let pb = NSPasteboard::generalPasteboard();
        for &mime in preferred {
            let ns_type = NSString::from_str(mime);
            if let Some(ns_data) = pb.dataForType(&ns_type) {
                let len = ns_data.length();
                // SAFETY: bytes() is valid for length() bytes per NSData contract.
                let bytes = unsafe {
                    std::slice::from_raw_parts(ns_data.bytes() as *const u8, len).to_vec()
                };
                return Some((mime.to_string(), bytes));
            }
        }
        None
    }
}

// ─────────────────────────────────── Windows ────────────────────────────────

#[cfg(target_os = "windows")]
mod platform_impl {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };

    static FORMAT_IDS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

    /// Register (or look up) the clipboard format ID for a MIME-type string.
    fn registered_format(mime: &str) -> u32 {
        let ids = FORMAT_IDS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = ids.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = guard.get(mime) {
            return id;
        }
        // RegisterClipboardFormatW requires a null-terminated UTF-16 string.
        let wide: Vec<u16> = mime.encode_utf16().chain(std::iter::once(0)).collect();
        let id = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
        guard.insert(mime.to_string(), id);
        id
    }

    pub fn set(entries: &[(&str, &[u8])]) {
        // arboard already called EmptyClipboard; we just append new formats.
        // OpenClipboard(NULL) acquires ownership without clearing.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return;
            }
            for &(mime, data) in entries {
                let fmt = registered_format(mime);
                let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
                if h.is_null() {
                    continue;
                }
                let ptr = GlobalLock(h);
                if ptr.is_null() {
                    // Cannot free `h` here: GlobalFree was removed from windows-sys 0.60
                    // (it still exists in Kernel32.dll, so a manual extern declaration
                    // would work if this ever becomes an issue).  The leak is bounded to
                    // one clipboard-payload-sized allocation and only occurs if GlobalLock
                    // fails on a handle we just allocated — essentially impossible in
                    // practice.
                    continue;
                }
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
                GlobalUnlock(h);
                SetClipboardData(fmt, h as HANDLE);
            }
            CloseClipboard();
        }
    }

    pub fn get(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let mut result = None;
            for &mime in preferred {
                let fmt = registered_format(mime);
                let h = GetClipboardData(fmt);
                if h.is_null() {
                    continue;
                }
                let ptr = GlobalLock(h as _);
                if ptr.is_null() {
                    continue;
                }
                let size = GlobalSize(h as _);
                let data = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                GlobalUnlock(h as _);
                result = Some((mime.to_string(), data));
                break;
            }
            CloseClipboard();
            result
        }
    }
}

// ─────────────────────────────────── Linux ──────────────────────────────────

#[cfg(target_os = "linux")]
mod platform_impl {
    pub fn set(entries: &[(&str, &[u8])]) {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            set_wayland(entries);
        } else {
            set_x11(entries);
        }
    }

    pub fn get(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            get_wayland(preferred)
        } else {
            get_x11(preferred)
        }
    }

    // ── Wayland ──────────────────────────────────────────────────────────────

    fn set_wayland(entries: &[(&str, &[u8])]) {
        use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

        let sources: Vec<MimeSource> = entries
            .iter()
            .map(|&(mime, data)| MimeSource {
                source: Source::Bytes(data.to_vec().into_boxed_slice()),
                mime_type: MimeType::Specific(mime.to_string()),
            })
            .collect();

        // copy_multi spawns a background thread that serves clipboard requests
        // until another client takes ownership — no blocking, no subprocess needed.
        if let Err(e) = Options::new().copy_multi(sources) {
            eprintln!("[clipboard_platform] wl-clipboard-rs set error: {e}");
        }
    }

    fn get_wayland(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        use std::io::Read;
        use wl_clipboard_rs::paste::{self, ClipboardType, Error, MimeType, Seat};

        for &mime in preferred {
            match paste::get_contents(
                ClipboardType::Regular,
                Seat::Unspecified,
                MimeType::Specific(mime),
            ) {
                Ok((mut pipe, _)) => {
                    let mut buf = Vec::new();
                    if pipe.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                        return Some((mime.to_string(), buf));
                    }
                }
                // These are non-error "not present" conditions — try the next type.
                Err(Error::ClipboardEmpty) | Err(Error::NoMimeType) => continue,
                Err(e) => {
                    eprintln!("[clipboard_platform] wl-clipboard-rs get error for {mime}: {e}");
                    continue;
                }
            }
        }
        None
    }

    // ── X11 ──────────────────────────────────────────────────────────────────

    use std::sync::Mutex;
    use std::time::Duration;

    /// Keeps the x11-clipboard instance alive so its background thread can
    /// continue serving SelectionRequest events.  Replaced on each `set_x11` call.
    static X11_CB: Mutex<Option<x11_clipboard::Clipboard>> = Mutex::new(None);

    fn set_x11(entries: &[(&str, &[u8])]) {
        // X11 clipboard can only serve one target atom per selection owner.
        // Use the first entry (the custom LB MIME type).
        let Some(&(mime, data)) = entries.first() else { return };

        let cb = match x11_clipboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[clipboard_platform] x11-clipboard init error: {e}");
                return;
            }
        };

        let atom = match cb.setter.get_atom(mime) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[clipboard_platform] x11-clipboard intern atom error for {mime}: {e}");
                return;
            }
        };

        if let Err(e) = cb.store(cb.setter.atoms.clipboard, atom, data.to_vec()) {
            eprintln!("[clipboard_platform] x11-clipboard store error: {e}");
            return;
        }

        // Keep alive to serve requests; replacing drops the previous instance.
        *X11_CB.lock().unwrap_or_else(|e| e.into_inner()) = Some(cb);
    }

    fn get_x11(preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        let cb = match x11_clipboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[clipboard_platform] x11-clipboard init error: {e}");
                return None;
            }
        };

        for &mime in preferred {
            let atom = match cb.getter.get_atom(mime) {
                Ok(a) => a,
                Err(_) => continue,
            };

            match cb.load(
                cb.getter.atoms.clipboard,
                atom,
                cb.getter.atoms.property,
                Duration::from_secs(1),
            ) {
                Ok(data) if !data.is_empty() => return Some((mime.to_string(), data)),
                _ => continue,
            }
        }
        None
    }
}

// ──────────────────────────── Fallback (other OS) ───────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform_impl {
    pub fn set(_entries: &[(&str, &[u8])]) {}
    pub fn get(_preferred: &[&str]) -> Option<(String, Vec<u8>)> {
        None
    }
}
