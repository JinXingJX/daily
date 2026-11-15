//! Native GPT Image 2.6 generation for plugins. The API key stays in the host: plugins send a
//! prompt and paths inside their own file sandbox, and receive the paths of the saved images.
//! Paid requests are never retried automatically.

use crate::agent::AgentStore;
use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::resolve_plugin_file_path;
use crate::limits::{
    MAX_IMAGE_INPUT_BYTES, MAX_IMAGE_REFERENCES, MAX_IMAGE_RESPONSE_BYTES, MAX_PLUGIN_PROMPT_BYTES,
};
use crate::model::types::DEFAULT_OPENAI_URL;
use crate::model::{ModelConfig, ModelInlineImage, ModelProviderKind};
use crate::net::describe_error_chain;
use crate::net::http::build_image_http_client;
use crate::util::unique_stamp;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MODELS: [&str; 2] = ["gpt-image-2.6-sunburst", "gpt-image-2.6-flare"];
const FORMATS: [&str; 5] = ["png", "jpeg", "webp", "svg", "gif"];
const MAX_PROMPT_CHARS: usize = 32_000;
const MAX_DATA_URL_BYTES: usize = 20 * 1024 * 1024;
const MAX_CREDENTIAL_FILE_BYTES: u64 = 64 * 1024;
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
const MAX_TIMEOUT_SECONDS: u64 = 1_800;
const SETTINGS_DIR_NAME: &str = "gpt-image-2-6";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ImageGenerateInput {
    prompt: String,
    model: Option<String>,
    format: Option<String>,
    size: Option<String>,
    quality: Option<String>,
    complexity: Option<String>,
    background: Option<String>,
    moderation: Option<String>,
    n: Option<u64>,
    compression: Option<u64>,
    frames: Option<u64>,
    frame_duration_ms: Option<u64>,
    /// Plugin-relative PNG, JPEG or WebP files; the first is the primary image.
    references: Vec<String>,
    mask: Option<String>,
    markup: Option<String>,
    /// Plugin-relative directory the generated files are saved in.
    output_dir: String,
    timeout_seconds: Option<u64>,
}

