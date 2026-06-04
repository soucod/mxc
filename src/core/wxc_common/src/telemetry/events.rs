// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TraceLogging ETW event emission for MXC telemetry.
//!
//! Event-specific data types and emission functions. The actual ETW
//! write is delegated to the `mxc_wil_telemetry` C++ shim, which
//! adds Part B common fields automatically.

/// Bounded set of failure categories for error classification.
/// Prevents free-form strings that could contain PII.
#[derive(Debug, Clone, Copy)]
pub enum FailureReason {
    ConfigError,
    PolicyError,
    ProcessError,
    Timeout,
    InitError,
    Unknown,
}

impl FailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigError => "config_error",
            Self::PolicyError => "policy_error",
            Self::ProcessError => "process_error",
            Self::Timeout => "timeout",
            Self::InitError => "init_error",
            Self::Unknown => "unknown",
        }
    }
}

/// Sanitize an error message by stripping potential PII (file paths, usernames).
pub fn sanitize_error_message(msg: &str) -> String {
    let mut sanitized = String::with_capacity(msg.len());
    let mut chars = msg.chars().peekable();

    while let Some(c) = chars.next() {
        // Detect Windows paths: letter followed by :\
        if c.is_ascii_alphabetic() {
            if let Some(&next) = chars.peek() {
                if next == ':' {
                    // Peek further for backslash
                    let rest: String = chars.clone().take(2).collect();
                    if rest.starts_with(":\\") {
                        sanitized.push_str("<path>");
                        // Skip until whitespace or quote
                        for ch in chars.by_ref() {
                            if ch.is_whitespace() || ch == '\'' || ch == '"' {
                                sanitized.push(ch);
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
            sanitized.push(c);
            continue;
        }

        // Detect Unix paths: /home/, /tmp/, /var/, /usr/, /etc/, /root/, /mnt/, /opt/
        if c == '/' {
            let prefixes = [
                "home/", "tmp/", "var/", "usr/", "etc/", "root/", "mnt/", "opt/",
            ];
            let upcoming: String = chars.clone().take(5).collect();
            if prefixes.iter().any(|p| upcoming.starts_with(p)) {
                sanitized.push_str("<path>");
                // Skip until whitespace or quote
                for ch in chars.by_ref() {
                    if ch.is_whitespace() || ch == '\'' || ch == '"' {
                        sanitized.push(ch);
                        break;
                    }
                }
                continue;
            }
        }

        sanitized.push(c);
    }

    // Truncate to a reasonable length, respecting UTF-8 char boundaries
    // to avoid panics from `String::truncate` on multi-byte characters.
    const MAX_LEN: usize = 256;
    const ELLIPSIS: &str = "...";
    if sanitized.len() > MAX_LEN {
        // Reserve space for the ellipsis so the final output is <= MAX_LEN.
        let mut truncate_at = MAX_LEN.saturating_sub(ELLIPSIS.len());
        while !sanitized.is_char_boundary(truncate_at) {
            truncate_at -= 1;
        }
        sanitized.truncate(truncate_at);
        sanitized.push_str(ELLIPSIS);
    }

    sanitized
}

/// Data for an MXC.Execution ETW event.
pub struct ExecutionEvent<'a> {
    pub backend: &'a str,
    pub exit_code: i32,
    pub outcome: &'a str,
    pub duration_ms: u64,
    pub version: &'a str,
    pub failure_reason: Option<FailureReason>,
}

/// Log an MXC.Execution ETW event.
///
/// Delegates to the WIL C++ shim which adds Part B common fields
/// (Version, Channel, IsDebugging, UTCReplace_AppSessionGuid).
pub fn log_execution(event: &ExecutionEvent<'_>) {
    let failure_str = event.failure_reason.map(|r| r.as_str()).unwrap_or("");

    mxc_wil_telemetry::log_execution(
        event.backend,
        event.exit_code,
        event.outcome,
        event.duration_ms,
        failure_str,
    );
}

/// Log an MXC.Error ETW event.
///
/// Delegates to the WIL C++ shim which adds Part B common fields.
pub fn log_error(backend: &str, error_type: FailureReason, error_message: &str, version: &str) {
    let _ = version; // Version is now provided by the C++ shim via Part B fields
    let sanitized = sanitize_error_message(error_message);

    mxc_wil_telemetry::log_error(backend, error_type.as_str(), &sanitized);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_reason_as_str() {
        assert_eq!(FailureReason::ConfigError.as_str(), "config_error");
        assert_eq!(FailureReason::PolicyError.as_str(), "policy_error");
        assert_eq!(FailureReason::ProcessError.as_str(), "process_error");
        assert_eq!(FailureReason::Timeout.as_str(), "timeout");
        assert_eq!(FailureReason::InitError.as_str(), "init_error");
        assert_eq!(FailureReason::Unknown.as_str(), "unknown");
    }

    #[test]
    fn sanitize_strips_windows_paths() {
        let msg = "Failed to read C:\\Users\\alice\\secret\\config.json";
        let result = sanitize_error_message(msg);
        assert!(!result.contains("alice"));
        assert!(!result.contains("secret"));
        assert!(result.contains("<path>"));
    }

    #[test]
    fn sanitize_strips_unix_paths() {
        let msg = "Cannot open /home/bob/project/data.txt";
        let result = sanitize_error_message(msg);
        assert!(!result.contains("bob"));
        assert!(result.contains("<path>"));
    }

    #[test]
    fn sanitize_truncates_long_messages() {
        let long_msg = "x".repeat(500);
        let result = sanitize_error_message(&long_msg);
        assert!(result.len() < 300);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn sanitize_preserves_safe_messages() {
        let msg = "Firewall rule creation failed";
        assert_eq!(sanitize_error_message(msg), msg);
    }
}
