//! Codex ChatGPT login: auth.json loading, token refresh and persistence.

use super::*;

#[derive(Clone)]
pub(super) struct CodexAuthSnapshot {
    pub(super) path: PathBuf,
    pub(super) document: Value,
    pub(super) auth_mode: Option<String>,
    pub(super) access_token: String,
    pub(super) refresh_token: String,
    pub(super) id_token: Option<String>,
    pub(super) email: Option<String>,
    pub(super) account_id: Option<String>,
    pub(super) plan_type: Option<String>,
    pub(super) is_fedramp_account: bool,
}

#[derive(Default)]
pub(super) struct CodexJwtClaims {
    pub(super) email: Option<String>,
    pub(super) account_id: Option<String>,
    pub(super) plan_type: Option<String>,
    pub(super) is_fedramp_account: bool,
}

pub fn codex_auth_status(codex_home: Option<&str>) -> CodexAuthStatusView {
    let codex_home_path = codex_home_path(codex_home).unwrap_or_else(|_| PathBuf::from(".codex"));
    let auth_path = codex_home_path.join(CODEX_AUTH_FILE);
    let cli_path = resolve_codex_binary();

    match load_codex_auth(codex_home) {
        Ok(auth) => CodexAuthStatusView {
            provider: "codex".into(),
            codex_home: codex_home_path.display().to_string(),
            auth_path: auth_path.display().to_string(),
            authenticated: true,
            auth_mode: auth.auth_mode,
            email: auth.email,
            account_id: auth.account_id,
            plan_type: auth.plan_type,
            is_fedramp_account: auth.is_fedramp_account,
            codex_cli_available: cli_path.is_some(),
            codex_cli_path: cli_path.map(|path| path.display().to_string()),
            error: None,
        },
        Err(error) => CodexAuthStatusView {
            provider: "codex".into(),
            codex_home: codex_home_path.display().to_string(),
            auth_path: auth_path.display().to_string(),
            authenticated: false,
            auth_mode: None,
            email: None,
            account_id: None,
            plan_type: None,
            is_fedramp_account: false,
            codex_cli_available: cli_path.is_some(),
            codex_cli_path: cli_path.map(|path| path.display().to_string()),
            error: Some(error),
        },
    }
}

pub fn start_codex_login(input: StartCodexLoginInput) -> Result<CodexLoginStartView, String> {
    let codex_home = codex_home_path(input.codex_home.as_deref())?;
    fs::create_dir_all(&codex_home).map_err(|error| {
        format!(
            "Failed to create Codex home directory {}: {error}",
            codex_home.display()
        )
    })?;
    let codex_bin = resolve_codex_binary().ok_or_else(|| {
        "Codex CLI was not found. Install Codex and make sure the `codex` command is on PATH, or set DAILY_CODEX_BIN to the executable path.".to_string()
    })?;

    let mut child = Command::new(&codex_bin)
        .arg("login")
        .env("CODEX_HOME", &codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "Failed to start Codex login using {}: {error}",
                codex_bin.display()
            )
        })?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(CodexLoginStartView {
        pid,
        command: format!(
            "CODEX_HOME={} {} login",
            codex_home.display(),
            codex_bin.display()
        ),
        status: codex_auth_status(input.codex_home.as_deref()),
    })
}

pub(super) fn load_codex_auth(codex_home: Option<&str>) -> Result<CodexAuthSnapshot, String> {
    let path = codex_auth_path(codex_home)?;
    let raw = read_limited_utf8(&path, MAX_CODEX_AUTH_FILE_BYTES, "Codex auth file")?;
    let document: Value = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Failed to parse Codex auth file at {} as JSON: {error}",
            path.display()
        )
    })?;

    let auth_mode = document
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(auth_mode) = auth_mode.as_deref() {
        if auth_mode != "chatgpt" && auth_mode != "chatgptAuthTokens" {
            return Err(format!(
                "Codex login mode requires ChatGPT login tokens, but {} uses auth_mode={}. Run Codex login with ChatGPT first.",
                path.display(),
                auth_mode
            ));
        }
    }

    let tokens = document
        .get("tokens")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "Codex login mode requires ChatGPT login tokens in {}. Run Codex login with ChatGPT first.",
                path.display()
            )
        })?;
    let access_token = token_field(tokens, "access_token").ok_or_else(|| {
        format!(
            "Codex auth file at {} does not contain a ChatGPT access token.",
            path.display()
        )
    })?;
    let refresh_token = token_field(tokens, "refresh_token").unwrap_or_default();
    let id_token = token_field(tokens, "id_token");
    let claims = id_token
        .as_deref()
        .and_then(decode_chatgpt_claims)
        .unwrap_or_default();
    let account_id = token_field(tokens, "account_id").or(claims.account_id.clone());

    Ok(CodexAuthSnapshot {
        path,
        document,
        auth_mode,
        access_token,
        refresh_token,
        id_token,
        email: claims.email,
        account_id,
        plan_type: claims.plan_type,
        is_fedramp_account: claims.is_fedramp_account,
    })
}