#[derive(Debug)]
struct PreparedRequest {
    endpoint: &'static str,
    body: Value,
    format: String,
    count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedKey {
    key: String,
    source: &'static str,
}

/// Reports whether a key is available, without revealing it.
pub(crate) fn plugin_image_status(state: &AgentStore) -> Result<Value, String> {
    let config = current_model_config(state)?;
    let resolved = resolve_api_key(
        &config,
        std::env::var("OPENAI_API_KEY").ok(),
        settings_dir(),
    )?;
    Ok(json!({
        "configured": resolved.is_some(),
        "source": resolved.map(|key| key.source),
        "models": MODELS,
    }))
}

pub(crate) async fn plugin_image_generate(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: ImageGenerateInput = serde_json::from_value(payload)
        .map_err(|error| format!("Invalid image.generate payload: {error}"))?;
    let config = current_model_config(state)?;
    let directory = settings_dir();
    let resolved = resolve_api_key(
        &config,
        std::env::var("OPENAI_API_KEY").ok(),
        directory.clone(),
    )?
    .ok_or_else(|| {
        "OPENAI_API_KEY is not configured. Add an OpenAI provider in Daily settings, set the environment variable, or create the GPT Image credentials file.".to_string()
    })?;
    let proxy = directory
        .as_deref()
        .map(selected_proxy)
        .transpose()?
        .flatten();
    let http = build_image_http_client(proxy.as_deref())?;
    generate_images(
        &http,
        DEFAULT_OPENAI_URL,
        &resolved.key,
        state.project_root.as_path(),
        plugin_id,
        input,
    )
    .await
}

fn current_model_config(state: &AgentStore) -> Result<ModelConfig, String> {
    let app_state = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?;
    Ok(app_state.model_config.clone())
}

async fn generate_images(
    http: &Client,
    api_base: &str,
    key: &str,
    project_root: &Path,
    plugin_id: &str,
    input: ImageGenerateInput,
) -> Result<Value, String> {
    if input.output_dir.trim().is_empty() {
        return Err("image.generate requires payload.outputDir.".into());
    }
    let timeout = input
        .timeout_seconds
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let output_dir = input.output_dir.trim().to_string();
    let prepared = prepare_request(&input, project_root, plugin_id)?;
    // Check the destination before spending money on the request.
    resolve_plugin_file_path(project_root, plugin_id, &output_dir)?;

    let url = Url::parse(&format!(
        "{}{}",
        api_base.trim_end_matches('/'),
        prepared.endpoint
    ))
    .map_err(|error| format!("Invalid image API URL: {error}"))?;
    let response = http
        .post(url)
        .bearer_auth(key)
        .timeout(Duration::from_secs(timeout))
        .json(&prepared.body)
        .send()
        .await
        .map_err(|error| {
            safe_message(
                &format!(
                    "Image request failed: {}. The server may have processed it; no automatic retry was made.",
                    describe_error_chain(&error)
                ),
                key,
            )
        })?;
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let body = read_limited_body(response, key).await?;
    if !status.is_success() {
        return Err(api_error(status.as_u16(), &body, key));
    }
    let result: Value = serde_json::from_slice(&body).map_err(|error| {
        safe_message(
            &format!("Image API response could not be read: {error}; no automatic retry was made."),
            key,
        )
    })?;
    let items = result
        .get("data")
        .and_then(Value::as_array)
        .filter(|items| items.len() == prepared.count)
        .ok_or_else(|| "Image API returned an unexpected image count.".to_string())?;

    // Decode everything first so a malformed item leaves no partial output behind.
    let decoded = items
        .iter()
        .map(|item| decode_image(item.get("b64_json")))
        .collect::<Result<Vec<_>, _>>()?;
    let extension = extension_for(&prepared.format);
    let stamp = unique_stamp();
    let mut files = Vec::with_capacity(decoded.len());
    for (index, bytes) in decoded.iter().enumerate() {
        let relative = format!(
            "{}/image-{stamp}-{}.{extension}",
            output_dir.trim_end_matches('/'),
            index + 1
        );
        let path = resolve_plugin_file_path(project_root, plugin_id, &relative)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create image directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        atomic_write(&path, bytes)
            .map_err(|error| format!("Failed to save image {}: {error}", path.display()))?;
        files.push(json!({
            "path": relative,
            "bytes": bytes.len(),
            "mime": mime_for(&prepared.format),
        }));
    }

    Ok(json!({
        "files": files,
        "model": prepared.body.get("model"),
        "endpoint": prepared.endpoint,
        "requestId": request_id,
        "outputFormat": result.get("output_format").and_then(Value::as_str).unwrap_or(&prepared.format),
        "size": result.get("size").or_else(|| prepared.body.get("size")),
        "usage": result.get("usage").cloned().unwrap_or(Value::Null),
    }))
}

async fn read_limited_body(response: reqwest::Response, key: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            safe_message(
                &format!(
                    "Image response was interrupted: {}. The server may have processed it; no automatic retry was made.",
                    describe_error_chain(&error)
                ),
                key,
            )
        })?;
        if body.len() + chunk.len() > MAX_IMAGE_RESPONSE_BYTES {
            return Err(format!(
                "Image API response exceeds the {MAX_IMAGE_RESPONSE_BYTES} byte limit."
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn prepare_request(
    input: &ImageGenerateInput,
    project_root: &Path,
    plugin_id: &str,
) -> Result<PreparedRequest, String> {
    let prompt = input.prompt.trim();
    if prompt.is_empty()
        || prompt.chars().count() > MAX_PROMPT_CHARS
        || prompt.len() > MAX_PLUGIN_PROMPT_BYTES
    {
        return Err(format!(
            "Provide a nonempty prompt of at most {MAX_PROMPT_CHARS} characters."
        ));
    }
    let model = match input.model.as_deref().unwrap_or("sunburst") {
        "sunburst" => MODELS[0],
        "flare" => MODELS[1],
        other => MODELS
            .iter()
            .copied()
            .find(|model| *model == other)
            .ok_or_else(|| format!("model must be one of: {}", MODELS.join(", ")))?,
    };
    let format = choice("format", input.format.as_deref().unwrap_or("png"), &FORMATS)?;
    let count = integer("n", input.n.unwrap_or(1), 1, 10)?;
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("prompt".into(), json!(prompt));
    body.insert("output_format".into(), json!(format));
    body.insert("n".into(), json!(count));
    for (name, value, allowed) in [
        (
            "quality",
            &input.quality,
            &["low", "medium", "high", "xhigh", "max", "auto"][..],
        ),
        (
            "complexity",
            &input.complexity,
            &["low", "medium", "high"][..],
        ),
        (
            "background",
            &input.background,
            &["transparent", "opaque", "auto"][..],
        ),
        ("moderation", &input.moderation, &["auto", "low"][..]),
    ] {
        if let Some(value) = value {
            body.insert(name.into(), json!(choice(name, value, allowed)?));
        }
    }

    let size = input.size.as_deref().unwrap_or("auto");
    validate_size(size, format)?;
    body.insert("size".into(), json!(size));

    if input.background.as_deref() == Some("transparent") && !matches!(format, "png" | "webp") {
        return Err("Transparent backgrounds require PNG or WebP.".into());
    }
    if let Some(compression) = input.compression {
        if !matches!(format, "jpeg" | "webp") {
            return Err("compression is available only for JPEG or WebP.".into());
        }
        body.insert(
            "output_compression".into(),
            json!(integer("compression", compression, 0, 100)?),
        );
    }
    if format == "svg" && count != 1 {
        return Err("SVG output requires n = 1.".into());
    }
    if format == "gif" {
        if let Some(frames) = input.frames {
            body.insert(
                "frame_count".into(),
                json!(integer("frames", frames, 2, 16)?),
            );
        }
        if let Some(duration) = input.frame_duration_ms {
            body.insert(
                "frame_duration_ms".into(),
                json!(integer("frameDurationMs", duration, 20, 2_000)?),
            );
        }
    } else if input.frames.is_some() || input.frame_duration_ms.is_some() {
        return Err("Animation options require GIF output.".into());
    }

    let editing = !input.references.is_empty();
    let endpoint = if editing {
        "/images/edits"
    } else {
        "/images/generations"
    };
    if editing {
        if matches!(format, "svg" | "gif") {
            return Err("SVG and GIF output are generation-only.".into());
        }
        if input.references.len() > MAX_IMAGE_REFERENCES {
            return Err(format!(
                "Editing accepts at most {MAX_IMAGE_REFERENCES} reference images."
            ));
        }
        if input.mask.is_some() && input.markup.is_some() {
            return Err("mask and markup cannot be combined.".into());
        }
        let references = input
            .references
            .iter()
            .map(|path| load_input_image(project_root, plugin_id, path, false))
            .collect::<Result<Vec<_>, _>>()?;
        for (field, path) in [("mask", &input.mask), ("markup", &input.markup)] {
            if let Some(path) = path {
                let overlay = load_input_image(project_root, plugin_id, path, true)?;
                if references[0].dimensions.is_some()
                    && overlay.dimensions.is_some()
                    && references[0].dimensions != overlay.dimensions
                {
                    return Err(format!(
                        "The {field} dimensions must match the first reference image."
                    ));
                }
                body.insert(field.into(), json!({ "image_url": overlay.data_url }));
            }
        }
        body.insert(
            "images".into(),
            json!(references
                .iter()
                .map(|image| json!({ "image_url": image.data_url }))
                .collect::<Vec<_>>()),
        );
    } else if input.mask.is_some() || input.markup.is_some() {
        return Err("mask and markup require reference images.".into());
    }

    Ok(PreparedRequest {
        endpoint,
        body: Value::Object(body),
        format: format.to_string(),
        count: count as usize,
    })
}

struct InputImage {
    data_url: String,
    dimensions: Option<(u32, u32)>,
}

fn load_input_image(
    project_root: &Path,
    plugin_id: &str,
    relative: &str,
    png_only: bool,
) -> Result<InputImage, String> {
    let path = resolve_plugin_file_path(project_root, plugin_id, relative)?;
    let mime = input_mime(&path).ok_or_else(|| {
        "Reference images must be PNG, JPEG or WebP files; SVG and GIF cannot be edited."
            .to_string()
    })?;
    if png_only && mime != "image/png" {
        return Err("Masks and markup must be transparent PNG files.".into());
    }
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("Failed to read reference image {relative}: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_INPUT_BYTES {
        return Err(format!(
            "Reference image {relative} is not a file or exceeds {MAX_IMAGE_INPUT_BYTES} bytes."
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("Failed to read reference image {relative}: {error}"))?;
    let dimensions = if mime == "image/png" {
        Some(png_dimensions(&bytes)?)
    } else {
        None
    };
    if png_only && !png_has_alpha(&bytes) {
        return Err("The PNG mask or markup has no alpha channel or transparency palette.".into());
    }
    let data_url = format!("data:{mime};base64,{}", BASE64.encode(&bytes));
    if data_url.len() > MAX_DATA_URL_BYTES {
        return Err(format!(
            "Reference image {relative} exceeds the API data URL limit."
        ));
    }
    Ok(InputImage {
        data_url,
        dimensions,
    })
}

/// A plugin-private image in the form model requests carry it.
pub(crate) fn load_model_image(
    project_root: &Path,
    plugin_id: &str,
    relative: &str,
) -> Result<ModelInlineImage, String> {
    let image = load_input_image(project_root, plugin_id, relative, false)?;
    let (head, data) = image
        .data_url
        .split_once(";base64,")
        .ok_or_else(|| "Reference image could not be encoded.".to_string())?;
    Ok(ModelInlineImage {
        name: None,
        mime_type: head.trim_start_matches("data:").to_string(),
        data_base64: data.to_string(),
    })
}

fn input_mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 33 || bytes[..8] != SIGNATURE {
        return Err("Input is not a valid PNG file.".into());
    }
    let word = |offset: usize| {
        u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    Ok((word(16), word(20)))
}

fn png_has_alpha(bytes: &[u8]) -> bool {
    bytes.len() > 25
        && (matches!(bytes[25], 4 | 6) || bytes.windows(4).any(|window| window == b"tRNS"))
}

fn choice<'a>(name: &str, value: &'a str, allowed: &[&str]) -> Result<&'a str, String> {
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(format!("{name} must be one of: {}", allowed.join(", ")))
    }
}

fn integer(name: &str, value: u64, min: u64, max: u64) -> Result<u64, String> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{name} must be between {min} and {max}."))
    }
}

