//! API keys for automation (scripts, MCP servers).
//!
//! A token looks like `gth_<id>_<secret>`: `id` is 16 hex characters used to
//! find the record, `secret` is 32 random bytes in unpadded base64url. Only an
//! Argon2id hash of the whole token is stored, in `<state_dir>/api_keys.json`
//! (0600). The plaintext token is returned exactly once, at creation.
//!
//! Argon2 is deliberately expensive, so verification runs on the blocking pool
//! behind a small semaphore, and successful verifications are cached briefly
//! (keyed by a SHA-256 of the token) so automation polling the API does not
//! pay the full cost on every request.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher as _, PasswordVerifier as _, phc::PasswordHash},
};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore};

use crate::secure_fs;

const FILE_NAME: &str = "api_keys.json";
const TOKEN_PREFIX: &str = "gth_";
const ID_BYTES: usize = 8;
const SECRET_BYTES: usize = 32;
const MAX_NAME_LEN: usize = 64;
const MAX_KEYS: usize = 100;
const CACHE_TTL: Duration = Duration::from_secs(300);
/// Concurrent Argon2 verifications (each needs ~19 MiB).
const MAX_CONCURRENT_VERIFICATIONS: usize = 2;

/// What an API key is allowed to do. Sessions implicitly have every scope.
#[expect(
    clippy::enum_variant_names,
    reason = "write scopes will be added alongside the read ones"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "system:read")]
    SystemRead,
    #[serde(rename = "network:read")]
    NetworkRead,
    #[serde(rename = "firewall:read")]
    FirewallRead,
}

impl Scope {
    pub const ALL: [Self; 3] = [Self::SystemRead, Self::NetworkRead, Self::FirewallRead];
}

#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    #[error("name must be 1-{MAX_NAME_LEN} printable characters")]
    InvalidName,
    #[error("at least one scope is required")]
    NoScopes,
    #[error("an API key with this name already exists")]
    DuplicateName,
    #[error("too many API keys (maximum {MAX_KEYS})")]
    TooMany,
    #[error("generating token: {0}")]
    Random(getrandom::Error),
    #[error("hashing token: {0}")]
    Hash(String),
    #[error("persisting API keys: {0}")]
    Storage(#[from] io::Error),
}

#[derive(Clone, Serialize, Deserialize)]
struct ApiKeyRecord {
    id: String,
    name: String,
    scopes: Vec<Scope>,
    /// Argon2id PHC string of the full token.
    hash: String,
    created_at: u64,
    created_by: String,
}

