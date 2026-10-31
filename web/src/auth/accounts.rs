//! Lookups in the local account databases (`passwd`, `shadow`, `group`).
//!
//! The files are re-read on every lookup: they are tiny, and this way password
//! changes, locks and group removals take effect immediately.

use std::{io, path::Path};

use crate::config::AccountPaths;

/// What we need to know about a local account to authenticate it.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    /// The crypt(3) hash from the shadow file (`$6$...`, `$y$...`, ...).
    pub password_hash: String,
    /// Whether the account may log in at all (not locked, has a password,
    /// not expired).
    pub usable: bool,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("username", &self.username)
            .field("uid", &self.uid)
            .field("gid", &self.gid)
            .field("usable", &self.usable)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct AccountDb {
    paths: AccountPaths,
}

impl AccountDb {
    pub fn new(paths: AccountPaths) -> Self {
        Self { paths }
    }

    /// Looks up `username`. Returns `None` if it is not a valid name or is
    /// missing from either `passwd` or `shadow`.
    pub fn lookup(&self, username: &str, today: u64) -> Result<Option<Account>, AccountError> {
        if !is_valid_username(username) {
            return Ok(None);
        }
        let passwd = read(&self.paths.passwd)?;
        let Some(pw) = find_passwd(&passwd, username) else {
            return Ok(None);
        };
        let shadow = read(&self.paths.shadow)?;
        let Some(sp) = find_shadow(&shadow, username) else {
            return Ok(None);
        };
        let usable = sp.is_usable(today);
        Ok(Some(Account {
            username: username.to_owned(),
            uid: pw.uid,
            gid: pw.gid,
            password_hash: sp.hash.to_owned(),
            usable,
        }))
    }

    /// Whether `account` belongs to `group`, either as its primary group or
    /// as a listed supplementary member.
    pub fn is_member(&self, account: &Account, group: &str) -> Result<bool, AccountError> {
        let groups = read(&self.paths.group)?;
        Ok(group_has_member(
            &groups,
            group,
            &account.username,
            account.gid,
        ))
    }
}

fn read(path: &Path) -> Result<String, AccountError> {
    std::fs::read_to_string(path).map_err(|source| AccountError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Days since the Unix epoch, as used by the shadow file.
pub fn days_since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

/// Conservative POSIX-ish username check (what `adduser` on Alpine accepts).
/// Rejecting odd input early keeps it out of logs and file scans.
pub fn is_valid_username(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= 32
        && (first.is_ascii_lowercase() || first == b'_')
        && rest.iter().enumerate().all(|(i, &b)| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || b == b'_'
                || b == b'-'
                || b == b'.'
                // A trailing `$` is allowed (Samba machine accounts).
                || (b == b'$' && i == rest.len() - 1)
        })
}

fn records<'a>(contents: &'a str) -> impl Iterator<Item = Vec<&'a str>> + 'a {
    contents
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split(':').collect())
}

#[derive(Debug, PartialEq, Eq)]
struct PasswdEntry {
    uid: u32,
    gid: u32,
}

fn find_passwd(contents: &str, username: &str) -> Option<PasswdEntry> {
    records(contents)
        .filter(|f| f.len() >= 7 && f[0] == username)
        .find_map(|f| {
            Some(PasswdEntry {
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
            })
        })
}

#[derive(Debug, PartialEq, Eq)]
struct ShadowEntry<'a> {
    hash: &'a str,
    /// Absolute expiry date, in days since the epoch (field 8).
    expires: Option<u64>,
}

impl ShadowEntry<'_> {
    fn is_usable(&self, today: u64) -> bool {
        let locked =
            self.hash.is_empty() || self.hash.starts_with('!') || self.hash.starts_with('*');
        let expired = self.expires.is_some_and(|day| today >= day);
        !locked && !expired
    }
}

fn find_shadow<'a>(contents: &'a str, username: &str) -> Option<ShadowEntry<'a>> {
    contents
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split(':');
            if fields.next()? != username {
                return None;
            }
            let hash = fields.next()?;
            let expires = fields.nth(5).and_then(|f| f.parse().ok());
            Some(ShadowEntry { hash, expires })
        })
}

