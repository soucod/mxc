// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Build script for `mxc_wil_telemetry`.
//!
//! On Windows targets, this script:
//! 1. Downloads the WIL NuGet package (header-only, MIT licensed)
//! 2. Extracts the `include/wil/` headers
//! 3. Applies the telemetry config override (if `MXC_TELEMETRY_CONFIG_OVERRIDE`
//!    is set) — this mirrors WinAppSDK's `UpdateTraceloggingConfig` pipeline step
//! 4. Compiles the C++ telemetry shim against those headers
//!
//! On non-Windows targets, nothing is compiled — the Rust library
//! exposes no-op stub functions instead.

fn main() {
    #[cfg(target_os = "windows")]
    windows_build::build();
}

#[cfg(target_os = "windows")]
mod windows_build {
    use std::fs;
    use std::io::{self, Read};
    use std::path::{Path, PathBuf};

    /// WIL NuGet package version — matches WinAppSDK's `Directory.Packages.props`.
    const WIL_VERSION: &str = "1.0.260126.7";

    /// NuGet package download URL.
    const WIL_NUGET_URL: &str = concat!(
        "https://www.nuget.org/api/v2/package/",
        "Microsoft.Windows.ImplementationLibrary/1.0.260126.7"
    );

    /// Cache directory name under `OUT_DIR` so Cargo automatically invalidates
    /// the cache when the target or profile changes.
    const CACHE_DIR_NAME: &str = "wil-cache";

    /// Sentinel file that indicates headers have already been extracted.
    const SENTINEL_NAME: &str = ".wil-extracted";

    pub fn build() {
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
        let cache_dir = out_dir.join(CACHE_DIR_NAME);
        let include_dir = cache_dir.join("include");
        let sentinel = cache_dir.join(SENTINEL_NAME);

        // Only download + extract if we haven't already, or if the cached version is stale.
        let cached_version = fs::read_to_string(&sentinel).unwrap_or_default();
        if cached_version.trim() != WIL_VERSION {
            if cache_dir.exists() {
                fs::remove_dir_all(&cache_dir)
                    .expect("failed to clear stale WIL cache directory");
            }
            fs::create_dir_all(&cache_dir).expect("failed to create WIL cache directory");
            download_and_extract_wil(&cache_dir, &include_dir);
            fs::write(&sentinel, WIL_VERSION).expect("failed to write WIL sentinel");
        }

        // Apply telemetry configuration override for internal builds.
        // This mirrors WinAppSDK's UpdateTraceloggingConfig pipeline step:
        // the private MicrosoftTelemetry.h overwrites the public no-op
        // traceloggingconfig.h so TraceLoggingOptionMicrosoftTelemetry()
        // expands to the real Microsoft telemetry group GUID.
        apply_telemetry_config_override(&include_dir);

        // Compile the C++ shim.
        let shim_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("shim");

        cc::Build::new()
            .cpp(true)
            .file(shim_dir.join("mxc_telemetry_shim.cpp"))
            .include(&include_dir)
            // WIL needs the Windows SDK headers — MSVC toolchain provides them.
            .std("c++17")
            // Suppress warnings from WIL headers that we don't control.
            .warnings(false)
            .compile("mxc_telemetry_shim");

        // Tell Cargo to re-run if the shim source changes.
        println!("cargo::rerun-if-changed=shim/mxc_telemetry_shim.cpp");
        println!("cargo::rerun-if-changed=shim/mxc_telemetry_shim.h");

        // Link against advapi32 for ETW functions (EventRegister, etc.).
        println!("cargo::rustc-link-lib=advapi32");
    }

