#[cfg(any(target_os = "macos", test))]
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(any(target_os = "macos", test))]
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::Mutex;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Desktop OAuth registration of the Antigravity client. Daily does not ship the client secret:
// users who opt into this unofficial adapter supply it (see docs/subscription-adapters.md).
const DEFAULT_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub(crate) const CLIENT_ID_ENV: &str = "DAILY_ANTIGRAVITY_CLIENT_ID";
pub(crate) const CLIENT_SECRET_ENV: &str = "DAILY_ANTIGRAVITY_CLIENT_SECRET";
#[cfg(any(target_os = "macos", test))]
const MAX_CREDENTIAL_BYTES: usize = 2 * 1024 * 1024;
static AUTH_CACHE: OnceLock<Mutex<Option<CachedAuth>>> = OnceLock::new();

/// Where and as which OAuth client access tokens are refreshed.
pub(super) struct TokenEndpoint {
    pub(super) url: String,
    pub(super) client_id: String,
    pub(super) client_secret: String,
}

impl TokenEndpoint {
    fn google() -> Self {
        let env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        Self {
            url: TOKEN_URL.into(),
            client_id: env(CLIENT_ID_ENV).unwrap_or_else(|| DEFAULT_CLIENT_ID.into()),
            client_secret: env(CLIENT_SECRET_ENV).unwrap_or_default(),
        }
    }
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AntigravityAuthStatusView {
    pub authenticated: bool,
    pub email: Option<String>,
    pub app_available: bool,
    pub app_path: Option<String>,
    pub expires_at: Option<String>,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AntigravityLoginStartView {
    pub opened: bool,
    pub status: AntigravityAuthStatusView,
}

#[derive(Clone)]
pub(super) struct AuthSnapshot {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry: u64,
    pub expires_at: Option<String>,
    pub email: Option<String>,
}

struct CachedAuth {
    source_access: String,
    source_refresh: String,
    auth: AuthSnapshot,
    revoked: bool,
}

struct AuthError {
    code: &'static str,
    message: String,
}

fn auth_cache() -> &'static Mutex<Option<CachedAuth>> {
    AUTH_CACHE.get_or_init(|| Mutex::new(None))
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn app_path() -> Option<String> {
    let mut paths = vec![std::path::PathBuf::from("/Applications/Antigravity.app")];
    if let Some(home) = crate::util::home_dir() {
        paths.push(home.join("Applications/Antigravity.app"));
    }
    paths
        .into_iter()
        .find(|p| p.is_dir())
        .map(|p| p.display().to_string())
}

pub(super) fn app_version() -> Option<String> {
    let path = std::path::PathBuf::from(app_path()?).join("Contents/Info.plist");
    let plist = std::fs::read_to_string(path).ok()?;
    let entry = plist
        .split("<key>CFBundleShortVersionString</key>")
        .nth(1)?;
    let version = entry
        .split("<string>")
        .nth(1)?
        .split("</string>")
        .next()?
        .trim();
    (!version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .then(|| version.to_owned())
}

#[cfg(target_os = "macos")]
fn read_keychain() -> Result<AuthSnapshot, AuthError> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AuthError {
            code: "keychain_unavailable",
            message: "Could not access the macOS Keychain.".into(),
        })?;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AuthError {
                    code: "access_denied",
                    message: "Keychain access was denied or timed out. Allow access, then refresh."
                        .into(),
                });
            }
        }
    }
    let output = child.wait_with_output().map_err(|_| AuthError {
        code: "keychain_unavailable",
        message: "Could not read the Antigravity Keychain credentials.".into(),
    })?;
    if !output.status.success() {
        return Err(if output.status.code() == Some(44) {
            AuthError {
                code: "not_logged_in",
                message: "Sign in to the Antigravity client with a personal Google account first."
                    .into(),
            }
        } else {
            AuthError {
                code: "access_denied",
                message: "Could not read the Keychain. Allow access, then refresh.".into(),
            }
        });
    }
    parse_credentials(&output.stdout).map_err(|message| AuthError {
        code: "invalid_credentials",
        message,
    })
}

