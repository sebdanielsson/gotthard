//! Command-line configuration.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use clap::Parser;

const DEFAULT_STATE_DIR: &str = "/etc/gotthard/auth";
const DEV_STATE_DIR: &str = "dev-state";
const DEV_PASSWD: &str = "dev/passwd";
const DEV_SHADOW: &str = "dev/shadow";
const DEV_GROUP: &str = "dev/group";

#[derive(Debug, Parser)]
#[command(
    name = "gotthard-web",
    version,
    about = "Gotthard router web UI and JSON API"
)]
pub struct Args {
    /// Address and port to listen on.
    #[arg(long, env = "GOTTHARD_WEB_LISTEN", default_value = "127.0.0.1:8080")]
    pub listen: SocketAddr,

    /// Directory holding TOTP secrets and API keys (created 0700, files 0600).
    /// Defaults to /etc/gotthard/auth, or ./dev-state with --dev.
    #[arg(long, env = "GOTTHARD_WEB_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Development mode: authenticate against fake account files instead of
    /// /etc/{passwd,shadow,group} and relax the root-only state checks.
    #[arg(long)]
    pub dev: bool,

    /// passwd file to use in --dev mode.
    #[arg(long, requires = "dev", default_value = DEV_PASSWD)]
    pub passwd_file: PathBuf,

    /// shadow file to use in --dev mode.
    #[arg(long, requires = "dev", default_value = DEV_SHADOW)]
    pub shadow_file: PathBuf,

    /// group file to use in --dev mode.
    #[arg(long, requires = "dev", default_value = DEV_GROUP)]
    pub group_file: PathBuf,

    /// Group whose members may log in.
    #[arg(long, default_value = "wheel")]
    pub admin_group: String,

    /// Session idle timeout in minutes.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=1440))]
    pub session_idle_minutes: u32,

    /// Drop the `Secure` attribute from the session cookie. Only needed when
    /// testing over plain HTTP on a non-localhost address (or with Safari).
    #[arg(long)]
    pub insecure_cookies: bool,
}

/// Locations of the account databases used for authentication.
#[derive(Debug, Clone)]
pub struct AccountPaths {
    pub passwd: PathBuf,
    pub shadow: PathBuf,
    pub group: PathBuf,
}

impl AccountPaths {
    pub fn system() -> Self {
        Self {
            passwd: PathBuf::from("/etc/passwd"),
            shadow: PathBuf::from("/etc/shadow"),
            group: PathBuf::from("/etc/group"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    pub dev: bool,
    pub accounts: AccountPaths,
    pub admin_group: String,
    pub session_idle: Duration,
    pub secure_cookies: bool,
}

impl From<Args> for Config {
    fn from(args: Args) -> Self {
        let state_dir = args.state_dir.unwrap_or_else(|| {
            PathBuf::from(if args.dev {
                DEV_STATE_DIR
            } else {
                DEFAULT_STATE_DIR
            })
        });
        let accounts = if args.dev {
            AccountPaths {
                passwd: args.passwd_file,
                shadow: args.shadow_file,
                group: args.group_file,
            }
        } else {
            AccountPaths::system()
        };
        Self {
            listen: args.listen,
            state_dir,
            dev: args.dev,
            accounts,
            admin_group: args.admin_group,
            session_idle: Duration::from_secs(u64::from(args.session_idle_minutes) * 60),
            secure_cookies: !args.insecure_cookies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, clap::Error> {
        Args::try_parse_from(std::iter::once("gotthard-web").chain(args.iter().copied()))
            .map(Config::from)
    }

    #[test]
    fn production_defaults() {
        let config = parse(&[]).unwrap();
        assert_eq!(config.listen.to_string(), "127.0.0.1:8080");
        assert_eq!(config.state_dir, PathBuf::from("/etc/gotthard/auth"));
        assert_eq!(config.accounts.shadow, PathBuf::from("/etc/shadow"));
        assert!(config.secure_cookies);
        assert!(!config.dev);
    }

    #[test]
    fn dev_mode_uses_fake_accounts() {
        let config = parse(&["--dev", "--shadow-file", "/tmp/fake-shadow"]).unwrap();
        assert_eq!(config.accounts.shadow, PathBuf::from("/tmp/fake-shadow"));
        assert_eq!(config.accounts.passwd, PathBuf::from("dev/passwd"));
        assert_eq!(config.state_dir, PathBuf::from("dev-state"));
    }

    #[test]
    fn account_overrides_require_dev() {
        assert!(parse(&["--shadow-file", "/tmp/fake-shadow"]).is_err());
    }
}
