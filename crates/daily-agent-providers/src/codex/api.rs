//! API-key authentication for the public Responses API and compatible gateways.

use super::*;

pub struct CodexApiKeyAdapter;

#[async_trait]
impl ModelAdapter for CodexApiKeyAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::CodexApiKey
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn retries_internally(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        if config.api_key.trim().is_empty() {
            return Err(ModelError::new(
                ModelErrorKind::Auth,
                "Codex API requires an API key. Set it in Model providers or OPENAI_API_KEY.",
            ));
        }
        // Both transports use the same Responses events, tools, images and stream retry logic.
        ResponsesAdapter
            .complete(http, config, request, on_chunk)
            .await
    }
}

pub(super) fn build_api_request_body(
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    identity: &CodexRequestIdentity,
) -> Value {
    let mut body = build_codex_request_body(config, request, true, identity);
    let object = body
        .as_object_mut()
        .expect("Responses request is an object");
    // ChatGPT-only metadata is not part of the public Responses API.
    object.remove("client_metadata");
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            // Daily tool schemas can have optional properties, so do not opt into strict mode.
            tool["strict"] = json!(false);
        }
    }
    let (_, effort) = codex_model_and_reasoning(&config.model);
    if let Some(effort) = effort {
        object.insert("reasoning".into(), json!({ "effort": effort }));
    } else {
        object.remove("reasoning");
    }
    // Key-based calls use their configured output budget, including per-call overrides.
    object.insert(
        "max_output_tokens".into(),
        json!(request
            .max_output_tokens
            .unwrap_or(config.max_output_tokens)),
    );
    body
}

pub(super) async fn send_api_request(
    http: &Client,
    config: &ModelConfig,
    body: &Value,
) -> Result<Response, String> {
    let base_url = config.base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/responses") {
        base_url.to_string()
    } else {
        format!("{base_url}/responses")
    };
    http.post(endpoint)
        .bearer_auth(config.api_key.trim())
        .header(ACCEPT, "text/event-stream")
        .json(body)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Codex API request failed: {}",
                describe_reqwest_error(&error)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(
        replies: Vec<(&'static str, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut socket, _) = timeout(Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "request ended before headers");
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec())
                    .unwrap()
                    .to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "request ended before body");
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                requests.push((headers, request));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        (format!("http://{address}/v1"), task)
    }

    fn config(base_url: String) -> ModelConfig {
        let mut config = ModelConfig::seeded();
        config.provider_kind = ModelProviderKind::CodexApiKey;
        config.base_url = base_url;
        config.api_key = "test-key".into();
        config.model = "gpt-5.5-high".into();
        config.codex_home = Some("/nonexistent/daily-test-codex-home".into());
        config.max_output_tokens = 2048;
        config
    }

    fn request() -> ModelCompletionRequest {
        ModelCompletionRequest {
            messages: vec![ModelMessage::system("Be helpful"), {
                let mut user = ModelMessage::user("Look this up");
                user.images.push(ModelInlineImage {
                    name: None,
                    mime_type: "image/png".into(),
                    data_base64: "QUJD".into(),
                });
                user
            }],
            temperature: 0.2,
            max_output_tokens: Some(4096),
            tools: vec![ModelToolDefinition {
                id: "daily.lookup".into(),
                name: "Lookup".into(),
                description: "Look up data".into(),
                input_schema: json!({"type":"object","properties":{"query":{"type":"string"}}}),
            }],
            tool_choice: ModelToolChoice::Auto,
            session_id: Some(42),
            turn_id: Some(1),
            request_name: None,
        }
    }

    fn events() -> String {
        [
            json!({"type":"response.output_text.delta","delta":"Hello"}),
            json!({"type":"response.completed","response":{"output":[
                {"type":"message","content":[{"type":"output_text","text":"Hello"}]},
                {"type":"function_call","call_id":"call-1","name":"daily_lookup","arguments":"{\"query\":\"test\"}"}
            ]}}),
        ].iter().map(|event| format!("data: {event}\n\n")).collect()
    }

    #[tokio::test]
    async fn api_key_streams_text_tools_and_images_without_cli_credentials() {
        let (base_url, server) = server(vec![("200 OK", events())]).await;
        let http = Client::builder().no_proxy().build().unwrap();
        let mut chunks = Vec::new();
        let result = CodexApiKeyAdapter
            .complete(
                &http,
                &config(base_url),
                &request(),
                Some(&mut |chunk| {
                    chunks.push(chunk.text);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.raw_output, "Hello");
        assert_eq!(chunks, vec!["Hello"]);
        assert_eq!(result.tool_calls[0].tool_id, "daily.lookup");
        assert_eq!(result.tool_calls[0].call_id.as_deref(), Some("call-1"));
        assert_eq!(result.tool_calls[0].arguments, json!({"query":"test"}));
        let requests = server.await.unwrap();
        let (headers, body) = &requests[0];
        assert!(headers.starts_with("post /v1/responses http/1.1"));
        assert!(headers.contains("authorization: bearer test-key\r\n"));
        assert!(!headers.contains("chatgpt-account-id"));
        assert!(!headers.contains("originator:"));
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["reasoning"], json!({"effort":"high"}));
        assert_eq!(body["instructions"], "Be helpful");
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("client_metadata").is_none());
        assert_eq!(
            body["input"][0]["content"][1]["image_url"],
            "data:image/png;base64,QUJD"
        );
        assert_eq!(body["tools"][0]["name"], "daily_lookup");
        assert_eq!(body["tools"][0]["strict"], false);
    }

    #[tokio::test]
    async fn api_key_401_returns_the_api_error_without_refreshing_cli_tokens() {
        let (base_url, server) = server(vec![(
            "401 Unauthorized",
            "{\"error\":{\"message\":\"Invalid API key\"}}".into(),
        )])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let error = CodexApiKeyAdapter
            .complete(&http, &config(base_url), &request(), None)
            .await
            .err()
            .unwrap();
        assert!(error.message.contains("401"));
        assert!(error.message.contains("Invalid API key"));
        assert_eq!(error.kind, ModelErrorKind::Auth);
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn api_key_retries_server_errors_and_accepts_a_full_responses_url() {
        let (base_url, server) = server(vec![
            ("500 Internal Server Error", "unavailable".into()),
            ("200 OK", events()),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let result = CodexApiKeyAdapter
            .complete(
                &http,
                &config(format!("{base_url}/responses/")),
                &request(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.raw_output, "Hello");
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|(headers, _)| headers.starts_with("post /v1/responses http/1.1")));
    }

    #[tokio::test]
    async fn missing_key_does_not_fall_back_to_cli_login() {
        let mut config = config("http://127.0.0.1:1".into());
        config.api_key.clear();
        let error = CodexApiKeyAdapter
            .complete(&Client::new(), &config, &request(), None)
            .await
            .err()
            .unwrap();
        assert!(error.message.contains("requires an API key"));
    }
}