fn validate_size(size: &str, format: &str) -> Result<(), String> {
    if size == "auto" {
        return Ok(());
    }
    let (width, height) = size
        .split_once('x')
        .and_then(|(width, height)| Some((width.parse::<u64>().ok()?, height.parse::<u64>().ok()?)))
        .ok_or_else(|| "size must be auto or WIDTHxHEIGHT.".to_string())?;
    if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
        return Err("Size dimensions must be positive multiples of 16.".into());
    }
    let (long, short) = (width.max(height), width.min(height));
    if long > 3840 || long > short * 3 {
        return Err("Size exceeds the edge or aspect-ratio limit.".into());
    }
    let pixels = width * height;
    if format != "gif" && !(655_360..=8_294_400).contains(&pixels) {
        return Err("Static image size must contain 655,360 to 8,294,400 pixels.".into());
    }
    Ok(())
}

fn decode_image(value: Option<&Value>) -> Result<Vec<u8>, String> {
    let encoded = value
        .and_then(Value::as_str)
        .filter(|encoded| !encoded.is_empty())
        .ok_or_else(|| "Image API returned missing image data.".to_string())?;
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| "Image API returned invalid base64 image data.".to_string())?;
    if bytes.is_empty() {
        return Err("Image API returned an empty image.".into());
    }
    Ok(bytes)
}

