# MXC Telemetry — WIL Integration Architecture

MXC uses the Windows Implementation Library (WIL) for TraceLogging ETW
telemetry, following the same pattern established by WinAppSDK.

## Overview

```
┌──────────────────────────────────────────────────────┐
│  wxc_common::telemetry                               │
│  (Rust — pure logic: config, sanitisation, types)    │
│                                                      │
│  init() / log_execution() / log_error() / shutdown() │
└───────────────┬──────────────────────────────────────┘
                │  FFI calls (CString → *const c_char)
                ▼
┌──────────────────────────────────────────────────────┐
│  mxc_wil_telemetry (Rust crate)                      │
│  src/lib.rs — safe wrappers + #[cfg] platform guards │
│                                                      │
│  Windows: extern "C" → C++ shim                      │
│  Linux/macOS: no-op stubs                            │
└───────────────┬──────────────────────────────────────┘
                │  Static linking
                ▼
┌──────────────────────────────────────────────────────┐
│  C++ shim (mxc_telemetry_shim.cpp)                   │
│  Compiled by cc crate in build.rs                    │
│                                                      │
│  MxcTelemetryProvider : wil::TraceLoggingProvider     │
│  ├── IMPLEMENT_TRACELOGGING_CLASS(...)               │
│  ├── TraceLoggingOptionMicrosoftTelemetry()           │
│  └── _MXC_GENERIC_PARTB_FIELDS on every event        │
│       ├── Version                                    │
│       ├── Channel ("dev" / "release")                │
│       ├── IsDebugging                                │
│       └── UTCReplace_AppSessionGuid = true           │
└───────────────┬──────────────────────────────────────┘
                │  Links against WIL headers (header-only)
                ▼
┌──────────────────────────────────────────────────────┐
│  WIL (Microsoft.Windows.ImplementationLibrary)       │
│  Downloaded from NuGet at build time                 │
│  MIT licensed, header-only                           │
│  Version: 1.0.260126.7                                │
└──────────────────────────────────────────────────────┘
```

## WIL Acquisition

The `build.rs` in `mxc_wil_telemetry` downloads the WIL NuGet package
(which is just a `.zip` file) and extracts the `include/wil/` headers.
The download is cached under `OUT_DIR/wil-cache/` so it only happens
once per clean build.

**No NuGet CLI or .NET SDK required** — the `.nupkg` is fetched via
HTTP and unzipped directly.

## Part B Common Fields

Following WinAppSDK's `_GENERIC_PARTB_FIELDS_ENABLED` macro, every MXC
telemetry event includes a `COMMON_MXC_PARAMS` struct with:

| Field | Type | Description |
|-------|------|-------------|
| `Version` | string | MXC crate version from `CARGO_PKG_VERSION` |
| `Channel` | string | `"dev"` for debug builds, `"release"` for release |
| `IsDebugging` | bool | `IsDebuggerPresent()` at event emission time |
| `UTCReplace_AppSessionGuid` | bool | Always `true` — tells UTC to replace the app session GUID with a per-session identifier for privacy |

## Provider GUID

The provider GUID in `mxc_telemetry_shim.cpp` is a **placeholder** for
the open-source build. Internal Microsoft builds replace this GUID at
packaging time with the production GUID registered in the telemetry
pipeline.

This follows the WinAppSDK pattern described in the WinAppSDK Telemetry
spec (per guidance from Mythilli Srinivasan).

The **provider group GUID** (set via `TraceLoggingOptionMicrosoftTelemetry()`)
identifies this as a Microsoft first-party TraceLogging telemetry provider.
This is a well-known GUID and is the same across all Microsoft products.

## Events

### MXC.Execution

Emitted on every sandbox execution completion.

| Field | Type | Description |
|-------|------|-------------|
| `mxc.backend` | string | Containment backend name |
| `mxc.exit_code` | int32 | Process exit code |
| `mxc.outcome` | string | `"success"` or `"failure"` |
| `mxc.duration_ms` | uint64 | Total execution time |
| `mxc.failure_reason` | string | Failure category (if applicable) |

### MXC.Error

Emitted on execution errors.

| Field | Type | Description |
|-------|------|-------------|
| `mxc.backend` | string | Containment backend name |
| `mxc.error_type` | string | Error category (`config_error`, `process_error`, etc.) |
| `mxc.error_message` | string | Sanitized error message (PII-stripped, max 256 chars) |

## Cross-Platform Behaviour

| Platform | Behaviour |
|----------|-----------|
| Windows | Full ETW telemetry via WIL C++ shim |
| Linux | No-op — all telemetry functions return immediately |
| macOS | No-op — all telemetry functions return immediately |

## Private GUID Substitution (Internal Builds)

MXC follows the same private telemetry GUID substitution pattern as WinAppSDK.
The mechanism is public; only the GUID value is private.

### Background

The public WIL NuGet package ships `wil/traceloggingconfig.h` with
`TraceLoggingOptionMicrosoftTelemetry()` defined as an **empty macro** (no-op).
This means community/OSS builds emit plain ETW events with no Microsoft
pipeline routing — which is the correct behaviour for external users.

Internal Microsoft builds need the **real Microsoft telemetry group GUID**
compiled in so events are tagged as Microsoft first-party TraceLogging telemetry.
This GUID lives in `MicrosoftTelemetry.h` inside the private
`Microsoft.Telemetry.Inbox.Native` NuGet package.

### How it works

