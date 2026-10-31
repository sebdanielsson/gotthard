//! End-to-end tests of the HTTP API against the `dev/` account fixtures.

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use clap::Parser as _;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use totp_rs::{Algorithm, Builder, Secret};
use tower::ServiceExt as _;

use super::build;
use crate::{
    auth::{
        Backend, accounts::AccountDb, api_keys::ApiKeyStore, rate_limit::RateLimiter,
        totp::TotpStore,
    },
    config::{Args, Config},
    state::AppState,
};

struct TestApp {
    router: Router,
    _state_dir: tempfile::TempDir,
}

fn test_app() -> TestApp {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dev");
    let state_dir = tempfile::tempdir().unwrap();
    let args = Args::try_parse_from([
        "gotthard-web".as_ref(),
        "--dev".as_ref(),
        "--passwd-file".as_ref(),
        dev.join("passwd").as_os_str(),
        "--shadow-file".as_ref(),
        dev.join("shadow").as_os_str(),
        "--group-file".as_ref(),
        dev.join("group").as_os_str(),
        "--state-dir".as_ref(),
        state_dir.path().as_os_str(),
    ])
    .unwrap();
    let config = Config::from(args);
    let state = AppState {
        totp: Arc::new(TotpStore::load(&config.state_dir, "Gotthard").unwrap()),
        api_keys: Arc::new(ApiKeyStore::load(&config.state_dir).unwrap()),
        limiter: Arc::new(RateLimiter::default()),
        hostname: "testhost".into(),
    };
    let backend = Backend::new(AccountDb::new(config.accounts.clone()), &config.admin_group);
    TestApp {
        router: build(state, backend, &config),
        _state_dir: state_dir,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    json: Value,
}

impl Reply {
    /// `name=value` of the session cookie set by this response, if any.
    fn cookie(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.contains("gotthard_session="))
            .and_then(|v| v.split(';').next())
            .map(str::to_owned)
    }
}

impl TestApp {
    async fn call(&self, method: Method, uri: &str, body: Option<Value>, auth: Auth<'_>) -> Reply {
        let mut req = Request::builder().method(method).uri(uri);
        match auth {
            Auth::None => {}
            Auth::Cookie(c) => req = req.header(header::COOKIE, c),
            Auth::Bearer(t) => req = req.header(header::AUTHORIZATION, format!("Bearer {t}")),
        }
        let body = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            headers,
            json,
        }
    }

    async fn login(&self, username: &str, password: &str) -> Reply {
        self.call(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": username, "password": password })),
            Auth::None,
        )
        .await
    }

    async fn login_cookie(&self) -> String {
        let reply = self.login("admin", "gotthard").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
        reply.cookie().expect("session cookie")
    }
}

