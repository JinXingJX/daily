use super::*;

#[test]
fn tool_results_follow_the_request_they_answer() {
    let task = AgentTask {
        name: "test".into(),
        session_id: None,
        turn_id: None,
        model: ModelConfig::seeded(),
        system_prompt: "system".into(),
        user_prompt: "write scene 2".into(),
        final_instructions: "final".into(),
        context: Vec::new(),
        conversation: vec![ModelMessage {
            role: ModelMessageRole::Assistant,
            content: "earlier turn".into(),
        }],
        prior_tool_observations: Vec::new(),
        tools: Vec::new(),
        max_iterations: 4,
        start_ply: 1,
        observation_budget: None,
        max_output_tokens: 256,
        temperature: 0.2,
        inline_images: Vec::new(),
        stream: false,
        require_tool_before_final: false,
        continue_on_tool_error: true,
        sub_agents: SubAgentConfig::default(),
        resume_sub_agent: None,
    };
    let observation = AgentToolObservation::resumed(
        "writer.draftScene",
        "Draft",
        "call 1",
        json!({ "sceneIndex": 1 }),
        "drafting",
        "scene 2 written",
        json!({}),
    );

    let first = build_task_messages(&task, &[], false, 0);
    assert_eq!(
        first.last().map(|message| message.content.as_str()),
        Some("write scene 2")
    );

    let messages = build_task_messages(&task, &[observation], false, 1);
    let last = messages.last().expect("request message");
    assert!(last.role == ModelMessageRole::User);
    assert!(last.content.starts_with("write scene 2"));
    assert!(last.content.find("write scene 2") < last.content.find("scene 2 written"));
    assert!(messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .all(|message| !message.content.contains("scene 2 written")));
}

#[test]
fn resumed_async_tool_observation_formats_single_tool_message_body() {
    let observation = AgentToolObservation::resumed(
        "tool-async-1",
        "Async External Tool",
        "Needs external work.",
        json!({ "toolInput": "fetch data" }),
        "正在执行异步工具",
        "External result is ready.",
        json!({ "ok": true }),
    );

    let formatted = format_observations(&[observation]);

    assert!(formatted.contains("\"type\": \"tool_message\""));
    assert!(formatted.contains("\"initial\": \"正在执行异步工具\""));
    assert!(formatted.contains("\"result\": \"External result is ready.\""));
}

#[test]
fn dynamic_async_tool_returns_pending_execution() {
    let tool = agent_tool_from_dynamic(DynamicAgentToolDefinition {
        id: "dynamic.ask".into(),
        name: "Ask".into(),
        description: "Ask user".into(),
        input_schema: json!({ "type": "object" }),
        execution: DynamicAgentToolExecution::Async {
            pending_message: "正在执行异步工具".into(),
            metadata: json!({ "interaction": { "type": "ask_user" } }),
        },
    });

    let execution = tool
        .execute(AgentToolCall {
            tool_id: "dynamic.ask".into(),
            rationale: "Need user input".into(),
            input: json!({ "question": "A or B?" }),
        })
        .expect("dynamic tool should execute");

    match execution {
        AgentToolExecution::Pending(pending) => {
            assert_eq!(pending.pending_message, "正在执行异步工具");
            assert_eq!(pending.metadata["input"]["question"], "A or B?");
        }
        AgentToolExecution::Completed(_) => panic!("expected pending execution"),
    }
}

#[test]
fn dynamic_async_tool_deserializes_camel_case_pending_message() {
    let definition: DynamicAgentToolDefinition = serde_json::from_value(json!({
        "id": "dynamic.ask",
        "name": "Ask",
        "description": "Ask user",
        "inputSchema": { "type": "object" },
        "execution": {
            "type": "async",
            "pendingMessage": "正在等待用户回答问题",
            "metadata": { "interaction": { "type": "ask_user" } }
        }
    }))
    .expect("camelCase dynamic async tool should deserialize");

    match definition.execution {
        DynamicAgentToolExecution::Async {
            pending_message,
            metadata,
        } => {
            assert_eq!(pending_message, "正在等待用户回答问题");
            assert_eq!(metadata["interaction"]["type"], "ask_user");
        }
        DynamicAgentToolExecution::Sync { .. } => panic!("expected async execution"),
    }
}