impl ApiKeyRecord {
    fn info(&self) -> ApiKeyInfo {
        ApiKeyInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            scopes: self.scopes.clone(),
            created_at: self.created_at,
            created_by: self.created_by.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct KeysFile {
    version: u32,
    keys: Vec<ApiKeyRecord>,
}

impl std::fmt::Debug for ApiKeyRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyRecord")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Public view of a key (never includes the hash).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
    pub created_at: u64,
    pub created_by: String,
}

/// Returned once from [`ApiKeyStore::create`].
#[derive(Serialize)]
pub struct NewApiKey {
    #[serde(flatten)]
    pub info: ApiKeyInfo,
    pub token: String,
}

#[derive(Debug)]
pub struct ApiKeyStore {
    path: PathBuf,
    state: Mutex<KeysFile>,
    cache: std::sync::Mutex<HashMap<[u8; 32], (String, Instant)>>,
    verify_permits: Arc<Semaphore>,
}

impl ApiKeyStore {
    pub fn load(state_dir: &Path) -> io::Result<Self> {
        let path = state_dir.join(FILE_NAME);
        let state = secure_fs::read_json(&path)?.unwrap_or(KeysFile {
            version: 1,
            ..KeysFile::default()
        });
        Ok(Self {
            path,
            state: Mutex::new(state),
            cache: std::sync::Mutex::default(),
            verify_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_VERIFICATIONS)),
        })
    }

    pub async fn list(&self) -> Vec<ApiKeyInfo> {
        self.state
            .lock()
            .await
            .keys
            .iter()
            .map(ApiKeyRecord::info)
            .collect()
    }

    pub async fn create(
        &self,
        name: &str,
        scopes: &[Scope],
        created_by: &str,
    ) -> Result<NewApiKey, ApiKeyError> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_LEN
            || name.chars().any(char::is_control)
        {
            return Err(ApiKeyError::InvalidName);
        }
        let mut scopes = scopes.to_vec();
        scopes.sort_unstable();
        scopes.dedup();
        if scopes.is_empty() {
            return Err(ApiKeyError::NoScopes);
        }

        let (id, token) = generate_token().map_err(ApiKeyError::Random)?;
        let hash = {
            let token = token.clone();
            tokio::task::spawn_blocking(move || hash_token(&token))
                .await
                .map_err(|e| ApiKeyError::Hash(e.to_string()))??
        };

        let mut state = self.state.lock().await;
        if state.keys.len() >= MAX_KEYS {
            return Err(ApiKeyError::TooMany);
        }
        if state.keys.iter().any(|k| k.name == name) {
            return Err(ApiKeyError::DuplicateName);
        }
        let record = ApiKeyRecord {
            id,
            name: name.to_owned(),
            scopes,
            hash,
            created_at: unix_now(),
            created_by: created_by.to_owned(),
        };
        let info = record.info();
        let mut next = state.clone();
        next.keys.push(record);
        self.persist(&next).await?;
        *state = next;
        Ok(NewApiKey { info, token })
    }

    /// Deletes a key. Returns `false` if no key has this id.
    pub async fn delete(&self, id: &str) -> Result<bool, ApiKeyError> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        next.keys.retain(|k| k.id != id);
        if next.keys.len() == state.keys.len() {
            return Ok(false);
        }
        self.persist(&next).await?;
        *state = next;
        drop(state);
        self.lock_cache()
            .retain(|_, (cached_id, _)| cached_id != id);
        Ok(true)
    }

    /// Resolves a presented token to its key, or `None` if it is not valid.
    pub async fn authenticate(&self, token: &str) -> Option<ApiKeyInfo> {
        let (id, _) = parse_token(token)?;
        let fingerprint: [u8; 32] = Sha256::digest(token.as_bytes()).into();

        let (record_hash, info) = {
            let state = self.state.lock().await;
            let record = state.keys.iter().find(|k| k.id == id)?;
            (record.hash.clone(), record.info())
        };

        if self.cached(&fingerprint, &info.id) {
            return Some(info);
        }

        let permit = self.verify_permits.clone().acquire_owned().await.ok()?;
        let token = token.to_owned();
        let ok = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            verify_token(&token, &record_hash)
        })
        .await
        .unwrap_or(false);
        if !ok {
            return None;
        }

        // Re-check the key was not deleted while we were hashing.
        if !self.state.lock().await.keys.iter().any(|k| k.id == info.id) {
            return None;
        }
        self.lock_cache()
            .insert(fingerprint, (info.id.clone(), Instant::now() + CACHE_TTL));
        Some(info)
    }

    fn cached(&self, fingerprint: &[u8; 32], id: &str) -> bool {
        let now = Instant::now();
        let mut cache = self.lock_cache();
        cache.retain(|_, (_, expires)| *expires > now);
        cache
            .get(fingerprint)
            .is_some_and(|(cached_id, _)| cached_id == id)
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 32], (String, Instant)>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn persist(&self, state: &KeysFile) -> io::Result<()> {
        let path = self.path.clone();
        let snapshot = state.clone();
        tokio::task::spawn_blocking(move || secure_fs::write_json_private(&path, &snapshot))
            .await
            .map_err(io::Error::other)?
    }
}

/// Returns `(id, token)`.
fn generate_token() -> Result<(String, String), getrandom::Error> {
    let mut id = [0u8; ID_BYTES];
    let mut secret = [0u8; SECRET_BYTES];
    getrandom::fill(&mut id)?;
    getrandom::fill(&mut secret)?;
    let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
    let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
    let token = format!("{TOKEN_PREFIX}{id}_{secret}");
    Ok((id, token))
}

/// Splits a token into `(id, secret)` if it is well-formed.
fn parse_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    let id_ok =
        id.len() == ID_BYTES * 2 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let secret_ok = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(secret)
        .is_ok_and(|s| s.len() == SECRET_BYTES);
    (id_ok && secret_ok).then_some((id, secret))
}

