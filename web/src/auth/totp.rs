//! TOTP (RFC 6238) second factor.
//!
//! Secrets live in `<state_dir>/totp.json` (0600). Enrolment is two-step: a
//! secret is generated and stored as *pending*, and only becomes active once
//! the user proves possession by submitting a valid code. Each accepted time
//! step is remembered so a code can never be used twice.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use base64::Engine as _;
use qrcode::{EcLevel, QrCode, render::svg};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use totp_rs::{Algorithm, Builder, Secret, Totp};

use crate::secure_fs;

const FILE_NAME: &str = "totp.json";
const SECRET_BYTES: usize = 20;
const DIGITS: u8 = 6;
const STEP_SECONDS: u64 = 30;
/// Accept the previous and next step as well, for clock drift.
const SKEW_STEPS: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum TotpError {
    #[error("two-factor authentication is not enrolled")]
    NotEnrolled,
    #[error("two-factor authentication is already enabled")]
    AlreadyEnabled,
    #[error("invalid verification code")]
    InvalidCode,
    #[error("generating secret: {0}")]
    Random(getrandom::Error),
    #[error("building TOTP: {0}")]
    Totp(#[from] totp_rs::TotpError),
    #[error("stored TOTP secret is corrupt")]
    CorruptSecret,
    #[error("rendering QR code: {0}")]
    Qr(#[from] qrcode::types::QrError),
    #[error("persisting TOTP state: {0}")]
    Storage(#[from] io::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TotpFile {
    version: u32,
    users: BTreeMap<String, TotpRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
struct TotpRecord {
    /// Base32 (RFC 4648, no padding), as shown to the user.
    secret: String,
    enabled: bool,
    created_at: u64,
    /// Last accepted time step, for replay protection.
    #[serde(default)]
    last_step: Option<u64>,
}

impl std::fmt::Debug for TotpRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpRecord")
            .field("enabled", &self.enabled)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TotpStatus {
    pub enabled: bool,
    pub pending: bool,
}

/// Everything the client needs to add the account to an authenticator app.
#[derive(Serialize)]
pub struct Enrollment {
    pub secret: String,
    pub otpauth_url: String,
    /// `data:image/svg+xml;base64,...` QR code of `otpauth_url`.
    pub qr_code: String,
}

#[derive(Debug)]
pub struct TotpStore {
    path: PathBuf,
    issuer: String,
    state: Mutex<TotpFile>,
}

impl TotpStore {
    pub fn load(state_dir: &Path, issuer: impl Into<String>) -> io::Result<Self> {
        let path = state_dir.join(FILE_NAME);
        let state = secure_fs::read_json(&path)?.unwrap_or(TotpFile {
            version: 1,
            ..TotpFile::default()
        });
        Ok(Self {
            path,
            issuer: issuer.into(),
            state: Mutex::new(state),
        })
    }

    pub async fn status(&self, user: &str) -> TotpStatus {
        let state = self.state.lock().await;
        match state.users.get(user) {
            Some(r) => TotpStatus {
                enabled: r.enabled,
                pending: !r.enabled,
            },
            None => TotpStatus {
                enabled: false,
                pending: false,
            },
        }
    }

    pub async fn is_enabled(&self, user: &str) -> bool {
        self.status(user).await.enabled
    }

    /// Generates a new pending secret for `user`, replacing any earlier
    /// pending one. Fails if TOTP is already enabled.
    pub async fn begin_enrollment(
        &self,
        user: &str,
        account_label: &str,
    ) -> Result<Enrollment, TotpError> {
        let mut state = self.state.lock().await;
        if state.users.get(user).is_some_and(|r| r.enabled) {
            return Err(TotpError::AlreadyEnabled);
        }
        let mut secret = [0u8; SECRET_BYTES];
        getrandom::fill(&mut secret).map_err(TotpError::Random)?;
        let totp = build_totp(&secret, account_label, &self.issuer)?;
        let enrollment = Enrollment {
            secret: totp.secret().to_base32(),
            otpauth_url: totp.to_url()?,
            qr_code: qr_data_uri(&totp.to_url()?)?,
        };

        let mut next = state.clone();
        next.users.insert(
            user.to_owned(),
            TotpRecord {
                secret: enrollment.secret.clone(),
                enabled: false,
                created_at: unix_now(),
                last_step: None,
            },
        );
        self.persist(&next).await?;
        *state = next;
        Ok(enrollment)
    }

    /// Activates a pending enrolment if `code` is valid.
    pub async fn confirm_enrollment(&self, user: &str, code: &str) -> Result<(), TotpError> {
        self.confirm_enrollment_at(user, code, unix_now()).await
    }

    async fn confirm_enrollment_at(
        &self,
        user: &str,
        code: &str,
        now: u64,
    ) -> Result<(), TotpError> {
        let mut state = self.state.lock().await;
        let record = match state.users.get(user) {
            Some(r) if r.enabled => return Err(TotpError::AlreadyEnabled),
            Some(r) => r,
            None => return Err(TotpError::NotEnrolled),
        };
        let step = check_record(record, user, &self.issuer, code, now)?;

        let mut next = state.clone();
        if let Some(r) = next.users.get_mut(user) {
            r.enabled = true;
            r.last_step = Some(step);
        }
        self.persist(&next).await?;
        *state = next;
        Ok(())
    }

    /// Checks a login code. On success the step is burnt so the same code
    /// cannot be replayed.
    pub async fn verify(&self, user: &str, code: &str) -> Result<(), TotpError> {
        self.verify_at(user, code, unix_now()).await
    }

    async fn verify_at(&self, user: &str, code: &str, now: u64) -> Result<(), TotpError> {
        let mut state = self.state.lock().await;
        let record = state
            .users
            .get(user)
            .filter(|r| r.enabled)
            .ok_or(TotpError::NotEnrolled)?;
        let step = check_record(record, user, &self.issuer, code, now)?;

        if let Some(r) = state.users.get_mut(user) {
            r.last_step = Some(step);
        }
        // Replay protection is enforced from memory; a failed write only
        // weakens it across restarts, so don't lock the user out over it.
        if let Err(err) = self.persist(&state).await {
            tracing::error!(%err, "failed to persist TOTP step");
        }
        Ok(())
    }

    /// Removes TOTP for `user` (enabled or pending). An enabled factor can
    /// only be removed with a valid code.
    pub async fn disable(&self, user: &str, code: &str) -> Result<(), TotpError> {
        self.disable_at(user, code, unix_now()).await
    }

    async fn disable_at(&self, user: &str, code: &str, now: u64) -> Result<(), TotpError> {
        let mut state = self.state.lock().await;
        let record = state.users.get(user).ok_or(TotpError::NotEnrolled)?;
        if record.enabled {
            check_record(record, user, &self.issuer, code, now)?;
        }
        let mut next = state.clone();
        next.users.remove(user);
        self.persist(&next).await?;
        *state = next;
        Ok(())
    }

    async fn persist(&self, state: &TotpFile) -> io::Result<()> {
        let path = self.path.clone();
        let snapshot = state.clone();
        tokio::task::spawn_blocking(move || secure_fs::write_json_private(&path, &snapshot))
            .await
            .map_err(io::Error::other)?
    }
}

fn check_record(
    record: &TotpRecord,
    user: &str,
    issuer: &str,
    code: &str,
    now: u64,
) -> Result<u64, TotpError> {
    let secret = Secret::try_from_base32(&record.secret).map_err(|_| TotpError::CorruptSecret)?;
    let totp = build_totp(secret.as_ref(), user, issuer)?;
    check_code(&totp, code, now, record.last_step).ok_or(TotpError::InvalidCode)
}

/// Standard authenticator-app compatible parameters: SHA-1, 6 digits, 30 s.
fn build_totp(secret: &[u8], account: &str, issuer: &str) -> Result<Totp, TotpError> {
    Ok(Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(DIGITS)
        .with_skew(SKEW_STEPS)
        .with_step_duration(STEP_SECONDS)
        .with_secret(secret.to_vec())
        .with_account_name(account)
        .with_issuer(Some(issuer))
        .build()?)
}

/// Validates `code` at time `now` and returns the matched step, rejecting any
/// step at or before `last_step` (replays).
fn check_code(totp: &Totp, code: &str, now: u64, last_step: Option<u64>) -> Option<u64> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != usize::from(DIGITS) || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let step = totp.check(&code, now)?;
    match last_step {
        Some(last) if step <= last => None,
        _ => Some(step),
    }
}

fn qr_data_uri(text: &str) -> Result<String, TotpError> {
    let svg = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)?
        .render::<svg::Color<'_>>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    ))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const NOW: u64 = 1_790_000_000;

    fn totp() -> Totp {
        build_totp(b"12345678901234567890", "admin@router", "Gotthard").unwrap()
    }

    fn code_at(totp: &Totp, time: u64) -> String {
        totp.generate(time).to_string()
    }

    #[test]
    fn rfc6238_reference_vector() {
        // RFC 6238 appendix B, SHA-1, T = 59 -> 94287082 (8 digits); the
        // 6-digit truncation is the last six digits.
        assert_eq!(code_at(&totp(), 59), "287082");
    }

    #[test]
    fn accepts_current_and_adjacent_steps_only() {
        let t = totp();
        let step = NOW / STEP_SECONDS;
        assert_eq!(check_code(&t, &code_at(&t, NOW), NOW, None), Some(step));
        assert!(check_code(&t, &code_at(&t, NOW - 30), NOW, None).is_some());
        assert!(check_code(&t, &code_at(&t, NOW + 30), NOW, None).is_some());
        assert!(check_code(&t, &code_at(&t, NOW - 90), NOW, None).is_none());
        assert!(check_code(&t, &code_at(&t, NOW + 90), NOW, None).is_none());
    }

    #[test]
    fn rejects_replayed_steps() {
        let t = totp();
        let code = code_at(&t, NOW);
        let step = check_code(&t, &code, NOW, None).unwrap();
        assert!(check_code(&t, &code, NOW, Some(step)).is_none());
        // An older (but still in-window) code is also rejected once a newer
        // step has been used.
        let older = code_at(&t, NOW - 30);
        assert!(check_code(&t, &older, NOW, Some(step)).is_none());
    }

    #[test]
    fn normalises_and_validates_input() {
        let t = totp();
        let code = code_at(&t, NOW);
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert!(check_code(&t, &spaced, NOW, None).is_some());
        for bad in ["", "12345", "1234567", "abcdef", "12345a", "١٢٣٤٥٦"] {
            assert!(check_code(&t, bad, NOW, None).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn short_secrets_are_rejected() {
        assert!(build_totp(b"too short", "admin", "Gotthard").is_err());
    }

    #[tokio::test]
    async fn enrollment_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = TotpStore::load(dir.path(), "Gotthard").unwrap();
        let none = TotpStatus {
            enabled: false,
            pending: false,
        };
        assert_eq!(store.status("admin").await, none);
        assert!(matches!(
            store.verify_at("admin", "000000", NOW).await,
            Err(TotpError::NotEnrolled)
        ));

        let enrollment = store
            .begin_enrollment("admin", "admin@router")
            .await
            .unwrap();
        assert!(enrollment.otpauth_url.starts_with("otpauth://totp/"));
        assert!(enrollment.otpauth_url.contains("issuer=Gotthard"));
        assert!(enrollment.qr_code.starts_with("data:image/svg+xml;base64,"));
        assert!(store.status("admin").await.pending);
        assert!(!store.is_enabled("admin").await, "pending is not enabled");

        let secret = Secret::try_from_base32(&enrollment.secret).unwrap();
        let t = build_totp(secret.as_ref(), "admin", "Gotthard").unwrap();

        assert!(matches!(
            store.confirm_enrollment_at("admin", "000000", NOW).await,
            Err(TotpError::InvalidCode)
        ));
        store
            .confirm_enrollment_at("admin", &code_at(&t, NOW), NOW)
            .await
            .unwrap();
        assert!(store.is_enabled("admin").await);
        assert!(matches!(
            store.begin_enrollment("admin", "admin@router").await,
            Err(TotpError::AlreadyEnabled)
        ));

        // The confirmation code is burnt; the next step works once.
        assert!(
            store
                .verify_at("admin", &code_at(&t, NOW), NOW)
                .await
                .is_err()
        );
        let next = code_at(&t, NOW + 30);
        store.verify_at("admin", &next, NOW + 30).await.unwrap();
        assert!(store.verify_at("admin", &next, NOW + 30).await.is_err());

        // State survives a reload and the file is private.
        let path = dir.path().join(FILE_NAME);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let reloaded = TotpStore::load(dir.path(), "Gotthard").unwrap();
        assert!(reloaded.is_enabled("admin").await);
        assert!(
            reloaded.verify_at("admin", &next, NOW + 30).await.is_err(),
            "replay protection persisted"
        );

        // Disabling needs a valid code.
        assert!(matches!(
            reloaded.disable_at("admin", "000000", NOW + 60).await,
            Err(TotpError::InvalidCode)
        ));
        reloaded
            .disable_at("admin", &code_at(&t, NOW + 60), NOW + 60)
            .await
            .unwrap();
        assert_eq!(reloaded.status("admin").await, none);
    }

    #[tokio::test]
    async fn pending_enrollment_can_be_cancelled_without_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = TotpStore::load(dir.path(), "Gotthard").unwrap();
        store.begin_enrollment("admin", "admin").await.unwrap();
        store.disable_at("admin", "", NOW).await.unwrap();
        assert!(!store.status("admin").await.pending);
    }
}
