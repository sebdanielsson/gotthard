//! System information from `/proc` and `/etc/os-release`.
//!
//! Every field is optional: on a non-Linux dev machine (or with a restricted
//! `/proc`) the endpoint degrades to whatever can be read instead of failing.

use std::{collections::BTreeMap, path::Path};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub hostname: Option<String>,
    pub kernel_release: Option<String>,
    pub uptime_seconds: Option<f64>,
    pub load_average: Option<LoadAverage>,
    pub memory: Option<Memory>,
    /// All key/value pairs from os-release (`ID`, `PRETTY_NAME`, ...).
    pub os_release: Option<BTreeMap<String, String>>,
    pub web_version: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Memory {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub free_bytes: u64,
    pub buffers_bytes: u64,
    pub cached_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
}

pub async fn collect() -> SystemInfo {
    let (hostname, kernel, uptime, loadavg, meminfo, os_release) = tokio::join!(
        read("/proc/sys/kernel/hostname"),
        read("/proc/sys/kernel/osrelease"),
        read("/proc/uptime"),
        read("/proc/loadavg"),
        read("/proc/meminfo"),
        read_os_release(),
    );
    SystemInfo {
        hostname: hostname.map(|s| s.trim().to_owned()),
        kernel_release: kernel.map(|s| s.trim().to_owned()),
        uptime_seconds: uptime.as_deref().and_then(parse_uptime),
        load_average: loadavg.as_deref().and_then(parse_loadavg),
        memory: meminfo.as_deref().and_then(parse_meminfo),
        os_release: os_release.as_deref().map(parse_os_release),
        web_version: env!("CARGO_PKG_VERSION"),
    }
}

/// Best-effort hostname for labels, without going async.
pub fn hostname() -> String {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "gotthard".to_owned())
}

async fn read(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref();
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Some(s),
        Err(err) => {
            tracing::debug!(path = %path.display(), %err, "cannot read system file");
            None
        }
    }
}

async fn read_os_release() -> Option<String> {
    // os-release(5): /etc/os-release takes precedence over /usr/lib/os-release.
    match read("/etc/os-release").await {
        Some(s) => Some(s),
        None => read("/usr/lib/os-release").await,
    }
}

fn parse_uptime(s: &str) -> Option<f64> {
    s.split_whitespace().next()?.parse().ok()
}

fn parse_loadavg(s: &str) -> Option<LoadAverage> {
    let mut it = s.split_whitespace().map(str::parse::<f64>);
    Some(LoadAverage {
        one: it.next()?.ok()?,
        five: it.next()?.ok()?,
        fifteen: it.next()?.ok()?,
    })
}

fn parse_meminfo(s: &str) -> Option<Memory> {
    let fields: BTreeMap<&str, u64> = s
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let mut parts = rest.split_whitespace();
            let value: u64 = parts.next()?.parse().ok()?;
            let multiplier = match parts.next() {
                Some("kB") => 1024,
                None => 1,
                Some(_) => return None,
            };
            Some((key.trim(), value.saturating_mul(multiplier)))
        })
        .collect();
    let get = |k: &str| fields.get(k).copied();
    let total = get("MemTotal")?;
    let free = get("MemFree")?;
    Some(Memory {
        total_bytes: total,
        // MemAvailable exists since Linux 3.14; fall back to MemFree.
        available_bytes: get("MemAvailable").unwrap_or(free),
        free_bytes: free,
        buffers_bytes: get("Buffers").unwrap_or(0),
        cached_bytes: get("Cached").unwrap_or(0),
        swap_total_bytes: get("SwapTotal").unwrap_or(0),
        swap_free_bytes: get("SwapFree").unwrap_or(0),
    })
}

/// Parses os-release(5): `KEY=value` lines with shell-style quoting.
fn parse_os_release(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let valid_key = !key.is_empty()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            valid_key.then(|| (key.to_owned(), unquote(value)))
        })
        .collect()
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.to_owned();
    }
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_and_loadavg() {
        assert_eq!(parse_uptime("12345.67 54321.00\n"), Some(12345.67));
        assert_eq!(parse_uptime(""), None);
        assert_eq!(
            parse_loadavg("0.15 0.10 0.05 1/123 4567\n"),
            Some(LoadAverage {
                one: 0.15,
                five: 0.10,
                fifteen: 0.05
            })
        );
        assert_eq!(parse_loadavg("0.15 x"), None);
    }

    #[test]
    fn meminfo() {
        let input = "\
MemTotal:        4030464 kB
MemFree:          123456 kB
MemAvailable:    3000000 kB
Buffers:            1000 kB
Cached:           200000 kB
SwapTotal:             0 kB
SwapFree:              0 kB
HugePages_Total:       0
";
        let mem = parse_meminfo(input).unwrap();
        assert_eq!(mem.total_bytes, 4_030_464 * 1024);
        assert_eq!(mem.available_bytes, 3_000_000 * 1024);
        assert_eq!(mem.cached_bytes, 200_000 * 1024);
        assert!(parse_meminfo("Garbage: 1 kB").is_none());
    }

    #[test]
    fn os_release() {
        let input = r#"
# comment
NAME="Alpine Linux"
ID=alpine
VERSION_ID=3.24.0
PRETTY_NAME="Gotthard \"edge\" (Alpine Linux v3.24)"
HOME_URL='https://alpinelinux.org/'
lowercase=ignored
"#;
        let map = parse_os_release(input);
        assert_eq!(map["NAME"], "Alpine Linux");
        assert_eq!(map["ID"], "alpine");
        assert_eq!(
            map["PRETTY_NAME"],
            r#"Gotthard "edge" (Alpine Linux v3.24)"#
        );
        assert_eq!(map["HOME_URL"], "https://alpinelinux.org/");
        assert!(!map.contains_key("lowercase"));
    }
}