fn extension_for(format: &str) -> &'static str {
    match format {
        "jpeg" => "jpg",
        "webp" => "webp",
        "svg" => "svg",
        "gif" => "gif",
        _ => "png",
    }
}

fn mime_for(format: &str) -> &'static str {
    match format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn api_error(status: u16, body: &[u8], key: &str) -> String {
    let data: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let error = data.get("error");
    let code = error
        .and_then(|error| error.get("code").or_else(|| error.get("type")))
        .and_then(Value::as_str)
        .unwrap_or("api_error");
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("The image request was rejected");
    let hint = match (status, code) {
        (401, _) => "Check the configured API key.",
        (_, "insufficient_quota") => "Add API credits or adjust the project billing limit.",
        (403 | 404, _) => "Check model and feature-preview access for this project.",
        _ => "",
    };
    safe_message(
        format!("OpenAI HTTP {status} ({code}): {message}. {hint}").trim_end(),
        key,
    )
}

/// Removes the active key and anything shaped like an OpenAI key from text that leaves the host.
fn safe_message(message: &str, key: &str) -> String {
    let mut result = if key.is_empty() {
        message.to_string()
    } else {
        message.replace(key, "[REDACTED]")
    };
    let mut cleaned = String::with_capacity(result.len());
    while let Some(start) = result.find("sk-") {
        cleaned.push_str(&result[..start]);
        cleaned.push_str("[REDACTED]");
        let rest = &result[start + 3..];
        let end = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .unwrap_or(rest.len());
        result = rest[end..].to_string();
    }
    cleaned.push_str(&result);
    cleaned
}