#[cfg(not(target_os = "macos"))]
fn read_keychain() -> Result<AuthSnapshot, AuthError> {
    Err(AuthError {
        code: "unsupported_platform",
        message: "Antigravity sign-in is only supported on macOS.".into(),
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_credentials(raw: &[u8]) -> Result<AuthSnapshot, String> {
    if raw.len() > MAX_CREDENTIAL_BYTES {
        return Err("The Antigravity credentials exceed the size limit.".into());
    }
    let raw = std::str::from_utf8(raw)
        .map_err(|_| "The Antigravity credentials are not valid UTF-8.")?
        .trim();
    let decoded;
    let payload = if let Some(encoded) = raw.strip_prefix("go-keyring-base64:") {
        decoded = STANDARD
            .decode(encoded)
            .map_err(|_| "The Antigravity credentials are not valid Base64.")?;
        decoded.as_slice()
    } else {
        raw.as_bytes()
    };
    let document: Value = serde_json::from_slice(payload)
        .map_err(|_| "The Antigravity credentials are not valid JSON.")?;
    if document
        .get("auth_method")
        .and_then(Value::as_str)
        .is_some_and(|method| method != "consumer")
    {
        return Err("Only personal Google accounts are supported. Switch the sign-in method in the official client.".into());
    }
    let token = document.get("token").unwrap_or(&document);
    let access_token = string_field(token, "access_token").unwrap_or_default();
    let refresh_token = string_field(token, "refresh_token").unwrap_or_default();
    if access_token.is_empty() && refresh_token.is_empty() {
        return Err(
            "The Antigravity credentials have no token. Sign in again in the official client."
                .into(),
        );
    }
    let expires_at = string_field(token, "expiry");
    let expiry = expires_at
        .as_deref()
        .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
        .map(|time| time.unix_timestamp().max(0) as u64)
        .unwrap_or(0);
    let email = string_field(&document, "email").or_else(|| {
        let jwt = document.get("id_token").and_then(Value::as_str)?;
        let payload = URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1)?).ok()?;
        let claims: Value = serde_json::from_slice(&payload).ok()?;
        string_field(&claims, "email")
    });
    Ok(AuthSnapshot {
        access_token,
        refresh_token,
        expiry,
        expires_at,
        email,
    })
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

async fn read_source() -> Result<AuthSnapshot, AuthError> {
    tokio::task::spawn_blocking(read_keychain)
        .await
        .map_err(|_| AuthError {
            code: "keychain_unavailable",
            message: "Could not read the Antigravity credentials.".into(),
        })?
}

pub async fn antigravity_auth_status() -> AntigravityAuthStatusView {
    let app_path = app_path();
    let mut view = AntigravityAuthStatusView {
        authenticated: false,
        email: None,
        app_available: app_path.is_some(),
        app_path,
        expires_at: None,
        error: None,
        error_code: None,
    };
    match read_source().await {
        Ok(source) => {
            let mut cache = auth_cache().lock().await;
            let matched = cache.as_ref().filter(|cached| {
                cached.source_access == source.access_token
                    && cached.source_refresh == source.refresh_token
            });
            view.authenticated = (!source.refresh_token.is_empty() || source.expiry > now())
                && !matched.is_some_and(|c| c.revoked);
            view.email = matched.and_then(|c| c.auth.email.clone()).or(source.email);
            view.expires_at = source.expires_at;
            if matched.is_none() {
                *cache = None;
            }
            if !view.authenticated {
                view.error = Some(
                    "The Antigravity authorization expired. Sign in again in the official client."
                        .into(),
                );
                view.error_code = Some("not_logged_in".into());
            }
        }
        Err(error) => {
            *auth_cache().lock().await = None;
            view.error = Some(error.message);
            view.error_code = Some(error.code.into());
        }
    }
    view
}

pub async fn start_antigravity_login() -> Result<AntigravityLoginStartView, String> {
    #[cfg(not(target_os = "macos"))]
    return Err("Antigravity sign-in is only supported on macOS.".into());
    #[cfg(target_os = "macos")]
    {
        if app_path().is_none() {
            return Err(
                "Antigravity 2.x was not found. Install the official macOS client first.".into(),
            );
        }
        let opened = tokio::task::spawn_blocking(|| {
            std::process::Command::new("/usr/bin/open")
                .args(["-b", "com.google.antigravity"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .await
        .map_err(|_| "Could not open the Antigravity client.")?;
        if !opened {
            return Err("Could not open the Antigravity client.".into());
        }
        Ok(AntigravityLoginStartView {
            opened,
            status: antigravity_auth_status().await,
        })
    }
}

pub(super) async fn credentials(
    http: &Client,
    rejected_token: Option<&str>,
) -> Result<AuthSnapshot, String> {
    let source = match read_source().await {
        Ok(source) => source,
        Err(error) => {
            *auth_cache().lock().await = None;
            return Err(error.message);
        }
    };
    let mut cache = auth_cache().lock().await;
    resolve_credentials(
        http,
        &mut cache,
        source,
        rejected_token,
        &TokenEndpoint::google(),
    )
    .await
}

async fn resolve_credentials(
    http: &Client,
    cache: &mut Option<CachedAuth>,
    source: AuthSnapshot,
    rejected_token: Option<&str>,
    endpoint: &TokenEndpoint,
) -> Result<AuthSnapshot, String> {
    let same_source = cache.as_ref().is_some_and(|cached| {
        cached.source_access == source.access_token && cached.source_refresh == source.refresh_token
    });
    if !same_source {
        *cache = None;
    }
    if cache.as_ref().is_some_and(|cached| cached.revoked) {
        return Err("The Antigravity authorization is no longer valid. Sign in again in the official client.".into());
    }
    let mut auth = cache
        .as_ref()
        .map(|cached| cached.auth.clone())
        .unwrap_or_else(|| source.clone());
    // A concurrent request may already have replaced the rejected access token.
    let force = rejected_token.is_some_and(|token| token == auth.access_token);
    if force || auth.access_token.is_empty() || auth.expiry <= now() + 60 {
        if auth.refresh_token.is_empty() {
            *cache = None;
            return Err(
                "The Antigravity authorization expired. Sign in again in the official client."
                    .into(),
            );
        }
        if endpoint.client_secret.is_empty() {
            return Err(format!(
                "Refreshing the Antigravity token needs the client's OAuth secret. Set {CLIENT_SECRET_ENV} (see docs/subscription-adapters.md) and restart Daily."
            ));
        }
        let response = http
            .post(&endpoint.url)
            .timeout(Duration::from_secs(20))
            .form(&[
                ("client_id", endpoint.client_id.as_str()),
                ("client_secret", endpoint.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", &auth.refresh_token),
            ])
            .send()
            .await
            .map_err(|_| {
                "Could not reach Google's authorization service. Check the network or proxy."
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let invalid =
                response.json::<Value>().await.ok().is_some_and(|v| {
                    v.get("error").and_then(Value::as_str) == Some("invalid_grant")
                });
            *cache = if invalid {
                Some(CachedAuth {
                    source_access: source.access_token,
                    source_refresh: source.refresh_token,
                    auth: auth.clone(),
                    revoked: true,
                })
            } else {
                None
            };
            return Err(if invalid {
                "The Antigravity authorization is no longer valid. Sign in again in the official client.".into()
            } else {
                format!("Refreshing the Antigravity token failed (HTTP {status}).")
            });
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|_| "Google returned an invalid token refresh response.")?;
        auth.access_token = string_field(&payload, "access_token")
            .ok_or("Google's token refresh response has no access token.")?;
        auth.refresh_token = string_field(&payload, "refresh_token").unwrap_or(auth.refresh_token);
        auth.expiry = now()
            + payload
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(3600);
    }
    *cache = Some(CachedAuth {
        source_access: source.access_token,
        source_refresh: source.refresh_token,
        auth: auth.clone(),
        revoked: false,
    });
    Ok(auth)
}

pub(super) async fn remember_email(access_token: &str, email: Option<String>) {
    if let Some(cache) = auth_cache().lock().await.as_mut() {
        if cache.auth.access_token == access_token {
            cache.auth.email = email;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_endpoint(url: &str) -> TokenEndpoint {
        TokenEndpoint {
            url: url.into(),
            client_id: "test-client".into(),
            client_secret: "test-secret".into(),
        }
    }

    #[tokio::test]
    async fn refresh_without_a_client_secret_explains_how_to_configure_it() {
        let http = Client::new();
        let mut cache = None;
        let source = AuthSnapshot {
            access_token: String::new(),
            refresh_token: "refresh".into(),
            expiry: 0,
            expires_at: None,
            email: None,
        };
        let endpoint = TokenEndpoint {
            url: "http://unused".into(),
            client_id: DEFAULT_CLIENT_ID.into(),
            client_secret: String::new(),
        };
        let error = resolve_credentials(&http, &mut cache, source, None, &endpoint)
            .await
            .err()
            .expect("refresh must fail without a client secret");
        assert!(error.contains(CLIENT_SECRET_ENV), "{error}");
    }

    #[tokio::test]
    async fn expired_token_refresh_is_shared_and_rotation_is_retained() {
        let (url, server) = crate::antigravity::test_support::serve(vec![(
            200,
            "application/json",
            json!({"access_token":"refreshed","refresh_token":"rotated","expires_in":3600})
                .to_string(),
            0,
        )])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let source = AuthSnapshot {
            access_token: "expired".into(),
            refresh_token: "original".into(),
            expiry: 0,
            expires_at: None,
            email: None,
        };
        let shared = Mutex::new(None);
        let mut jobs = Vec::new();
        for _ in 0..3 {
            let (http, source, url, shared) = (&http, source.clone(), &url, &shared);
            jobs.push(async move {
                resolve_credentials(
                    http,
                    &mut *shared.lock().await,
                    source,
                    None,
                    &test_endpoint(url),
                )
                .await
                .unwrap()
            });
        }
        for auth in futures_util::future::join_all(jobs).await {
            assert_eq!(auth.access_token, "refreshed");
            assert_eq!(auth.refresh_token, "rotated");
        }
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejection_of_cached_token_forces_refresh_and_revocation_is_remembered() {
        let (url, server) = crate::antigravity::test_support::serve(vec![
            (
                200,
                "application/json",
                json!({"access_token":"refreshed","expires_in":3600}).to_string(),
                0,
            ),
            (
                400,
                "application/json",
                json!({"error":"invalid_grant","error_description":"secret"}).to_string(),
                0,
            ),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let source = AuthSnapshot {
            access_token: "expired".into(),
            refresh_token: "original".into(),
            expiry: 0,
            expires_at: None,
            email: None,
        };
        let mut cache = None;
        resolve_credentials(
            &http,
            &mut cache,
            source.clone(),
            None,
            &test_endpoint(&url),
        )
        .await
        .unwrap();
        let error = resolve_credentials(
            &http,
            &mut cache,
            source.clone(),
            Some("refreshed"),
            &test_endpoint(&url),
        )
        .await
        .err()
        .unwrap();
        assert!(!error.contains("secret"));
        assert!(cache.as_ref().unwrap().revoked);
        assert!(
            resolve_credentials(&http, &mut cache, source, None, &test_endpoint(&url))
                .await
                .is_err()
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[test]
    fn parses_plain_and_go_keyring_payloads_without_exposing_secrets() {
        let payload = json!({"auth_method":"consumer","email":"test@example.com","token":{"access_token":"access-test","refresh_token":"refresh-test","expiry":"2099-01-01T00:00:00Z"}}).to_string();
        for bytes in [
            payload.clone().into_bytes(),
            format!("go-keyring-base64:{}", STANDARD.encode(payload)).into_bytes(),
        ] {
            let auth = parse_credentials(&bytes).ok().unwrap();
            assert_eq!(auth.email.as_deref(), Some("test@example.com"));
            assert!(auth.expiry > now());
            assert_eq!(auth.refresh_token, "refresh-test");
        }
        for raw in [
            b"invalid secret-value".as_slice(),
            b"{\"token\":{}}",
            b"go-keyring-base64:invalid",
            b"{\"auth_method\":\"enterprise\"}",
        ] {
            let error = parse_credentials(raw).err().unwrap();
            assert!(!error.contains("secret-value"));
        }
    }

    #[tokio::test]
    async fn source_change_invalidates_cached_identity() {
        let http = Client::new();
        let source = AuthSnapshot {
            access_token: "first".into(),
            refresh_token: "r1".into(),
            expiry: now() + 3600,
            expires_at: None,
            email: Some("first@example.com".into()),
        };
        let mut cache = None;
        resolve_credentials(
            &http,
            &mut cache,
            source,
            None,
            &test_endpoint("http://unused"),
        )
        .await
        .unwrap();
        let changed = AuthSnapshot {
            access_token: "second".into(),
            refresh_token: "r2".into(),
            expiry: now() + 3600,
            expires_at: None,
            email: None,
        };
        let result = resolve_credentials(
            &http,
            &mut cache,
            changed,
            None,
            &test_endpoint("http://unused"),
        )
        .await
        .unwrap();
        assert_eq!(result.access_token, "second");
        assert!(result.email.is_none());
    }
}
