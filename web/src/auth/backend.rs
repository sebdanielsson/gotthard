//! axum-login backend authenticating against the local Linux accounts.

use std::sync::Arc;

use axum_login::{AuthUser, AuthnBackend, UserId};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    accounts::{self, Account, AccountDb, AccountError},
    password,
};

/// A logged-in administrator.
#[derive(Clone)]
pub struct User {
    pub username: String,
    pub uid: u32,
    /// SHA-256 of the shadow hash. axum-login stores it in the session and
    /// compares it on every request, so changing the password (or locking
    /// the account) invalidates existing sessions.
    auth_hash: Vec<u8>,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("username", &self.username)
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}

impl User {
    fn from_account(account: &Account) -> Self {
        Self {
            username: account.username.clone(),
            uid: account.uid,
            auth_hash: Sha256::digest(account.password_hash.as_bytes()).to_vec(),
        }
    }
}

impl AuthUser for User {
    type Id = String;

    fn id(&self) -> Self::Id {
        self.username.clone()
    }

    fn session_auth_hash(&self) -> &[u8] {
        &self.auth_hash
    }
}

#[derive(Clone, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error(transparent)]
    Accounts(#[from] AccountError),
    #[error("authentication task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

#[derive(Debug, Clone)]
pub struct Backend {
    accounts: Arc<AccountDb>,
    admin_group: Arc<str>,
}

impl Backend {
    pub fn new(accounts: AccountDb, admin_group: &str) -> Self {
        Self {
            accounts: Arc::new(accounts),
            admin_group: admin_group.into(),
        }
    }

    /// Loads `username` if it is an active member of the admin group.
    fn load_admin(&self, username: &str) -> Result<Option<Account>, AccountError> {
        let Some(account) = self
            .accounts
            .lookup(username, accounts::days_since_epoch())?
        else {
            return Ok(None);
        };
        if !account.usable {
            tracing::debug!(user = %username, "account is locked or expired");
            return Ok(None);
        }
        if !self.accounts.is_member(&account, &self.admin_group)? {
            tracing::info!(user = %username, group = %self.admin_group, "user is not in the admin group");
            return Ok(None);
        }
        Ok(Some(account))
    }
}

impl AuthnBackend for Backend {
    type User = User;
    type Credentials = Credentials;
    type Error = BackendError;

    async fn authenticate(&self, creds: Credentials) -> Result<Option<User>, BackendError> {
        let backend = self.clone();
        tokio::task::spawn_blocking(move || {
            let Some(account) = backend.load_admin(&creds.username)? else {
                // Spend comparable time so the response does not reveal
                // whether the account exists.
                password::dummy_verify(&creds.password);
                return Ok(None);
            };
            match password::verify(&creds.password, &account.password_hash) {
                Ok(true) => Ok(Some(User::from_account(&account))),
                Ok(false) => Ok(None),
                Err(err) => {
                    tracing::warn!(user = %account.username, %err, "cannot verify password hash");
                    Ok(None)
                }
            }
        })
        .await?
    }

    async fn get_user(&self, username: &UserId<Self>) -> Result<Option<User>, BackendError> {
        let backend = self.clone();
        let username = username.clone();
        tokio::task::spawn_blocking(move || {
            Ok(backend
                .load_admin(&username)?
                .map(|account| User::from_account(&account)))
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AccountPaths;
    use std::path::PathBuf;

    fn dev_backend() -> Backend {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dev");
        Backend::new(
            AccountDb::new(AccountPaths {
                passwd: dev.join("passwd"),
                shadow: dev.join("shadow"),
                group: dev.join("group"),
            }),
            "wheel",
        )
    }

    fn creds(username: &str, password: &str) -> Credentials {
        Credentials {
            username: username.into(),
            password: password.into(),
        }
    }

    #[tokio::test]
    async fn authenticates_wheel_members_only() {
        let backend = dev_backend();
        let user = backend
            .authenticate(creds("admin", "gotthard"))
            .await
            .unwrap()
            .expect("admin can log in");
        assert_eq!(user.username, "admin");
        assert!(!user.session_auth_hash().is_empty());

        assert!(
            backend
                .authenticate(creds("admin", "wrong"))
                .await
                .unwrap()
                .is_none()
        );
        // Correct password but not in wheel.
        assert!(
            backend
                .authenticate(creds("operator", "gotthard"))
                .await
                .unwrap()
                .is_none()
        );
        // Locked account.
        assert!(
            backend
                .authenticate(creds("root", ""))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            backend
                .authenticate(creds("ghost", "gotthard"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn get_user_rechecks_membership() {
        let backend = dev_backend();
        let admin = backend
            .get_user(&"admin".to_owned())
            .await
            .unwrap()
            .unwrap();
        let again = backend
            .authenticate(creds("admin", "gotthard"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(admin.session_auth_hash(), again.session_auth_hash());
        assert!(
            backend
                .get_user(&"operator".to_owned())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let c = creds("admin", "hunter2");
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
