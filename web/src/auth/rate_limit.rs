//! In-memory exponential backoff for authentication attempts.
//!
//! Failures are tracked per client address and per username (and separately
//! for TOTP codes and API tokens). After `free_attempts` consecutive failures a
//! key is blocked for `base_delay * 2^(n - free_attempts)`, capped at
//! `max_delay`. A success clears the username key; an entry is forgotten after
//! `forget_after` without failures. State is lost on restart, which is fine.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub free_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub forget_after: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            free_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(15 * 60),
            forget_after: Duration::from_secs(60 * 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Client address. IPv6 addresses are truncated to their /64 so a single
    /// host cannot trivially rotate through its prefix.
    Client(IpAddr),
    User(String),
    Totp(String),
}

impl Key {
    pub fn client(ip: IpAddr) -> Self {
        Self::Client(match ip {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => {
                    let s = v6.segments();
                    IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
                }
            },
            v4 => v4,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    last_failure: Instant,
    blocked_until: Option<Instant>,
}

/// Upper bound on tracked keys; stale entries are pruned when it is reached.
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Default)]
pub struct RateLimiter {
    policy: Policy,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl RateLimiter {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            entries: Mutex::default(),
        }
    }

    /// Returns `Err(wait)` if any of `keys` is currently blocked.
    pub fn check(&self, keys: &[Key]) -> Result<(), Duration> {
        self.check_at(keys, Instant::now())
    }

    pub fn record_failure(&self, keys: &[Key]) {
        self.record_failure_at(keys, Instant::now());
    }

    /// Clears the given keys (typically only the user key; the client key is
    /// left alone so one valid account cannot launder a scanner's address).
    pub fn record_success(&self, keys: &[Key]) {
        let mut entries = self.lock();
        for key in keys {
            entries.remove(key);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
        // The map holds plain data, so a poisoned lock is still consistent.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check_at(&self, keys: &[Key], now: Instant) -> Result<(), Duration> {
        let entries = self.lock();
        let wait = keys
            .iter()
            .filter_map(|key| entries.get(key)?.blocked_until)
            .filter_map(|until| until.checked_duration_since(now))
            .filter(|wait| !wait.is_zero())
            .max();
        wait.map_or(Ok(()), Err)
    }

    fn record_failure_at(&self, keys: &[Key], now: Instant) {
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            let forget_after = self.policy.forget_after;
            entries.retain(|_, e| now.duration_since(e.last_failure) < forget_after);
        }
        for key in keys {
            let entry = entries.entry(key.clone()).or_insert(Entry {
                failures: 0,
                last_failure: now,
                blocked_until: None,
            });
            if now.duration_since(entry.last_failure) >= self.policy.forget_after {
                entry.failures = 0;
            }
            entry.failures = entry.failures.saturating_add(1);
            entry.last_failure = now;
            entry.blocked_until = self.delay_for(entry.failures).map(|d| now + d);
        }
    }

    fn delay_for(&self, failures: u32) -> Option<Duration> {
        let over = failures.checked_sub(self.policy.free_attempts)?;
        let factor = 1u32.checked_shl(over).unwrap_or(u32::MAX);
        Some(
            self.policy
                .base_delay
                .saturating_mul(factor)
                .min(self.policy.max_delay),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn limiter() -> RateLimiter {
        RateLimiter::new(Policy {
            free_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            forget_after: Duration::from_secs(600),
        })
    }

    fn keys() -> Vec<Key> {
        vec![
            Key::client(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            Key::User("admin".into()),
        ]
    }

    #[test]
    fn allows_free_attempts_then_backs_off_exponentially() {
        let rl = limiter();
        let t0 = Instant::now();
        let k = keys();
        for _ in 0..2 {
            rl.record_failure_at(&k, t0);
            assert!(rl.check_at(&k, t0).is_ok());
        }
        rl.record_failure_at(&k, t0); // third failure: 1s
        assert_eq!(rl.check_at(&k, t0), Err(Duration::from_secs(1)));
        assert!(rl.check_at(&k, t0 + Duration::from_secs(1)).is_ok());

        rl.record_failure_at(&k, t0); // fourth: 2s
        assert_eq!(rl.check_at(&k, t0), Err(Duration::from_secs(2)));
        for _ in 0..20 {
            rl.record_failure_at(&k, t0);
        }
        assert_eq!(rl.check_at(&k, t0), Err(Duration::from_secs(60)), "capped");
    }

    #[test]
    fn success_clears_only_given_keys() {
        let rl = limiter();
        let t0 = Instant::now();
        let k = keys();
        for _ in 0..3 {
            rl.record_failure_at(&k, t0);
        }
        rl.record_success(&k[1..]);
        assert!(rl.check_at(&k[1..], t0).is_ok());
        assert!(rl.check_at(&k[..1], t0).is_err(), "client stays blocked");
    }

    #[test]
    fn failures_are_forgotten_after_quiet_period() {
        let rl = limiter();
        let t0 = Instant::now();
        let k = keys();
        rl.record_failure_at(&k, t0);
        rl.record_failure_at(&k, t0);
        let later = t0 + Duration::from_secs(601);
        rl.record_failure_at(&k, later);
        assert!(rl.check_at(&k, later).is_ok(), "counter restarted at 1");
    }

    #[test]
    fn ipv6_clients_are_grouped_by_prefix() {
        let a = Key::client(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 3, 4, 5, 6)));
        let b = Key::client(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 9, 9, 9, 9)));
        let c = Key::client(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 3, 3, 4, 5, 6)));
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mapped = Key::client(IpAddr::V6(Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped()));
        assert_eq!(mapped, Key::client(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))));
    }
}