    /// Download the WIL NuGet package and extract `include/wil/` headers.
    fn download_and_extract_wil(cache_dir: &Path, include_dir: &Path) {
        eprintln!("Downloading WIL NuGet package v{WIL_VERSION}...");

        let resp = ureq::get(WIL_NUGET_URL)
            .call()
            .expect("failed to download WIL NuGet package");

        let mut body = Vec::new();
        resp.into_reader()
            .read_to_end(&mut body)
            .expect("failed to read WIL NuGet response body");

        let cursor = io::Cursor::new(body);
        let mut archive =
            zip::ZipArchive::new(cursor).expect("WIL NuGet package is not a valid zip");

        // Extract only files under `include/` — WIL is header-only so this
        // is all we need.
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            let rel_path = match file.enclosed_name() {
                Some(p) => p.to_owned(),
                None => continue,
            };

            // NuGet packages use forward-slash paths internally.
            if !rel_path.starts_with("include") || file.is_dir() {
                continue;
            }

            let dest = cache_dir.join(&rel_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).unwrap();
            }

            let mut out = fs::File::create(&dest).unwrap();
            io::copy(&mut file, &mut out).unwrap();
        }

        // Verify that we actually got the headers.
        assert!(
            include_dir.join("wil").join("tracelogging.h").exists(),
            "WIL headers not found after extraction — download may have failed"
        );

        eprintln!("WIL headers extracted to {}", include_dir.display());
    }

    /// Apply — or revert — the telemetry configuration override based on
    /// `MXC_TELEMETRY_CONFIG_OVERRIDE`.
    ///
    /// This is the Cargo equivalent of WinAppSDK's `UpdateTraceloggingConfig`
    /// PowerShell step. The env var should point to `MicrosoftTelemetry.h`
    /// from the `Microsoft.Telemetry.Inbox.Native` NuGet package. When set,
    /// the file is copied over `wil/traceloggingconfig.h` so
    /// `TraceLoggingOptionMicrosoftTelemetry()` expands to the real Microsoft
    /// telemetry group GUID instead of the public no-op stub.
    ///
    /// When the env var is **unset**, the function restores the original
    /// public header from a `.public` backup. This prevents a stale private
    /// header from persisting in `OUT_DIR` across incremental builds.
    ///
    /// Public/community builds leave the env var unset — the macro stays empty
    /// and events fire as plain ETW with no Microsoft pipeline routing.
    fn apply_telemetry_config_override(include_dir: &Path) {
        // Re-run build.rs whenever this env var changes so toggling between
        // public/private builds picks up the new value without a clean build.
        println!("cargo::rerun-if-env-changed=MXC_TELEMETRY_CONFIG_OVERRIDE");

        let dest = include_dir.join("wil").join("traceloggingconfig.h");
        let backup = include_dir.join("wil").join("traceloggingconfig.h.public");

        // On first run, save the pristine public header so we can restore it
        // later when toggling back from a private build.
        if !backup.exists() && dest.exists() {
            fs::copy(&dest, &backup).unwrap_or_else(|e| {
                panic!("failed to back up public traceloggingconfig.h: {e}");
            });
        }

        let override_path = match std::env::var("MXC_TELEMETRY_CONFIG_OVERRIDE") {
            Ok(p) if !p.is_empty() => PathBuf::from(p),
            _ => {
                // Not set or empty — restore the public stub if a private
                // override was applied in a previous build.
                if backup.exists() {
                    fs::copy(&backup, &dest).unwrap_or_else(|e| {
                        panic!("failed to restore public traceloggingconfig.h: {e}");
                    });
                    eprintln!("Restored public (no-op) traceloggingconfig.h");
                }
                return;
            }
        };

        if !override_path.exists() {
            panic!(
                "MXC_TELEMETRY_CONFIG_OVERRIDE points to non-existent file: {}\n\
                 Verify that the Microsoft.Telemetry.Inbox.Native NuGet package \
                 was restored correctly.",
                override_path.display()
            );
        }

        fs::copy(&override_path, &dest).unwrap_or_else(|e| {
            panic!(
                "failed to apply telemetry config override {} -> {}: {e}",
                override_path.display(),
                dest.display()
            );
        });

        // Also tell Cargo to re-run if the override file itself changes.
        println!("cargo::rerun-if-changed={}", override_path.display());

        eprintln!(
            "Applied telemetry config override from {}",
            override_path.display()
        );
    }
}