pub(super) async fn refresh_codex_auth(
    http: &Client,
    mut auth: CodexAuthSnapshot,
) -> Result<CodexAuthSnapshot, String> {
    if auth.refresh_token.trim().is_empty() {
        return Err(format!(
            "Codex access token expired and {} does not contain a refresh token. Run Codex login with ChatGPT again.",
            auth.path.display()
        ));
    }

    let endpoint = std::env::var("CODEX_REFRESH_TOKEN_URL_OVERRIDE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| CODEX_REFRESH_TOKEN_URL.to_string());
    let body = json!({
        "client_id": CODEX_OAUTH_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": auth.refresh_token,
    });
    let originator = codex_originator();
    let client_version = codex_client_version();
    let user_agent = codex_user_agent(&originator, &client_version);

    let response = http
        .post(endpoint)
        .header("originator", originator)
        .header(USER_AGENT, user_agent)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Codex token refresh request failed: {error}"))?;
    let status = response.status();
    let response_body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Codex token refresh response: {error}"))?;

    if !status.is_success() {
        return Err(format!(
            "Codex token refresh failed with status {}: {}",
            status, response_body
        ));
    }

    let parsed: Value = serde_json::from_str(&response_body)
        .map_err(|error| format!("Failed to parse Codex token refresh response: {error}"))?;
    let access_token = parsed
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            "Codex token refresh response did not include an access token.".to_string()
        })?;
    let refresh_token = parsed
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| auth.refresh_token.clone());
    let id_token = parsed
        .get("id_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| auth.id_token.clone());
    let claims = id_token
        .as_deref()
        .and_then(decode_chatgpt_claims)
        .unwrap_or_default();
    let account_id = claims
        .account_id
        .clone()
        .or_else(|| auth.account_id.clone());

    {
        let document = auth
            .document
            .as_object_mut()
            .ok_or_else(|| "Codex auth file must contain a JSON object.".to_string())?;
        let tokens = document
            .entry("tokens".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let tokens = tokens
            .as_object_mut()
            .ok_or_else(|| "Codex auth tokens must be a JSON object.".to_string())?;
        tokens.insert("access_token".into(), Value::String(access_token.clone()));
        tokens.insert("refresh_token".into(), Value::String(refresh_token.clone()));
        if let Some(id_token) = id_token.as_deref() {
            tokens.insert("id_token".into(), Value::String(id_token.to_string()));
        }
        if let Some(account_id) = account_id.as_deref() {
            tokens.insert("account_id".into(), Value::String(account_id.to_string()));
        }
    }

    persist_codex_auth(&auth.path, &auth.document)?;
    auth.access_token = access_token;
    auth.refresh_token = refresh_token;
    auth.id_token = id_token;
    auth.email = claims.email;
    auth.account_id = account_id;
    auth.plan_type = claims.plan_type;
    auth.is_fedramp_account = claims.is_fedramp_account;
    Ok(auth)
}

pub(super) fn codex_auth_path(codex_home: Option<&str>) -> Result<PathBuf, String> {
    Ok(codex_home_path(codex_home)?.join(CODEX_AUTH_FILE))
}

pub(super) fn codex_home_path(codex_home: Option<&str>) -> Result<PathBuf, String> {
    if let Some(value) = codex_home.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(expand_home_path(value));
    }

    let codex_home = if let Ok(value) = std::env::var("CODEX_HOME") {
        let value = value.trim();
        if !value.is_empty() {
            expand_home_path(value)
        } else {
            default_codex_home()?
        }
    } else {
        default_codex_home()?
    };
    Ok(codex_home)
}

pub(super) fn default_codex_home() -> Result<PathBuf, String> {
    home_dir().map(|home| home.join(".codex"))
}

pub(super) fn home_dir() -> Result<PathBuf, String> {
    crate::util::home_dir()
        .ok_or_else(|| "Failed to resolve the user home directory for Codex auth.".into())
}

pub(super) fn persist_codex_auth(path: &PathBuf, document: &Value) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(document)
        .map_err(|error| format!("Failed to serialize Codex auth file: {error}"))?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("Failed to open Codex auth file {}: {error}", path.display()))?;
    file.write_all(serialized.as_bytes()).map_err(|error| {
        format!(
            "Failed to write Codex auth file {}: {error}",
            path.display()
        )
    })?;
    file.write_all(b"\n").map_err(|error| {
        format!(
            "Failed to write Codex auth file {}: {error}",
            path.display()
        )
    })?;
    file.flush().map_err(|error| {
        format!(
            "Failed to flush Codex auth file {}: {error}",
            path.display()
        )
    })
}

pub(super) fn token_field(tokens: &Map<String, Value>, key: &str) -> Option<String> {
    tokens
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn decode_chatgpt_claims(jwt: &str) -> Option<CodexJwtClaims> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    let email = value
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("https://api.openai.com/profile")
                .and_then(|profile| profile.get("email"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned);
    let chatgpt_auth = value.get("https://api.openai.com/auth")?;
    let account_id = chatgpt_auth
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let plan_type = chatgpt_auth
        .get("chatgpt_plan_type")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let is_fedramp_account = chatgpt_auth
        .get("chatgpt_account_is_fedramp")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(CodexJwtClaims {
        email,
        account_id,
        plan_type,
        is_fedramp_account,
    })
}

pub(super) fn expand_home_path(value: &str) -> PathBuf {
    if value == "~" {
        return home_dir().unwrap_or_else(|_| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

pub(super) fn resolve_codex_binary() -> Option<PathBuf> {
    for env_name in ["DAILY_CODEX_BIN", "CODEX_BIN"] {
        if let Ok(value) = std::env::var(env_name) {
            let value = value.trim();
            if !value.is_empty() {
                let path = expand_home_path(value);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    let mut candidates = vec![PathBuf::from("codex")];
    if let Ok(home) = home_dir() {
        candidates.push(home.join(".cargo/bin/codex"));
        candidates.push(home.join(".local/bin/codex"));
        candidates.push(home.join(".npm-global/bin/codex"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
    candidates.push(PathBuf::from("/usr/local/bin/codex"));

    for candidate in candidates {
        if candidate.components().count() > 1 && !candidate.is_file() {
            continue;
        }
        if Command::new(&candidate)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }

    None
}