fn hash_token(token: &str) -> Result<String, ApiKeyError> {
    Argon2::default()
        .hash_password(token.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| ApiKeyError::Hash(e.to_string()))
}

fn verify_token(token: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(token.as_bytes(), &hash)
            .is_ok()
    })
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

    #[test]
    fn tokens_are_well_formed_and_unique() {
        let (id1, t1) = generate_token().unwrap();
        let (id2, t2) = generate_token().unwrap();
        assert_ne!(t1, t2);
        assert_ne!(id1, id2);
        assert!(t1.starts_with("gth_"));
        assert_eq!(parse_token(&t1).map(|(id, _)| id), Some(id1.as_str()));
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let (_, good) = generate_token().unwrap();
        let bad = [
            "",
            "gth_",
            "gth_abc_def",
            &good.replacen("gth_", "ghp_", 1),
            &good[..good.len() - 1],
            &format!("{good}A"),
            &good.to_uppercase(),
        ];
        for token in bad {
            assert!(parse_token(token).is_none(), "{token:?}");
        }
    }

    #[test]
    fn argon2_hash_round_trip() {
        let (_, token) = generate_token().unwrap();
        let hash = hash_token(&token).unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains(&token));
        assert!(verify_token(&token, &hash));
        let (_, other) = generate_token().unwrap();
        assert!(!verify_token(&other, &hash));
        assert!(!verify_token(&token, "not a phc string"));
    }

    #[tokio::test]
    async fn create_authenticate_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = ApiKeyStore::load(dir.path()).unwrap();
        let created = store
            .create(
                " monitoring ",
                &[Scope::SystemRead, Scope::SystemRead, Scope::NetworkRead],
                "admin",
            )
            .await
            .unwrap();
        assert_eq!(created.info.name, "monitoring");
        assert_eq!(
            created.info.scopes,
            vec![Scope::SystemRead, Scope::NetworkRead]
        );

        // The plaintext token is not on disk; the file is private.
        let path = dir.path().join(FILE_NAME);
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(&created.token));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        assert_eq!(
            store.authenticate(&created.token).await,
            Some(created.info.clone())
        );
        // Second call is served from the cache and must agree.
        assert_eq!(
            store.authenticate(&created.token).await,
            Some(created.info.clone())
        );

        // A token with the right id but wrong secret fails.
        let (_, other) = generate_token().unwrap();
        let (_, other_secret) = parse_token(&other).unwrap();
        let forged = format!("gth_{}_{}", created.info.id, other_secret);
        assert_eq!(store.authenticate(&forged).await, None);

        // Survives a reload.
        let reloaded = ApiKeyStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.list().await, vec![created.info.clone()]);
        assert!(reloaded.authenticate(&created.token).await.is_some());

        assert!(store.delete(&created.info.id).await.unwrap());
        assert!(!store.delete(&created.info.id).await.unwrap());
        assert_eq!(
            store.authenticate(&created.token).await,
            None,
            "cache invalidated"
        );
        assert!(store.list().await.is_empty());
    }

    #[tokio::test]
    async fn validates_input() {
        let dir = tempfile::tempdir().unwrap();
        let store = ApiKeyStore::load(dir.path()).unwrap();
        assert!(matches!(
            store.create("  ", &[Scope::SystemRead], "admin").await,
            Err(ApiKeyError::InvalidName)
        ));
        assert!(matches!(
            store
                .create("bad\nname", &[Scope::SystemRead], "admin")
                .await,
            Err(ApiKeyError::InvalidName)
        ));
        assert!(matches!(
            store.create("ok", &[], "admin").await,
            Err(ApiKeyError::NoScopes)
        ));
        store
            .create("ok", &[Scope::FirewallRead], "admin")
            .await
            .unwrap();
        assert!(matches!(
            store.create("ok", &[Scope::FirewallRead], "admin").await,
            Err(ApiKeyError::DuplicateName)
        ));
    }

    #[test]
    fn scopes_serialize_as_strings() {
        let json = serde_json::to_string(&Scope::ALL).unwrap();
        assert_eq!(json, r#"["system:read","network:read","firewall:read"]"#);
        assert!(serde_json::from_str::<Scope>(r#""system:write""#).is_err());
    }
}