#[test]
fn model_response_decision_preserves_multiple_tool_calls() {
    let response = crate::model::ModelCompletionResponse {
        raw_output: String::new(),
        tool_calls: vec![
            crate::model::ModelToolCall {
                call_id: Some("call-1".into()),
                tool_id: "tool-a".into(),
                arguments: json!({ "a": 1 }),
            },
            crate::model::ModelToolCall {
                call_id: Some("call-2".into()),
                tool_id: "tool-b".into(),
                arguments: json!({ "b": 2 }),
            },
        ],
    };

    match model_response_decision(&response).expect("multiple API tool calls should parse") {
        AgentDecision::Tools(calls) => {
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].tool_id, "tool-a");
            assert_eq!(calls[1].tool_id, "tool-b");
        }
        AgentDecision::Final { .. } => panic!("expected tool calls"),
    }
}

mod sub_agents {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves scripted OpenAI-compatible responses in order and records each request body.
    async fn scripted_model(responses: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(responses)));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen);
                let queue = Arc::clone(&queue);
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    loop {
                        let mut chunk = [0u8; 8192];
                        let Ok(read) = socket.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                            continue;
                        };
                        let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buffer.len() < end + 4 + length {
                            continue;
                        }
                        let body = buffer[end + 4..end + 4 + length].to_vec();
                        buffer.drain(..end + 4 + length);
                        seen.lock()
                            .unwrap()
                            .push(serde_json::from_slice::<Value>(&body).unwrap());
                        let reply = queue
                            .lock()
                            .unwrap()
                            .pop_front()
                            .expect("no scripted response left");
                        let reply = reply.to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            reply.len(),
                            reply
                        );
                        if socket.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (format!("http://{address}"), requests)
    }

    fn tool_call(name: &str, arguments: Value) -> Value {
        json!({ "choices": [{ "message": { "content": null, "tool_calls": [
            { "id": format!("call-{name}"), "type": "function", "function": { "name": name, "arguments": arguments.to_string() } }
        ] } }] })
    }

    fn text(content: &str) -> Value {
        json!({ "choices": [{ "message": { "content": content } }] })
    }

    fn model(base_url: String) -> ModelConfig {
        ModelConfig {
            provider_kind: ModelProviderKind::OpenAiCompatible,
            base_url,
            model: "test".into(),
            api_key: String::new(),
            ..ModelConfig::seeded()
        }
    }

    fn tools() -> Vec<AgentTool> {
        vec![
            agent_tool_from_dynamic(DynamicAgentToolDefinition {
                id: "t.lookup".into(),
                name: "Lookup".into(),
                description: "Looks something up".into(),
                input_schema: json!({ "type": "object" }),
                execution: DynamicAgentToolExecution::Sync {
                    observation: "lookup result".into(),
                    metadata: Value::Null,
                },
            }),
            agent_tool_from_dynamic(DynamicAgentToolDefinition {
                id: "t.write".into(),
                name: "Write".into(),
                description: "Writes, run by the host".into(),
                input_schema: json!({ "type": "object" }),
                execution: DynamicAgentToolExecution::Async {
                    pending_message: "writing".into(),
                    metadata: Value::Null,
                },
            }),
        ]
    }

    fn task(base_url: String, agents: Vec<AgentToolSpec>) -> AgentTask {
        AgentTask {
            name: "test".into(),
            session_id: None,
            turn_id: None,
            model: model(base_url),
            system_prompt: "parent system".into(),
            user_prompt: "do the job".into(),
            final_instructions: "final".into(),
            context: vec![AgentContextBlock::new("Samples", "sample text")],
            conversation: Vec::new(),
            prior_tool_observations: Vec::new(),
            tools: tools(),
            max_iterations: 8,
            start_ply: 1,
            observation_budget: None,
            max_output_tokens: 256,
            temperature: 0.2,
            inline_images: Vec::new(),
            stream: false,
            require_tool_before_final: false,
            continue_on_tool_error: true,
            sub_agents: SubAgentConfig {
                allow_create: true,
                agents,
            },
            resume_sub_agent: None,
        }
    }

    fn tool_names(request: &Value) -> Vec<String> {
        request["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn writer_spec() -> AgentToolSpec {
        AgentToolSpec {
            id: "drafter".into(),
            name: "Drafter".into(),
            description: String::new(),
            instructions: "drafter instructions".into(),
            tools: vec!["t.lookup".into(), "t.write".into()],
            max_iterations: None,
        }
    }

    #[tokio::test]
    async fn model_creates_an_agent_and_calls_it_as_a_tool() {
        let (url, requests) = scripted_model(vec![
            tool_call("agent_create", json!({ "id": "checker", "name": "Checker", "instructions": "checker instructions", "tools": ["t.lookup"] })),
            tool_call("subagent_checker", json!({ "task": "check the facts" })),
            tool_call("t_lookup", json!({})),
            text("child done"),
            text("parent done"),
        ])
        .await;
        let mut defined = Vec::new();
        let result = run_agent_task_until_pause(&Client::new(), task(url, Vec::new()), |event| {
            if let AgentTaskEvent::AgentDefined { spec, .. } = event {
                defined.push(spec);
            }
            Ok(())
        })
        .await
        .unwrap();
        let AgentTaskRunResult::Completed(outcome) = result else {
            panic!("expected completion")
        };
        assert_eq!(outcome.final_text, "parent done");
        assert_eq!(defined.len(), 1);
        assert_eq!(defined[0].tools, vec!["t.lookup".to_string()]);

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(tool_names(&requests[1]).contains(&"subagent_checker".to_string()));
        // The sub-agent runs on its own prompt, the parent's context, only its tools and no agent tools.
        let child = &requests[2];
        let child_text = child["messages"].to_string();
        assert!(child_text.contains("checker instructions"));
        assert!(child_text.contains("sample text"));
        assert!(child_text.contains("check the facts"));
        assert!(!child_text.contains("parent system"));
        assert_eq!(tool_names(child), vec!["t_lookup".to_string()]);
        // Its answer comes back to the parent as the tool result.
        assert!(requests[4]["messages"].to_string().contains("child done"));
    }

    #[tokio::test]
    async fn a_sub_agent_waiting_on_a_host_tool_pauses_and_resumes_the_parent() {
        let (url, requests) = scripted_model(vec![
            tool_call("subagent_drafter", json!({ "task": "draft it" })),
            tool_call("t_write", json!({ "text": "draft" })),
            text("drafted"),
            text("all done"),
        ])
        .await;
        let first = run_agent_task_until_pause(
            &Client::new(),
            task(url.clone(), vec![writer_spec()]),
            |_| Ok(()),
        )
        .await
        .unwrap();
        let AgentTaskRunResult::Paused(pause) = first else {
            panic!("expected a pause")
        };
        assert_eq!(pause.tool_id, "t.write");
        assert_eq!(pause.ply, 1);
        assert!(pause.observations.is_empty());
        let mut frame = pause.sub_agent.expect("paused inside the sub-agent");
        assert_eq!(frame.agent_id, "drafter");
        assert_eq!(frame.parent_ply, 1);
        assert_eq!(frame.next_ply, 2);

        frame.observations.push(AgentToolObservation::resumed(
            pause.tool_id,
            pause.tool_name,
            pause.rationale,
            pause.input,
            pause.pending_message,
            "written",
            json!({}),
        ));
        let mut resumed = task(url, vec![writer_spec()]);
        resumed.prior_tool_observations = pause.observations;
        resumed.resume_sub_agent = Some(frame);
        let mut sub_events = 0;
        let second = run_agent_task_until_pause(&Client::new(), resumed, |event| {
            if matches!(event, AgentTaskEvent::SubAgent { .. }) {
                sub_events += 1;
            }
            Ok(())
        })
        .await
        .unwrap();
        let AgentTaskRunResult::Completed(outcome) = second else {
            panic!("expected completion")
        };
        assert_eq!(outcome.final_text, "all done");
        assert!(sub_events > 0);
        let requests = requests.lock().unwrap();
        assert!(requests[2]["messages"].to_string().contains("written"));
        let parent_final = requests[3]["messages"].to_string();
        assert!(parent_final.contains("drafted"));
        assert!(parent_final.contains("parent system"));
    }

    #[test]
    fn agent_definitions_are_checked_against_the_available_tools() {
        let tools = vec!["t.lookup".to_string()];
        let mut spec = writer_spec();
        assert!(validate_agent_tool_spec(&spec, &tools, &[])
            .unwrap_err()
            .contains("t.write"));
        spec.tools = vec!["t.lookup".into()];
        assert!(validate_agent_tool_spec(&spec, &tools, &[]).is_ok());
        spec.id = "bad id".into();
        assert!(validate_agent_tool_spec(&spec, &tools, &[]).is_err());
        let full = (0..MAX_SUB_AGENTS)
            .map(|index| AgentToolSpec {
                id: format!("a{index}"),
                ..writer_spec()
            })
            .collect::<Vec<_>>();
        let extra = AgentToolSpec {
            id: "extra".into(),
            tools: Vec::new(),
            ..writer_spec()
        };
        assert!(validate_agent_tool_spec(&extra, &tools, &full).is_err());
        let replacing = AgentToolSpec {
            id: "a0".into(),
            tools: Vec::new(),
            ..writer_spec()
        };
        assert!(validate_agent_tool_spec(&replacing, &tools, &full).is_ok());
    }
}
