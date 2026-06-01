// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// MXC telemetry shim — C++ layer that wraps WIL TraceLogging helpers
// and exposes a flat extern "C" API for Rust FFI.
//
// Design follows the WinAppSDK pattern:
//   - Provider class inherits wil::TraceLoggingProvider
//   - Every event includes Part B common fields (version, channel,
//     IsDebugging, UTCReplace_AppSessionGuid)
//   - Provider GUID is a placeholder for OSS — replaced at packaging
//     for internal builds

#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Register the ETW provider and store the version/channel strings.
/// Returns true on success.
bool mxc_telemetry_init(const char* version, const char* channel);

/// Unregister the ETW provider.
void mxc_telemetry_shutdown(void);

/// Emit an MXC.Execution event.
void mxc_telemetry_log_execution(
    const char* backend,
    int exit_code,
    const char* outcome,
    unsigned long long duration_ms,
    const char* failure_reason);

/// Emit an MXC.Error event.
void mxc_telemetry_log_error(
    const char* backend,
    const char* error_type,
    const char* error_message);

#ifdef __cplusplus
}
#endif