/// Key order: an OpenAI provider configured in Daily, `OPENAI_API_KEY`, then the GPT Image
/// credentials file. Relay providers are skipped so the key only ever reaches api.openai.com.
fn resolve_api_key(
    config: &ModelConfig,
    env_key: Option<String>,
    directory: Option<PathBuf>,
) -> Result<Option<ResolvedKey>, String> {
    let official = config.auth_configs.iter().find_map(|auth| {
        let auth = auth.clone().normalized();
        let is_openai = auth.provider_kind == ModelProviderKind::OpenAiCompatible
            && auth.base_url.trim_end_matches('/') == DEFAULT_OPENAI_URL;
        let key = auth.resolved_api_key();
        (is_openai && !key.is_empty()).then_some(key)
    });
    if let Some(key) = official {
        return Ok(Some(ResolvedKey {
            key,
            source: "daily_provider",
        }));
    }
    if let Some(key) = env_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(ResolvedKey {
            key,
            source: "environment",
        }));
    }
    let Some(directory) = directory else {
        return Ok(None);
    };
    Ok(
        read_credential_file(&directory.join("credentials.env"))?.map(|key| ResolvedKey {
            key,
            source: "credentials_file",
        }),
    )
}

fn read_credential_file(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Cannot inspect the GPT Image credential file: {error}"
            ))
        }
    };
    if !metadata.is_file() || metadata.len() > MAX_CREDENTIAL_FILE_BYTES {
        return Err("The GPT Image credential file must be a small regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "The GPT Image credential file permissions are too broad; use chmod 600.".into(),
            );
        }
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Cannot read the GPT Image credential file: {error}"))?;
    Ok(content.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let value = line.strip_prefix("OPENAI_API_KEY=")?.trim();
        let value = value.trim_matches(|ch| ch == '"' || ch == '\'').trim();
        (!value.is_empty()).then(|| value.to_string())
    }))
}