#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    Cookie(&'a str),
    Bearer(&'a str),
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn code_for(secret_b32: &str, time: u64) -> String {
    let secret = Secret::try_from_base32(secret_b32).unwrap();
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_secret(secret.as_ref().to_vec())
        .with_account_name("x")
        .build()
        .unwrap()
        .generate(time)
        .to_string()
}

#[tokio::test]
async fn protected_endpoints_require_authentication() {
    let app = test_app();
    for uri in [
        "/api/system",
        "/api/interfaces",
        "/api/firewall/ruleset",
        "/api/auth/me",
        "/api/keys",
    ] {
        let reply = app.call(Method::GET, uri, None, Auth::None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(reply.json["error"]["code"], "unauthenticated", "{uri}");
    }
}

#[tokio::test]
async fn login_sets_a_hardened_session_cookie() {
    let app = test_app();
    let reply = app.login("admin", "gotthard").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json["status"], "ok");
    assert_eq!(reply.json["user"]["username"], "admin");
    assert_eq!(reply.json["user"]["totp_enabled"], false);

    let set_cookie = reply.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(
        set_cookie.starts_with("__Host-gotthard_session="),
        "{set_cookie}"
    );
    for attr in ["HttpOnly", "SameSite=Strict", "Secure", "Path=/"] {
        assert!(set_cookie.contains(attr), "missing {attr}: {set_cookie}");
    }
    let cookie = reply.cookie().unwrap();

    let me = app
        .call(Method::GET, "/api/auth/me", None, Auth::Cookie(&cookie))
        .await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json["username"], "admin");
    assert_eq!(me.json["auth_method"], "session");
    assert_eq!(me.headers[header::CACHE_CONTROL], "no-store");

    let system = app
        .call(Method::GET, "/api/system", None, Auth::Cookie(&cookie))
        .await;
    assert_eq!(system.status, StatusCode::OK);
    assert!(system.json["web_version"].is_string());

    let logout = app
        .call(
            Method::POST,
            "/api/auth/logout",
            None,
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    let me = app
        .call(Method::GET, "/api/auth/me", None, Auth::Cookie(&cookie))
        .await;
    assert_eq!(
        me.status,
        StatusCode::UNAUTHORIZED,
        "session is gone after logout"
    );
}

#[tokio::test]
async fn rejects_bad_credentials_and_non_admins() {
    let app = test_app();
    for (user, pass) in [
        ("admin", "wrong"),
        ("operator", "gotthard"),
        ("ghost", "x"),
        ("root", ""),
    ] {
        let reply = app.login(user, pass).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{user}");
        assert_eq!(reply.json["error"]["code"], "invalid_credentials");
        assert!(reply.cookie().is_none(), "no session for failed login");
    }
    let reply = app
        .call(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "admin"})),
            Auth::None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn repeated_failures_are_rate_limited() {
    let app = test_app();
    for _ in 0..5 {
        assert_eq!(
            app.login("admin", "wrong").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let reply = app.login("admin", "gotthard").await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.json["error"]["code"], "rate_limited");
    assert!(reply.headers.contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn totp_enrolment_and_two_step_login() {
    let app = test_app();
    let cookie = app.login_cookie().await;

    let enroll = app
        .call(
            Method::POST,
            "/api/auth/totp/enroll",
            None,
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(enroll.status, StatusCode::OK);
    let secret = enroll.json["secret"].as_str().unwrap().to_owned();
    assert!(
        enroll.json["otpauth_url"]
            .as_str()
            .unwrap()
            .contains("admin%40testhost")
    );

    let wrong = app
        .call(
            Method::POST,
            "/api/auth/totp/verify",
            Some(json!({"code": "000000"})),
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.json["error"]["code"], "invalid_totp_code");

    let t = now();
    let verify = app
        .call(
            Method::POST,
            "/api/auth/totp/verify",
            Some(json!({"code": code_for(&secret, t)})),
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(verify.status, StatusCode::OK, "{}", verify.json);
    assert_eq!(verify.json["enabled"], true);

    // Password alone is no longer enough.
    let step1 = app.login("admin", "gotthard").await;
    assert_eq!(step1.status, StatusCode::OK);
    assert_eq!(step1.json["status"], "totp_required");
    let pending = step1.cookie().expect("pending session cookie");
    let me = app
        .call(Method::GET, "/api/auth/me", None, Auth::Cookie(&pending))
        .await;
    assert_eq!(
        me.status,
        StatusCode::UNAUTHORIZED,
        "half-logged-in session has no access"
    );

    let bad = app
        .call(
            Method::POST,
            "/api/auth/login/totp",
            Some(json!({"code": "123456"})),
            Auth::Cookie(&pending),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);

    let step2 = app
        .call(
            Method::POST,
            "/api/auth/login/totp",
            Some(json!({"code": code_for(&secret, t + 30)})),
            Auth::Cookie(&pending),
        )
        .await;
    assert_eq!(step2.status, StatusCode::OK, "{}", step2.json);
    assert_eq!(step2.json["user"]["totp_enabled"], true);
    let full = step2.cookie().expect("session id is cycled on login");
    assert_ne!(full, pending);
    let me = app
        .call(Method::GET, "/api/auth/me", None, Auth::Cookie(&full))
        .await;
    assert_eq!(me.status, StatusCode::OK);

    // A TOTP step without a pending login is refused.
    let orphan = app
        .call(
            Method::POST,
            "/api/auth/login/totp",
            Some(json!({"code": "123456"})),
            Auth::None,
        )
        .await;
    assert_eq!(orphan.json["error"]["code"], "no_pending_login");
}

#[tokio::test]
async fn api_keys_are_scoped_and_cannot_manage_keys() {
    let app = test_app();
    let cookie = app.login_cookie().await;

    let created = app
        .call(
            Method::POST,
            "/api/keys",
            Some(json!({"name": "monitoring", "scopes": ["system:read"]})),
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let token = created.json["token"].as_str().unwrap().to_owned();
    let id = created.json["id"].as_str().unwrap().to_owned();

    let listed = app
        .call(Method::GET, "/api/keys", None, Auth::Cookie(&cookie))
        .await;
    assert_eq!(listed.json["keys"][0]["name"], "monitoring");
    assert!(listed.json["keys"][0].get("token").is_none());
    assert!(listed.json["keys"][0].get("hash").is_none());

    let system = app
        .call(Method::GET, "/api/system", None, Auth::Bearer(&token))
        .await;
    assert_eq!(system.status, StatusCode::OK);
    let me = app
        .call(Method::GET, "/api/auth/me", None, Auth::Bearer(&token))
        .await;
    assert_eq!(me.json["auth_method"], "api_key");
    assert_eq!(me.json["api_key"]["name"], "monitoring");

    let firewall = app
        .call(
            Method::GET,
            "/api/firewall/ruleset",
            None,
            Auth::Bearer(&token),
        )
        .await;
    assert_eq!(firewall.status, StatusCode::FORBIDDEN);

    for (method, uri) in [
        (Method::GET, "/api/keys"),
        (Method::POST, "/api/auth/totp/enroll"),
    ] {
        let reply = app.call(method, uri, None, Auth::Bearer(&token)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{uri}");
    }

    let bogus = app
        .call(Method::GET, "/api/system", None, Auth::Bearer("gth_nope"))
        .await;
    assert_eq!(bogus.status, StatusCode::UNAUTHORIZED);

    let deleted = app
        .call(
            Method::DELETE,
            &format!("/api/keys/{id}"),
            None,
            Auth::Cookie(&cookie),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let revoked = app
        .call(Method::GET, "/api/system", None, Auth::Bearer(&token))
        .await;
    assert_eq!(revoked.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_api_routes_are_json_404s_with_security_headers() {
    let app = test_app();
    let reply = app.call(Method::GET, "/api/nope", None, Auth::None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json["error"]["code"], "not_found");
    assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(reply.headers[header::X_FRAME_OPTIONS], "DENY");
    assert!(
        reply.headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
}

#[tokio::test]
async fn missing_static_files_are_404_and_routes_fall_back_to_the_spa() {
    let app = test_app();
    let missing = app
        .call(Method::GET, "/does-not-exist.js", None, Auth::None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    // Either the SPA shell (UI built) or a 503 explaining it is not built.
    let route = app.call(Method::GET, "/settings", None, Auth::None).await;
    assert!(
        route.status == StatusCode::OK || route.status == StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        route.status
    );
}