```
build.rs execution flow
========================

1. Download WIL NuGet from nuget.org
2. Extract headers to OUT_DIR/wil-cache/include/
   └── wil/traceloggingconfig.h  ← PUBLIC (empty macro)

3. Check MXC_TELEMETRY_CONFIG_OVERRIDE env var
   ├── NOT set → keep public stub (community build)
   └── SET → copy the file it points to over traceloggingconfig.h
              └── wil/traceloggingconfig.h  ← NOW has real GUID

4. cc::Build compiles mxc_telemetry_shim.cpp
   └── #include <wil/tracelogging.h>
       └── #include <wil/traceloggingconfig.h>  ← whichever version is on disk
```

### CI pipeline steps

The Azure Pipelines build (`.azure-pipelines/templates/Rust.Build.Job.yml`)
adds three steps before `cargo build` on Windows:

1. **`NuGetAuthenticate@1`** — authenticates to the `TelemetryInternal` service
   connection (an ADO service connection providing credentials to the private feed)
2. **`NuGetCommand@2`** — restores `Microsoft.Telemetry.Inbox.Native` from
   `build/telemetry/packages.config` into `build/telemetry/packages/`
3. **PowerShell** — finds `MicrosoftTelemetry.h` in the restored packages and
   sets `MXC_TELEMETRY_CONFIG_OVERRIDE` to its path

These steps do **not** use `continueOnError` — they hard-fail if the private
feed is unavailable, matching the WinAppSDK pattern. Community forks that lack
access to the private feed should not add these pipeline steps; the public
WIL headers are used by default when the env var is unset.

### Local developer testing

If you have access to `Microsoft.Telemetry.Inbox.Native`, you can test the
override locally:

```powershell
# Restore the package manually
nuget restore build\telemetry\packages.config -PackagesDirectory build\telemetry\packages

# Point build.rs to the private header
$env:MXC_TELEMETRY_CONFIG_OVERRIDE = (Get-ChildItem -Path 'build\telemetry\packages' `
    -File 'MicrosoftTelemetry.h' -Recurse).FullName

# Build — traceloggingconfig.h will be overwritten
cargo build -p mxc_wil_telemetry
```

Without the env var (or without the package), `build.rs` uses the public WIL
headers as-is. No code changes needed.

### What's public vs. private

| Item | Public? | Why |
|------|---------|-----|
| Provider GUID `(0x4f50731a...)` | ✅ | Identifies the provider, harmless |
| Provider name `"Microsoft.MXC"` | ✅ | Standard ETW naming |
| `TraceLoggingOptionMicrosoftTelemetry()` call | ✅ | Compiles to no-op without private header |
| `build.rs` override logic | ✅ | Mechanism is public (same as WinAppSDK) |
| `packages.config` (package name/version) | ✅ | WinAppSDK publishes theirs too |
| Pipeline YAML (NuGet restore + env var) | ✅ | WinAppSDK publishes theirs too |
| Env var name `MXC_TELEMETRY_CONFIG_OVERRIDE` | ✅ | Key is public; value is machine-local |
| `MicrosoftTelemetry.h` (group GUID content) | ❌ | Private NuGet feed only |
| `build/telemetry/packages/` (restored files) | ❌ | `.gitignore`d |
| `TelemetryInternal` service connection creds | ❌ | ADO project settings |

## SDK License Override (EULA for npm Package)

The public GitHub repo ships `sdk/LICENSE.md` as a plain MIT license. For
internal npm publishes, a separate EULA containing a **Section 2 — DATA**
clause (covering telemetry disclosure, opt-out, and GDPR) is injected at
pack/publish time. This mirrors the WinAppSDK pattern where the NuGet
binary package carries a proprietary EULA while the source remains MIT.

### How it works

```
1. CI pipeline (or local script) sets MXC_LICENSE_OVERRIDE env var
   pointing to the private EULA markdown file.

2. scripts/apply-license-override.ps1 runs:
   ├── MXC_LICENSE_OVERRIDE is set:
   │   ├── Back up sdk/LICENSE.md → sdk/LICENSE.md.public
   │   └── Copy private EULA over sdk/LICENSE.md
   └── MXC_LICENSE_OVERRIDE is NOT set:
       └── Restore sdk/LICENSE.md from .public backup (if exists)

3. npm pack / npm publish picks up the private EULA as the LICENSE.md
   in the published package (sdk/package.json "files" includes LICENSE.md).

4. After publish, the revert path restores the MIT license.
```

### What the private EULA must contain

The private EULA should include a DATA section modeled after WinAppSDK's
NuGet license (Microsoft Software License Terms), covering:

- **Section 2a — Data Collection**: Disclosure that the software may collect
  usage data; "Your use of the software operates as your consent to these
  practices"; link to https://privacy.microsoft.com
- **Section 2b — Processing of Personal Data**: GDPR commitment referencing
  the Online Services Terms
- **Developer responsibility**: Note that developers using the SDK must
  comply with applicable law and provide appropriate notices to their users

### What's public vs. private

| Item | Public? | Why |
|------|---------|-----|
| `scripts/apply-license-override.ps1` | ✅ | Mechanism is public (same as WinAppSDK) |
| Env var name `MXC_LICENSE_OVERRIDE` | ✅ | Key is public; value is machine-local |
| `sdk/LICENSE.md` (MIT, in repo) | ✅ | Standard open-source license |
| Private EULA file (with DATA section) | ❌ | Internal artifact store only |
| `sdk/LICENSE.md.public` (backup) | ❌ | `.gitignore`d, transient build artifact |
| `build/eula/` (staging directory) | ❌ | `.gitignore`d |