fn settings_dir() -> Option<PathBuf> {
    settings_dir_from(&|name: &str| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

fn settings_dir_from(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(directory) = env("IMAGE26_CONFIG_DIR") {
        return Some(PathBuf::from(directory));
    }
    if let Some(base) = env("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join(SETTINGS_DIR_NAME));
    }
    if cfg!(windows) {
        return env("APPDATA").map(|base| PathBuf::from(base).join(SETTINGS_DIR_NAME));
    }
    env("HOME").map(|home| PathBuf::from(home).join(".config").join(SETTINGS_DIR_NAME))
}

/// The proxy opted into through the GPT Image settings file. Without one, requests follow Daily's
/// normal network behavior, which honors the system proxy.
fn selected_proxy(directory: &Path) -> Result<Option<String>, String> {
    let path = directory.join("config.json");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read the GPT Image settings file: {error}")),
    };
    let config: Value = serde_json::from_str(&content)
        .map_err(|_| "The GPT Image settings file must be valid JSON.".to_string())?;
    if config.get("proxyEnabled").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let Some(proxy) = config.get("proxy").and_then(Value::as_str) else {
        return Ok(None);
    };
    let url = Url::parse(proxy).map_err(|_| "The image proxy must be a valid URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Daily image requests support HTTP and HTTPS proxies only.".into());
    }
    Ok(Some(proxy.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelAuthConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("daily-image-{name}-{}", unique_stamp()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn input(prompt: &str) -> ImageGenerateInput {
        ImageGenerateInput {
            prompt: prompt.into(),
            output_dir: "images".into(),
            ..Default::default()
        }
    }

    fn provider(base_url: &str, key: &str) -> ModelAuthConfig {
        serde_json::from_value(json!({
            "id": base_url,
            "name": "OpenAI",
            "providerKind": "open_ai_compatible",
            "baseUrl": base_url,
            "codexHome": null,
            "models": ["gpt"],
            "defaultModel": "gpt",
            "apiKey": key,
        }))
        .unwrap()
    }

    fn prepare(input: &ImageGenerateInput) -> Result<PreparedRequest, String> {
        prepare_request(input, Path::new("/tmp/daily-unused"), "test.plugin")
    }

    /// Answers one HTTP request, returning the raw request text it received.
    async fn serve_once(
        status: &'static str,
        body: String,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                received.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&received);
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if received.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\nx-request-id: req_1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&received).to_string()
        });
        (format!("http://{address}/v1"), handle)
    }

    #[test]
    fn request_matches_the_documented_shape() {
        let mut request = input("a tiny fox");
        request.model = Some("flare".into());
        request.format = Some("webp".into());
        request.size = Some("1536x1024".into());
        request.quality = Some("high".into());
        request.compression = Some(88);
        let prepared = prepare(&request).unwrap();
        assert_eq!(prepared.endpoint, "/images/generations");
        assert_eq!(prepared.body["model"], "gpt-image-2.6-flare");
        assert_eq!(prepared.body["output_format"], "webp");
        assert_eq!(prepared.body["output_compression"], 88);
        assert_eq!(prepared.body["size"], "1536x1024");
        assert_eq!(prepared.count, 1);
    }

    #[test]
    fn invalid_options_are_rejected_before_any_request() {
        for mutate in [
            (|r: &mut ImageGenerateInput| r.model = Some("gpt-image-1".into()))
                as fn(&mut ImageGenerateInput),
            |r| r.prompt = "  ".into(),
            |r| r.prompt = "x".repeat(32_001),
            |r| r.n = Some(11),
            |r| r.quality = Some("ultra".into()),
            |r| r.size = Some("1000x1000".into()),
            |r| r.size = Some("3200x800".into()),
            |r| r.size = Some("512x512".into()),
            |r| r.compression = Some(50),
            |r| r.frames = Some(4),
            |r| {
                r.format = Some("jpeg".into());
                r.background = Some("transparent".into());
            },
            |r| {
                r.format = Some("svg".into());
                r.n = Some(2);
            },
            |r| {
                r.format = Some("gif".into());
                r.frames = Some(1);
            },
            |r| r.mask = Some("mask.png".into()),
        ] {
            let mut request = input("fox");
            mutate(&mut request);
            assert!(prepare(&request).is_err(), "{request:?} should be rejected");
        }
    }

    #[test]
    fn reference_images_use_the_edit_endpoint_inside_the_sandbox() {
        let root = temp_root("refs");
        let dir = root.join(".daily/plugin-files/test.plugin/images");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.webp"), b"webp-bytes").unwrap();
        let mut request = input("change the scarf");
        request.references = vec!["images/a.webp".into()];
        let prepared = prepare_request(&request, &root, "test.plugin").unwrap();
        assert_eq!(prepared.endpoint, "/images/edits");
        assert!(prepared.body["images"][0]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/webp;base64,"));

        request.references = vec!["../../outside.png".into()];
        assert!(prepare_request(&request, &root, "test.plugin").is_err());
        fs::write(dir.join("b.gif"), b"gif").unwrap();
        request.references = vec!["images/b.gif".into()];
        assert!(prepare_request(&request, &root, "test.plugin").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn masks_must_carry_transparency() {
        let root = temp_root("mask");
        let dir = root.join(".daily/plugin-files/test.plugin");
        fs::create_dir_all(&dir).unwrap();
        let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&[0, 0, 0, 16, 0, 0, 0, 16, 8, 2, 0, 0, 0, 0, 0, 0, 0]);
        fs::write(dir.join("a.png"), &png).unwrap();
        fs::write(dir.join("opaque.png"), &png).unwrap();
        let mut request = input("x");
        request.references = vec!["a.png".into()];
        request.mask = Some("opaque.png".into());
        let error = prepare_request(&request, &root, "test.plugin").unwrap_err();
        assert!(error.contains("alpha"), "{error}");
        png[25] = 6;
        fs::write(dir.join("alpha.png"), &png).unwrap();
        request.mask = Some("alpha.png".into());
        assert!(prepare_request(&request, &root, "test.plugin").is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn generated_files_are_saved_in_the_plugin_sandbox() {
        let root = temp_root("save");
        let encoded = BASE64.encode(b"image-bytes");
        let (base, server) = serve_once(
            "200 OK",
            json!({ "data": [{ "b64_json": encoded }], "output_format": "webp", "size": "1536x1024", "usage": { "total_tokens": 7 } }).to_string(),
        )
        .await;
        let mut request = input("fox");
        request.format = Some("webp".into());
        let result = generate_images(
            &Client::new(),
            &base,
            "sk-secret",
            &root,
            "test.plugin",
            request,
        )
        .await
        .unwrap();
        let received = server.await.unwrap();
        assert!(received.starts_with("POST /v1/images/generations"));
        assert!(received
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-secret"));
        let file = &result["files"][0];
        let path = file["path"].as_str().unwrap();
        assert!(path.starts_with("images/image-") && path.ends_with("-1.webp"));
        assert_eq!(result["requestId"], "req_1");
        assert_eq!(file["bytes"], 11);
        assert!(!result.to_string().contains("sk-secret"));
        assert_eq!(
            fs::read(root.join(".daily/plugin-files/test.plugin").join(path)).unwrap(),
            b"image-bytes"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn api_failures_report_the_cause_without_the_key() {
        let root = temp_root("error");
        let (base, server) = serve_once(
            "429 Too Many Requests",
            json!({ "error": { "code": "insufficient_quota", "message": "bad key sk-secret leaked" } }).to_string(),
        )
        .await;
        let error = generate_images(
            &Client::new(),
            &base,
            "sk-secret",
            &root,
            "test.plugin",
            input("fox"),
        )
        .await
        .unwrap_err();
        server.await.unwrap();
        assert!(
            error.contains("OpenAI HTTP 429 (insufficient_quota)"),
            "{error}"
        );
        assert!(error.contains("Add API credits"), "{error}");
        assert!(!error.contains("sk-secret"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_short_or_invalid_response_saves_nothing() {
        for body in [
            json!({ "data": [] }).to_string(),
            json!({ "data": [{ "b64_json": "%%%" }] }).to_string(),
        ] {
            let root = temp_root("invalid");
            let (base, server) = serve_once("200 OK", body).await;
            assert!(generate_images(
                &Client::new(),
                &base,
                "k",
                &root,
                "test.plugin",
                input("fox")
            )
            .await
            .is_err());
            server.await.unwrap();
            assert!(!root.join(".daily/plugin-files/test.plugin/images").exists());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn keys_resolve_in_order_and_never_from_relay_providers() {
        let root = temp_root("key");
        let mut config = ModelConfig::seeded();
        config.auth_configs = vec![provider("https://relay.example/v1", "sk-relay")];

        assert_eq!(
            resolve_api_key(&config, None, Some(root.clone())).unwrap(),
            None
        );
        fs::write(
            root.join("credentials.env"),
            "export OPENAI_API_KEY='sk-file'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join("credentials.env"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert!(resolve_api_key(&config, None, Some(root.clone())).is_err());
            fs::set_permissions(
                root.join("credentials.env"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let file = resolve_api_key(&config, None, Some(root.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(
            (file.key.as_str(), file.source),
            ("sk-file", "credentials_file")
        );
        let env = resolve_api_key(&config, Some(" sk-env ".into()), Some(root.clone()))
            .unwrap()
            .unwrap();
        assert_eq!((env.key.as_str(), env.source), ("sk-env", "environment"));

        config
            .auth_configs
            .push(provider("https://api.openai.com/v1/", "sk-daily"));
        let daily = resolve_api_key(&config, Some("sk-env".into()), Some(root.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(
            (daily.key.as_str(), daily.source),
            ("sk-daily", "daily_provider")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn proxy_is_off_unless_the_settings_file_opts_in() {
        let root = temp_root("proxy");
        assert_eq!(selected_proxy(&root).unwrap(), None);
        fs::write(
            root.join("config.json"),
            r#"{"proxyEnabled":false,"proxy":"http://127.0.0.1:7890"}"#,
        )
        .unwrap();
        assert_eq!(selected_proxy(&root).unwrap(), None);
        fs::write(
            root.join("config.json"),
            r#"{"proxyEnabled":true,"proxy":"http://127.0.0.1:7890"}"#,
        )
        .unwrap();
        assert_eq!(
            selected_proxy(&root).unwrap().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        fs::write(
            root.join("config.json"),
            r#"{"proxyEnabled":true,"proxy":"socks5h://127.0.0.1:7891"}"#,
        )
        .unwrap();
        assert!(selected_proxy(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_directory_follows_the_documented_order() {
        let lookup = |values: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                values
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(
            settings_dir_from(&lookup(&[("IMAGE26_CONFIG_DIR", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(
            settings_dir_from(&lookup(&[("XDG_CONFIG_HOME", "/c"), ("HOME", "/h")])),
            Some(PathBuf::from("/c/gpt-image-2-6"))
        );
        if !cfg!(windows) {
            assert_eq!(
                settings_dir_from(&lookup(&[("HOME", "/h")])),
                Some(PathBuf::from("/h/.config/gpt-image-2-6"))
            );
        }
    }

    #[test]
    fn messages_are_scrubbed_of_key_shaped_text() {
        assert_eq!(
            safe_message("use sk-abc_1 now, key zz", "zz"),
            "use [REDACTED] now, key [REDACTED]"
        );
    }
}
