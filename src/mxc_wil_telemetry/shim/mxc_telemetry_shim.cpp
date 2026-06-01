// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// MXC telemetry shim — WIL-based TraceLogging provider.
//
// This file implements the same pattern used by WinAppSDK
// (WindowsAppRuntimeInsights.h / DeploymentTraceLogging.h):
//
//   1. Provider class via IMPLEMENT_TRACELOGGING_CLASS
//   2. Part B common fields on every event (_MXC_GENERIC_PARTB_FIELDS)
//   3. UTCReplace_AppSessionGuid for privacy
//   4. Flat extern "C" surface for Rust FFI
//
// WIL is header-only (MIT licensed), acquired from NuGet at build time.

#include <windows.h>
#include <synchapi.h>
#include <evntrace.h>
#include <TraceLoggingProvider.h>
#include <wil/tracelogging.h>
#include <wil/resource.h>
#include <string>
#include <cstring>

#include "mxc_telemetry_shim.h"

// ---------------------------------------------------------------------------
// Provider definition
// ---------------------------------------------------------------------------

// The provider GUID below is a PLACEHOLDER for the open-source build.
// Internal Microsoft builds replace this GUID at packaging time with the
// production GUID registered in the telemetry pipeline. This mirrors the
// WinAppSDK pattern described in the WinAppSDK Telemetry spec.
//
// The provider group GUID (set via TraceLoggingOptionMicrosoftTelemetry())
// identifies this as a Microsoft first-party TraceLogging telemetry provider.
// It is part of the IMPLEMENT_TRACELOGGING_CLASS macro when using the
// *MicrosoftTelemetry variant.

class MxcTelemetryProvider final : public wil::TraceLoggingProvider
{
    IMPLEMENT_TRACELOGGING_CLASS(
        MxcTelemetryProvider,
        "Microsoft.MXC",
        // Placeholder provider GUID for OSS — (4f50731a-89cf-4782-b3e0-dce8c90476ba)
        // Replace at packaging for internal builds.
        (0x4f50731a, 0x89cf, 0x4782, 0xb3, 0xe0, 0xdc, 0xe8, 0xc9, 0x04, 0x76, 0xba),
        TraceLoggingOptionMicrosoftTelemetry());
};

// ---------------------------------------------------------------------------
// Cached state — set at init, read on every event.
// Protected by an SRWLOCK for thread safety across the FFI boundary.
// ---------------------------------------------------------------------------

static SRWLOCK s_lock = SRWLOCK_INIT;
static std::string s_version;
static std::string s_channel;

// ---------------------------------------------------------------------------
// Part B common fields macro
// ---------------------------------------------------------------------------
// Modelled on WinAppSDK's _GENERIC_PARTB_FIELDS_ENABLED.
// Every event includes:
//   - Version:  MXC crate version (e.g., "0.3.0")
//   - Channel:  build channel ("dev" or "release")
//   - IsDebugging:  whether a debugger is attached
//   - UTCReplace_AppSessionGuid:  tells UTC to replace the app session GUID
//     for privacy (per-session GUID instead of persistent identifier)
//
// Callers must snapshot s_version/s_channel under the read lock and pass
// them as local variables `snap_version` and `snap_channel`.

#define _MXC_GENERIC_PARTB_FIELDS \
    TraceLoggingStruct(4, "COMMON_MXC_PARAMS"), \
    TraceLoggingString(snap_version.c_str(), "Version"), \
    TraceLoggingString(snap_channel.c_str(), "Channel"), \
    TraceLoggingBool(!!IsDebuggerPresent(), "IsDebugging"), \
    TraceLoggingBool(true, "UTCReplace_AppSessionGuid")

// ---------------------------------------------------------------------------
// extern "C" API for Rust FFI
// ---------------------------------------------------------------------------

extern "C" bool mxc_telemetry_init(const char* version, const char* channel)
{
    if (!version || !channel)
    {
        return false;
    }

    AcquireSRWLockExclusive(&s_lock);
    s_version = version;
    s_channel = channel;
    ReleaseSRWLockExclusive(&s_lock);

    // Provider registration happens lazily via WIL's singleton pattern
    // on first TraceLoggingWrite. Return true to indicate init succeeded
    // (IsEnabled() only checks if an ETW session is listening, which is
    // orthogonal to whether the provider is correctly registered).
    return true;
}

extern "C" void mxc_telemetry_shutdown()
{
    // WIL providers are process-lifetime singletons — they unregister
    // automatically at DLL/EXE unload. This function is provided for
    // symmetry with the Rust API and to allow explicit cleanup.
    AcquireSRWLockExclusive(&s_lock);
    s_version.clear();
    s_channel.clear();
    ReleaseSRWLockExclusive(&s_lock);
}

extern "C" void mxc_telemetry_log_execution(
    const char* backend,
    int exit_code,
    const char* outcome,
    unsigned long long duration_ms,
    const char* failure_reason)
{
    if (!MxcTelemetryProvider::IsEnabled())
    {
        return;
    }

    const char* safe_backend = backend ? backend : "";
    const char* safe_outcome = outcome ? outcome : "";
    const char* safe_failure = failure_reason ? failure_reason : "";

    // Snapshot cached strings under the shared (read) lock so we don't
    // race with init/shutdown which acquire the exclusive (write) lock.
    AcquireSRWLockShared(&s_lock);
    std::string snap_version = s_version;
    std::string snap_channel = s_channel;
    ReleaseSRWLockShared(&s_lock);

    TraceLoggingWrite(
        MxcTelemetryProvider::Provider(),
        "MXC.Execution",
        TraceLoggingKeyword(MICROSOFT_KEYWORD_MEASURES),
        TraceLoggingLevel(TRACE_LEVEL_INFORMATION),
        // Part B common fields
        _MXC_GENERIC_PARTB_FIELDS,
        // Event-specific fields
        TraceLoggingString(safe_backend, "mxc.backend"),
        TraceLoggingInt32(exit_code, "mxc.exit_code"),
        TraceLoggingString(safe_outcome, "mxc.outcome"),
        TraceLoggingUInt64(duration_ms, "mxc.duration_ms"),
        TraceLoggingString(safe_failure, "mxc.failure_reason"));
}

extern "C" void mxc_telemetry_log_error(
    const char* backend,
    const char* error_type,
    const char* error_message)
{
    if (!MxcTelemetryProvider::IsEnabled())
    {
        return;
    }

    const char* safe_backend = backend ? backend : "";
    const char* safe_type = error_type ? error_type : "";
    const char* safe_message = error_message ? error_message : "";

    // Snapshot cached strings under the shared (read) lock.
    AcquireSRWLockShared(&s_lock);
    std::string snap_version = s_version;
    std::string snap_channel = s_channel;
    ReleaseSRWLockShared(&s_lock);

    TraceLoggingWrite(
        MxcTelemetryProvider::Provider(),
        "MXC.Error",
        TraceLoggingKeyword(MICROSOFT_KEYWORD_MEASURES),
        TraceLoggingLevel(TRACE_LEVEL_WARNING),
        // Part B common fields
        _MXC_GENERIC_PARTB_FIELDS,
        // Event-specific fields
        TraceLoggingString(safe_backend, "mxc.backend"),
        TraceLoggingString(safe_type, "mxc.error_type"),
        TraceLoggingString(safe_message, "mxc.error_message"));
}
