// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! WIL-based TraceLogging ETW telemetry for MXC.
//!
//! This crate provides a Rust-safe interface to MXC's C++ telemetry shim,
//! which uses the Windows Implementation Library (WIL) `TraceLoggingProvider`
//! class — the same pattern used by WinAppSDK.
//!
//! # Platform behaviour
//!
//! - **Windows**: Functions call into the compiled C++ shim via FFI, which
//!   emits ETW events with Part B common fields and `UTCReplace_AppSessionGuid`.
//! - **Non-Windows**: All functions are no-ops — telemetry is a Windows-only feature.
//!
//! # Thread safety
//!
//! The underlying C++ shim protects shared state (`s_version`, `s_channel`)
//! with an SRWLOCK — readers (event loggers) take a shared lock while writers
//! (`init`, `shutdown`) take an exclusive lock. The FFI functions are safe to
//! call from any thread.

use std::ffi::CString;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `&str` to a `CString`, truncating at the first interior NUL byte
/// instead of dropping the entire string.
fn to_cstring_lossy(s: &str) -> CString {
    match CString::new(s) {
        Ok(c) => c,
        Err(e) => {
            let pos = e.nul_position();
            // The prefix up to the first NUL is guaranteed NUL-free.
            CString::new(&s[..pos]).unwrap_or_default()
        }
    }
}

// ---------------------------------------------------------------------------
// FFI declarations (Windows only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
extern "C" {
    fn mxc_telemetry_init(
        version: *const std::ffi::c_char,
        channel: *const std::ffi::c_char,
    ) -> bool;
    fn mxc_telemetry_shutdown();
    fn mxc_telemetry_log_execution(
        backend: *const std::ffi::c_char,
        exit_code: i32,
        outcome: *const std::ffi::c_char,
        duration_ms: u64,
        failure_reason: *const std::ffi::c_char,
    );
    fn mxc_telemetry_log_error(
        backend: *const std::ffi::c_char,
        error_type: *const std::ffi::c_char,
        error_message: *const std::ffi::c_char,
    );
}

// ---------------------------------------------------------------------------
// Safe Rust wrappers
// ---------------------------------------------------------------------------

/// Initialize the WIL TraceLogging provider.
///
/// Must be called once at process startup. Returns `true` if the provider
/// was successfully registered, `false` if arguments were invalid or on
/// non-Windows platforms.
///
/// Note: A `true` return means the provider is registered — it does *not*
/// guarantee an ETW session is actively listening.
///
/// # Arguments
///
/// * `version` — MXC version string (e.g., from `CARGO_PKG_VERSION`)
/// * `channel` — Build channel (`"dev"` or `"release"`)
pub fn init(version: &str, channel: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let c_version = to_cstring_lossy(version);
        let c_channel = to_cstring_lossy(channel);
        // SAFETY: The C++ shim copies the strings into `std::string` storage
        // under an exclusive lock. The CStrings remain valid for the call.
        unsafe { mxc_telemetry_init(c_version.as_ptr(), c_channel.as_ptr()) }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (version, channel);
        false
    }
}

/// Shut down the WIL TraceLogging provider.
///
/// Should be called before process exit for clean resource release,
/// although WIL providers auto-unregister at process termination.
pub fn shutdown() {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: The C++ shim clears cached strings under an exclusive lock;
        // the WIL provider singleton handles its own thread-safe cleanup.
        unsafe {
            mxc_telemetry_shutdown();
        }
    }
}

