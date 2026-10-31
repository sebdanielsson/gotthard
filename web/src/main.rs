//! gotthard-web: the Gotthard router's web UI and JSON API.

mod api;
mod app;
mod assets;
mod auth;
mod command;
mod config;
mod error;
mod secure_fs;
mod state;
mod sysinfo;

use std::{
    io::IsTerminal as _, net::SocketAddr, os::unix::fs::MetadataExt, process::ExitCode, sync::Arc,
};

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::{
    auth::{
        Backend,
        accounts::AccountDb,
        api_keys::ApiKeyStore,
        rate_limit::{Policy, RateLimiter},
        totp::TotpStore,
    },
    config::{Args, Config},
    state::AppState,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const TOTP_ISSUER: &str = "Gotthard";

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        // No colour codes when logging to syslog/files (OpenRC, docker logs).
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    match run(Config::from(args)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "fatal");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: Config) -> Result<(), BoxError> {
    if config.dev {
        tracing::warn!(
            passwd = %config.accounts.passwd.display(),
            shadow = %config.accounts.shadow.display(),
            group = %config.accounts.group.display(),
            "DEVELOPMENT MODE: authenticating against fake account files"
        );
    } else {
        check_production_privileges(&config);
    }
    if !config.secure_cookies {
        tracing::warn!("session cookie is sent without the Secure attribute");
    }

    secure_fs::ensure_private_dir(&config.state_dir)
        .map_err(|e| format!("state dir {}: {e}", config.state_dir.display()))?;
    let state = AppState {
        totp: Arc::new(
            TotpStore::load(&config.state_dir, TOTP_ISSUER)
                .map_err(|e| format!("loading TOTP state: {e}"))?,
        ),
        api_keys: Arc::new(
            ApiKeyStore::load(&config.state_dir).map_err(|e| format!("loading API keys: {e}"))?,
        ),
        limiter: Arc::new(RateLimiter::new(Policy::default())),
        hostname: sysinfo::hostname().into(),
    };
    let backend = Backend::new(AccountDb::new(config.accounts.clone()), &config.admin_group);
    let app = app::build(state, backend, &config);

    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(|e| format!("binding {}: {e}", config.listen))?;
    tracing::info!(
        address = %listener.local_addr()?,
        state_dir = %config.state_dir.display(),
        admin_group = %config.admin_group,
        "gotthard-web listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    tracing::info!("shut down");
    Ok(())
}

/// Warns (does not refuse) about setups that will not work in production.
fn check_production_privileges(config: &Config) {
    // /proc/self is owned by the effective uid; avoids libc/unsafe.
    match std::fs::metadata("/proc/self") {
        Ok(meta) if meta.uid() != 0 => tracing::warn!(
            uid = meta.uid(),
            "not running as root: /etc/shadow is probably unreadable and logins will fail"
        ),
        Ok(_) => {}
        Err(err) => tracing::warn!(%err, "cannot determine effective uid"),
    }
    if let Ok(meta) = std::fs::metadata(&config.state_dir)
        && meta.uid() != 0
    {
        tracing::warn!(
            state_dir = %config.state_dir.display(),
            owner = meta.uid(),
            "state dir is not owned by root"
        );
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(%err, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!(%err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown requested");
}