fn group_has_member(contents: &str, group: &str, username: &str, primary_gid: u32) -> bool {
    records(contents)
        .filter(|f| f.len() >= 4 && f[0] == group)
        .any(|f| {
            f[2].parse::<u32>().is_ok_and(|gid| gid == primary_gid)
                || f[3].split(',').map(str::trim).any(|m| m == username)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/ash
# comment
admin:x:1000:1000:Admin:/home/admin:/bin/ash
operator:x:1001:1001::/home/operator:/bin/ash
broken:x:notanumber:1002::/:/sbin/nologin
";

    const SHADOW: &str = "\
root:!::0:::::
admin:$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1:20000:0:99999:7:::
operator:$6$abc$def:20000:0:99999:7::19000:
nopass::20000:0:99999:7:::
";

    const GROUP: &str = "\
root:x:0:root
wheel:x:10:root,admin
operator:x:1001:
staff:x:50:
";

    #[test]
    fn username_validation() {
        for ok in ["admin", "_svc", "a-b.c_d", "machine$", "u1"] {
            assert!(is_valid_username(ok), "{ok} should be valid");
        }
        for bad in [
            "",
            "Admin",
            "1abc",
            "a b",
            "a:b",
            "ab$c",
            "../etc",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_username(bad), "{bad} should be invalid");
        }
    }

    #[test]
    fn parses_passwd_entries() {
        assert_eq!(
            find_passwd(PASSWD, "admin"),
            Some(PasswdEntry {
                uid: 1000,
                gid: 1000
            })
        );
        assert_eq!(find_passwd(PASSWD, "nobody"), None);
        assert_eq!(find_passwd(PASSWD, "broken"), None);
        // Prefix matches must not count.
        assert_eq!(find_passwd(PASSWD, "adm"), None);
    }

    #[test]
    fn parses_shadow_entries() {
        let admin = find_shadow(SHADOW, "admin").unwrap();
        assert!(admin.hash.starts_with("$6$saltstring$"));
        assert_eq!(admin.expires, None);
        assert!(admin.is_usable(20_500));

        let operator = find_shadow(SHADOW, "operator").unwrap();
        assert_eq!(operator.expires, Some(19_000));
        assert!(operator.is_usable(18_999));
        assert!(!operator.is_usable(19_000), "expired accounts are unusable");

        assert!(!find_shadow(SHADOW, "root").unwrap().is_usable(0), "locked");
        assert!(
            !find_shadow(SHADOW, "nopass").unwrap().is_usable(0),
            "empty hash"
        );
        assert!(find_shadow(SHADOW, "ghost").is_none());
    }

    #[test]
    fn group_membership() {
        assert!(group_has_member(GROUP, "wheel", "admin", 1000));
        assert!(!group_has_member(GROUP, "wheel", "operator", 1001));
        // Primary group membership counts too.
        assert!(group_has_member(GROUP, "staff", "operator", 50));
        assert!(!group_has_member(GROUP, "nonexistent", "admin", 1000));
        // No substring matches in member lists.
        assert!(!group_has_member(GROUP, "wheel", "adm", 999));
    }

    #[test]
    fn lookup_reads_files() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, contents: &str| {
            let path = dir.path().join(name);
            std::fs::File::create(&path)
                .unwrap()
                .write_all(contents.as_bytes())
                .unwrap();
            path
        };
        let db = AccountDb::new(AccountPaths {
            passwd: write("passwd", PASSWD),
            shadow: write("shadow", SHADOW),
            group: write("group", GROUP),
        });

        let admin = db.lookup("admin", 20_500).unwrap().unwrap();
        assert_eq!(admin.uid, 1000);
        assert!(admin.usable);
        assert!(db.is_member(&admin, "wheel").unwrap());

        assert!(db.lookup("ghost", 20_500).unwrap().is_none());
        assert!(db.lookup("../../etc/passwd", 20_500).unwrap().is_none());

        // Debug output must never contain the hash.
        assert!(!format!("{admin:?}").contains("$6$"));
    }

    #[test]
    fn lookup_reports_missing_files() {
        let db = AccountDb::new(AccountPaths {
            passwd: "/nonexistent/passwd".into(),
            shadow: "/nonexistent/shadow".into(),
            group: "/nonexistent/group".into(),
        });
        assert!(matches!(
            db.lookup("admin", 0),
            Err(AccountError::Io { .. })
        ));
    }
}