/// Emit an `MXC.Execution` ETW event with Part B common fields.
///
/// All string arguments are copied by the C++ shim — no lifetime
/// requirements beyond the function call.
pub fn log_execution(
    backend: &str,
    exit_code: i32,
    outcome: &str,
    duration_ms: u64,
    failure_reason: &str,
) {
    #[cfg(target_os = "windows")]
    {
        let c_backend = to_cstring_lossy(backend);
        let c_outcome = to_cstring_lossy(outcome);
        let c_failure = to_cstring_lossy(failure_reason);
        // SAFETY: All pointers are valid CStrings; the C++ shim reads them
        // synchronously under a shared lock and does not retain references.
        unsafe {
            mxc_telemetry_log_execution(
                c_backend.as_ptr(),
                exit_code,
                c_outcome.as_ptr(),
                duration_ms,
                c_failure.as_ptr(),
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (backend, exit_code, outcome, duration_ms, failure_reason);
    }
}

/// Emit an `MXC.Error` ETW event with Part B common fields.
pub fn log_error(backend: &str, error_type: &str, error_message: &str) {
    #[cfg(target_os = "windows")]
    {
        let c_backend = to_cstring_lossy(backend);
        let c_type = to_cstring_lossy(error_type);
        let c_message = to_cstring_lossy(error_message);
        // SAFETY: All pointers are valid CStrings; the C++ shim reads them
        // synchronously under a shared lock and does not retain references.
        unsafe {
            mxc_telemetry_log_error(c_backend.as_ptr(), c_type.as_ptr(), c_message.as_ptr());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (backend, error_type, error_message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The C++ shim uses global state protected by an SRWLOCK, so tests that
    /// call `init`/`shutdown` must not run concurrently. This mutex serialises
    /// all FFI-touching tests without adding an external crate dependency.
    static FFI_LOCK: Mutex<()> = Mutex::new(());

    // -------------------------------------------------------------------
    // to_cstring_lossy tests (pure Rust, no FFI — can run in parallel)
    // -------------------------------------------------------------------

    #[test]
    fn to_cstring_lossy_normal_string() {
        let c = to_cstring_lossy("hello");
        assert_eq!(c.to_bytes(), b"hello");
    }

    #[test]
    fn to_cstring_lossy_interior_nul() {
        let c = to_cstring_lossy("ab\0cd");
        assert_eq!(c.to_bytes(), b"ab");
    }

    #[test]
    fn to_cstring_lossy_empty_string() {
        let c = to_cstring_lossy("");
        assert_eq!(c.to_bytes(), b"");
    }

    #[test]
    fn to_cstring_lossy_leading_nul() {
        let c = to_cstring_lossy("\0rest");
        assert_eq!(c.to_bytes(), b"");
    }

    // -------------------------------------------------------------------
    // FFI lifecycle tests (serialised via FFI_LOCK)
    // -------------------------------------------------------------------

    #[test]
    fn init_shutdown_roundtrip() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ok = init("0.0.0-test", "dev");
        // On Windows this registers the ETW provider; on other platforms
        // it returns false (no-op).
        if cfg!(target_os = "windows") {
            assert!(ok, "init should succeed on Windows");
        } else {
            assert!(!ok, "init should be a no-op on non-Windows");
        }
        shutdown();
    }

    #[test]
    fn double_init_is_safe() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = init("0.0.0-test", "dev");
        let _ = init("0.0.0-test", "dev");
        shutdown();
    }

    #[test]
    fn shutdown_without_init() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Must not panic or crash.
        shutdown();
    }

    #[test]
    fn log_execution_after_init() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = init("0.0.0-test", "dev");
        log_execution("test_backend", 0, "success", 100, "");
        shutdown();
    }

    #[test]
    fn log_error_after_init() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = init("0.0.0-test", "dev");
        log_error("test_backend", "config_error", "test error message");
        shutdown();
    }

    #[test]
    fn log_without_init() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Must be safe no-ops — no init called.
        log_execution("test_backend", 0, "success", 50, "none");
        log_error("test_backend", "unknown", "no init");
    }

    #[test]
    fn log_after_shutdown() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = init("0.0.0-test", "dev");
        shutdown();
        // Must be safe no-ops — provider already unregistered.
        log_execution("test_backend", 1, "failure", 200, "timeout");
        log_error("test_backend", "process_error", "after shutdown");
    }

    #[test]
    fn handles_empty_strings() {
        let _lock = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = init("", "");
        log_execution("", 0, "", 0, "");
        log_error("", "", "");
        shutdown();
    }
}
