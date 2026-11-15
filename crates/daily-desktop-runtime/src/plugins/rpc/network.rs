use crate::agent::AgentStore;
use crate::limits::{
    MAX_NETWORK_REDIRECTS, MAX_PLUGIN_HEADER_VALUE_BYTES, MAX_PLUGIN_URL_BYTES,
    NETWORK_FETCH_RESPONSE_LIMIT_BYTES,
};
use crate::net::describe_error_chain;
use crate::net::ssrf::validate_plugin_network_url;
use futures_util::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_LENGTH, COOKIE, HOST,
    LOCATION, PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use reqwest::Url;
use serde_json::{json, Value};
use std::time::Duration;

pub(crate) async fn plugin_network_fetch(
    state: &AgentStore,
    payload: Value,
) -> Result<Value, String> {
    let http = state.plugin_http.clone();
    let raw_url = payload
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "network.fetch requires payload.url.".to_string())?;
    if raw_url.len() > MAX_PLUGIN_URL_BYTES {
        return Err(format!(
            "network.fetch URLs must be at most {MAX_PLUGIN_URL_BYTES} bytes."
        ));
    }
    let url = Url::parse(raw_url).map_err(|error| format!("Invalid network.fetch URL: {error}"))?;
    validate_plugin_network_url(&url)?;

    let timeout_ms = payload
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(20_000)
        .clamp(1_000, 30_000);
    let mut request_headers = HeaderMap::new();
    request_headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/rss+xml, application/atom+xml, application/json, text/xml, text/plain, text/html;q=0.8"),
    );
    request_headers.insert(
        reqwest::header::USER_AGENT,
        HeaderValue::from_static("Daily-Plugin-Host/0.1"),
    );

    if let Some(headers) = payload.get("headers").and_then(Value::as_object) {
        if headers.len() > 32 {
            return Err("network.fetch supports at most 32 request headers.".into());
        }
        for (name, value) in headers {
            let value = value
                .as_str()
                .ok_or_else(|| format!("network.fetch header {name} must be a string."))?;
            if value.len() > MAX_PLUGIN_HEADER_VALUE_BYTES {
                return Err(format!(
                    "network.fetch header {name} exceeds the {MAX_PLUGIN_HEADER_VALUE_BYTES} byte limit."
                ));
            }
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| format!("Invalid network.fetch header name: {error}"))?;
            if matches!(
                name,
                HOST | CONNECTION | CONTENT_LENGTH | TRANSFER_ENCODING | TE | TRAILER | UPGRADE
            ) {
                return Err(format!(
                    "network.fetch does not allow the {name} request header."
                ));
            }
            let value = HeaderValue::from_str(value)
                .map_err(|error| format!("Invalid network.fetch header value: {error}"))?;
            request_headers.insert(name, value);
        }
    }

    let mut current_url = url;
    let mut redirect_count = 0;
    let response = loop {
        validate_plugin_network_url(&current_url)?;
        let response = http
            .get(current_url.clone())
            .timeout(Duration::from_millis(timeout_ms))
            .headers(request_headers.clone())
            .send()
            .await
            .map_err(|error| {
                format!(
                    "network.fetch request failed: {}",
                    describe_error_chain(&error)
                )
            })?;

        if !response.status().is_redirection() {
            break response;
        }
        if redirect_count >= MAX_NETWORK_REDIRECTS {
            return Err(format!(
                "network.fetch exceeded the {MAX_NETWORK_REDIRECTS} redirect limit."
            ));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                "network.fetch redirect did not include a valid Location header.".to_string()
            })?;
        let next_url = current_url
            .join(location)
            .map_err(|error| format!("Invalid network.fetch redirect URL: {error}"))?;
        validate_plugin_network_url(&next_url)?;
        if current_url.origin() != next_url.origin() {
            request_headers.remove(AUTHORIZATION);
            request_headers.remove(COOKIE);
            request_headers.remove(PROXY_AUTHORIZATION);
        }
        current_url = next_url;
        redirect_count += 1;
    };
    if response
        .content_length()
        .is_some_and(|length| length > NETWORK_FETCH_RESPONSE_LIMIT_BYTES as u64)
    {
        return Err(format!(
            "network.fetch response exceeds the {} byte limit.",
            NETWORK_FETCH_RESPONSE_LIMIT_BYTES
        ));
    }
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            format!(
                "network.fetch response failed: {}",
                describe_error_chain(&error)
            )
        })?;
        if body.len() + chunk.len() > NETWORK_FETCH_RESPONSE_LIMIT_BYTES {
            return Err(format!(
                "network.fetch response exceeds the {} byte limit.",
                NETWORK_FETCH_RESPONSE_LIMIT_BYTES
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(json!({
        "ok": (200..300).contains(&status),
        "status": status,
        "url": final_url,
        "contentType": content_type,
        "text": String::from_utf8_lossy(&body),
    }))
}
