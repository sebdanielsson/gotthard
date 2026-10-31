//! Running system tools that emit JSON (`ip -j`, `nft -j`).

use std::{io, process::Stdio, time::Duration};

use tokio::process::Command;

use crate::error::ApiError;

const TIMEOUT: Duration = Duration::from_secs(10);
/// Reject absurdly large outputs instead of parsing them (a big ruleset is a
/// few MB).
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
const SAFE_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// Runs `program args...` with a clean environment and parses its stdout as
/// JSON. Missing binaries, permission problems, timeouts and bad output are
/// mapped to distinct [`ApiError`]s.
pub async fn run_json(tool: &'static str, args: &[&str]) -> Result<serde_json::Value, ApiError> {
    let child = Command::new(tool)
        .args(args)
        .env_clear()
        .env("PATH", SAFE_PATH)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ApiError::ToolUnavailable { tool },
            io::ErrorKind::PermissionDenied => ApiError::ToolPermissionDenied { tool },
            _ => ApiError::internal(tool, e),
        })?;

    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ApiError::ToolFailed {
            tool,
            message: format!("timed out after {}s", TIMEOUT.as_secs()),
        })?
        .map_err(|e| ApiError::internal(tool, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first_line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        tracing::warn!(tool, status = %output.status, stderr = %first_line, "command failed");
        if looks_like_permission_error(&stderr) {
            return Err(ApiError::ToolPermissionDenied { tool });
        }
        return Err(ApiError::ToolFailed {
            tool,
            message: truncate(first_line, 200).unwrap_or_else(|| output.status.to_string()),
        });
    }
    if output.stdout.len() > MAX_OUTPUT {
        return Err(ApiError::ToolFailed {
            tool,
            message: "output too large".into(),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|e| ApiError::ToolFailed {
        tool,
        message: format!("unparseable JSON output: {e}"),
    })
}

fn looks_like_permission_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("operation not permitted") || lower.contains("permission denied")
}

fn truncate(s: &str, max_chars: usize) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.chars().take(max_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_tool_is_unavailable() {
        let err = run_json("gotthard-definitely-not-a-command", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::ToolUnavailable { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn parses_json_output() {
        let value = run_json("sh", &["-c", r#"echo '[{"ifname":"lo"}]'"#])
            .await
            .unwrap();
        assert_eq!(value[0]["ifname"], "lo");
    }

    #[tokio::test]
    async fn environment_is_scrubbed() {
        // HOME (like every other variable but PATH/LC_ALL) must not leak in.
        let value = run_json("sh", &["-c", r#"printf '"%s"' "${HOME:-unset}""#])
            .await
            .unwrap();
        assert_eq!(value, "unset");
    }

    #[tokio::test]
    async fn maps_failures() {
        let err = run_json(
            "sh",
            &["-c", "echo 'Error: Operation not permitted' >&2; exit 1"],
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ApiError::ToolPermissionDenied { .. }),
            "{err:?}"
        );

        let err = run_json("sh", &["-c", "echo 'boom' >&2; exit 2"])
            .await
            .unwrap_err();
        match err {
            ApiError::ToolFailed { message, .. } => assert_eq!(message, "boom"),
            other => panic!("unexpected {other:?}"),
        }

        let err = run_json("sh", &["-c", "echo not-json"]).await.unwrap_err();
        assert!(matches!(err, ApiError::ToolFailed { .. }), "{err:?}");
    }
}
