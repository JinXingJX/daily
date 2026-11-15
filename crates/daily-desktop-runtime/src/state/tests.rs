use crate::agent::types::{AgentAppConfig, AgentEventViewKind, AgentTurn, TurnStatus};
use crate::plugins::PluginState;
use crate::state::persistence::{
    sanitize_loaded_session, validate_and_normalize_loaded_workspace, WorkspaceFile,
};
use crate::state::AppState;
use std::path::Path;
use std::sync::Arc;

fn seeded_test_state() -> AppState {
    let loaded = crate::settings::default_settings(Path::new("/tmp/daily-agent-routing-tests"))
        .expect("default settings should load");
    let mut state = AppState::seed_new(
        loaded.agent,
        loaded.model,
        loaded.view,
        PluginState::default(),
    );
    let base_agent = state.agent_apps[0].clone();
    state.agent_apps = vec![
        AgentAppConfig {
            id: 1,
            name: "Agent One".into(),
            ..base_agent.clone()
        },
        AgentAppConfig {
            id: 2,
            name: "Research Agent".into(),
            ..base_agent.clone()
        },
        AgentAppConfig {
            id: 3,
            name: "Writer Agent".into(),
            ..base_agent
        },
    ];
    Arc::make_mut(&mut state.sessions[0]).agent_app_id = Some(1);
    Arc::make_mut(&mut state.sessions[0]).last_agent_app_id = None;
    state
}

#[test]
fn antigravity_agent_binding_survives_create_update_and_restart() {
    use crate::model::{ModelAuthConfig, ModelProviderKind};
    let mut state = seeded_test_state();
    let mut auth = ModelAuthConfig::antigravity_default();
    auth.models = vec!["gemini-pro".into(), "claude-sonnet".into()];
    auth.default_model = "gemini-pro".into();
    state.model_config.auth_configs.push(auth.clone());
    state.next_agent_app_id = 4;
    let input = serde_json::from_value(serde_json::json!({
        "name":"Antigravity Agent", "systemPrompt":"Test", "authConfigId":auth.id,
        "model":"gemini-pro", "temperature":0.2,
    }))
    .unwrap();
    let agent = state.create_agent_app(input).unwrap();
    assert!(agent.provider_kind == ModelProviderKind::AntigravityLogin);
    state
        .update_agent_app(
            serde_json::from_value(serde_json::json!({
                "id":agent.id,"name":agent.name,"systemPrompt":"Test", "authConfigId":auth.id,
                "model":"claude-sonnet","temperature":0.2,
            }))
            .unwrap(),
        )
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "daily-antigravity-restart-{}",
        crate::util::unique_stamp()
    ));
    std::fs::create_dir_all(&root).unwrap();
    state.persist(&root).unwrap();
    let restored = AppState::seeded(
        &root,
        state.agent_config.clone(),
        state.model_config.clone(),
        state.settings.clone(),
    )
    .unwrap();
    let restored_agent = restored
        .agent_apps
        .iter()
        .find(|a| a.id == agent.id)
        .unwrap();
    assert!(restored_agent.provider_kind == ModelProviderKind::AntigravityLogin);
    assert_eq!(restored_agent.auth_config_id, auth.id);
    assert_eq!(restored_agent.model, "claude-sonnet");
    assert!(restored.agent_apps[0].provider_kind == ModelProviderKind::CodexLogin);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn find_agent_to_run_continues_previous_turn_agent() {
    let mut state = seeded_test_state();
    Arc::make_mut(&mut state.sessions[0]).turns.push(AgentTurn {
        id: 1,
        prompt: "First question".into(),
        response: "Answer".into(),
        strategy: "Completed".into(),
        status: TurnStatus::Completed,
        partial_response: String::new(),
        steps: Vec::new(),
        event_views: Vec::new(),
        images: Vec::new(),
        agent_app_id: Some(2),
        agent_name: Some("Research Agent".into()),
        pending_async_tool: None,
        checkpoint: None,
        usage: None,
        error_code: None,
    });

    let selected = state
        .find_agent_to_run(1, "continue with the same topic")
        .expect("agent should be selected");

    assert_eq!(selected.id, 2);
}

#[test]
fn find_agent_to_run_prefers_explicit_agent_mention() {
    let mut state = seeded_test_state();
    Arc::make_mut(&mut state.sessions[0]).last_agent_app_id = Some(2);

    let selected = state
        .find_agent_to_run(1, "@Writer Agent revise this section")
        .expect("agent should be selected");

    assert_eq!(selected.id, 3);
}

#[test]
fn bare_agent_names_and_partial_mentions_do_not_route() {
    let mut state = seeded_test_state();
    Arc::make_mut(&mut state.sessions[0]).last_agent_app_id = Some(2);

    for prompt in [
        "ask the writer agent to revise this",
        "mail writer agent@writer agent.example",
        "@Writer Agents should not match",
    ] {
        let selected = state.find_agent_to_run(1, prompt).unwrap();
        assert_eq!(selected.id, 2, "{prompt}");
    }
    let selected = state
        .find_agent_to_run(1, "(@writer agent) please revise")
        .unwrap();
    assert_eq!(selected.id, 3);
}

#[test]
fn queue_turn_records_user_input_event_view() {
    let mut state = seeded_test_state();

    let turn_id = state
        .queue_turn(1, "请检查项目结构", Vec::new())
        .expect("turn should queue");
    let turn = state.turn_mut(1, turn_id).expect("turn should exist");

    assert_eq!(turn.event_views.len(), 1);
    assert_eq!(turn.event_views[0].kind, AgentEventViewKind::UserInput);
    assert_eq!(turn.event_views[0].content, "请检查项目结构");
}

#[test]
fn loaded_workspace_repairs_stale_counters_and_rejects_duplicate_nodes() {
    let state = seeded_test_state();
    let mut file = WorkspaceFile {
        root_id: state.root_id,
        active_session_id: state.active_session_id,
        nodes: state.nodes.clone(),
        agent_apps: state.agent_apps.clone(),
        next_node_id: 1,
        next_session_id: 1,
        next_agent_app_id: 1,
    };

    validate_and_normalize_loaded_workspace(&mut file)
        .expect("valid workspace should be normalized");
    assert_eq!(file.next_node_id, 3);
    assert_eq!(file.next_session_id, 2);
    assert_eq!(file.next_agent_app_id, 4);

    file.nodes.push(file.nodes[0].clone());
    assert!(validate_and_normalize_loaded_workspace(&mut file)
        .expect_err("duplicate node ids must be rejected")
        .contains("unique non-zero"));
}

#[test]
fn loaded_session_repairs_stale_counters_and_rejects_duplicate_turns() {
    let mut state = seeded_test_state();
    Arc::make_mut(&mut state.sessions[0]).next_message_id = 1;
    Arc::make_mut(&mut state.sessions[0]).next_memory_id = 1;
    Arc::make_mut(&mut state.sessions[0]).next_turn_id = 1;
    sanitize_loaded_session(Arc::make_mut(&mut state.sessions[0]))
        .expect("valid session should be normalized");
    assert_eq!(state.sessions[0].next_message_id, 2);
    assert_eq!(state.sessions[0].next_memory_id, 2);

    let turn = AgentTurn {
        id: 1,
        prompt: "hello".into(),
        response: String::new(),
        strategy: String::new(),
        status: TurnStatus::Completed,
        partial_response: String::new(),
        steps: Vec::new(),
        event_views: Vec::new(),
        images: Vec::new(),
        agent_app_id: None,
        agent_name: None,
        pending_async_tool: None,
        checkpoint: None,
        usage: None,
        error_code: None,
    };
    Arc::make_mut(&mut state.sessions[0]).turns = vec![turn.clone(), turn];
    assert!(
        sanitize_loaded_session(Arc::make_mut(&mut state.sessions[0]))
            .expect_err("duplicate turn ids must be rejected")
            .contains("duplicate turn ids")
    );
}

fn persisted_test_state() -> (std::path::PathBuf, AppState, u64, u64) {
    use crate::agent::types::WorkspaceNodeKind;

    let root = std::env::temp_dir().join(format!(
        "daily-persist-test-{}",
        crate::util::unique_stamp()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut state = seeded_test_state();
    let first = state.sessions[0].id;
    let second = state
        .create_workspace_node(state.root_id, WorkspaceNodeKind::Session, "Second")
        .unwrap()
        .unwrap();
    state.persist(&root).unwrap();
    (root, state, first, second)
}

fn session_path(root: &Path, session_id: u64) -> std::path::PathBuf {
    root.join(".daily/sessions")
        .join(format!("{session_id}.json"))
}

#[test]
fn persist_rewrites_only_sessions_that_changed() {
    use crate::agent::types::MessageRole;

    let (root, mut state, first, second) = persisted_test_state();
    std::fs::remove_file(session_path(&root, first)).unwrap();
    std::fs::remove_file(session_path(&root, second)).unwrap();

    state.push_message(second, MessageRole::User, "hello", None);
    state.persist(&root).unwrap();

    assert!(
        !session_path(&root, first).exists(),
        "unchanged session was rewritten"
    );
    let written = std::fs::read_to_string(session_path(&root, second)).unwrap();
    assert!(written.contains("hello"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rolled_back_clone_does_not_mask_unpersisted_changes() {
    use crate::agent::types::MessageRole;

    let (root, mut state, first, _) = persisted_test_state();
    let previous = state.clone();
    state.push_message(first, MessageRole::User, "rolled back", None);
    // Simulate a failed write path: the cache is cleared, then memory rolls back.
    state.persisted_sessions.clear();
    state = previous;

    std::fs::remove_file(session_path(&root, first)).unwrap();
    state.persist(&root).unwrap();
    assert!(
        session_path(&root, first).exists(),
        "a cleared cache must force every session to be rewritten"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn continue_failed_turn_reopens_only_the_latest_failed_turn() {
    use crate::agent::{apply_event, AgentEvent};
    let mut state = seeded_test_state();
    let session_id = state.sessions[0].id;
    let first = state
        .queue_turn(session_id, "第一个问题", Vec::new())
        .unwrap();
    state.complete_turn(session_id);
    state.turn_mut(session_id, first).unwrap().status = TurnStatus::Completed;
    let turn_id = state
        .queue_turn(session_id, "重构世界观", Vec::new())
        .unwrap();
    state.turn_mut(session_id, turn_id).unwrap().checkpoint = Some(crate::agent::TurnCheckpoint {
        runtime: None,
        observations: Vec::new(),
        next_ply: 3,
    });
    apply_event(
        &mut state,
        AgentEvent::TurnFailed {
            session_id,
            turn_id,
            error: "Transport error".into(),
            code: None,
            raw_output: None,
        },
    );
    let before = state.session(session_id).unwrap().messages.len();
    assert!(state.continue_failed_turn(session_id, first).is_err());

    state.continue_failed_turn(session_id, turn_id).unwrap();
    let session = state.session(session_id).unwrap();
    let turn = session.turns.last().unwrap();
    assert!(matches!(turn.status, TurnStatus::Running));
    assert!(turn.response.is_empty());
    assert!(turn.steps.iter().all(|step| step.selected_kind != "error"));
    assert!(turn
        .event_views
        .iter()
        .all(|view| !matches!(view.kind, AgentEventViewKind::Error)));
    assert_eq!(turn.checkpoint.as_ref().unwrap().next_ply, 3);
    // The user's message stays; the failure reply is gone and no new message was added.
    assert_eq!(session.messages.len(), before - 1);
    assert_eq!(session.messages.last().unwrap().content, "重构世界观");
    assert_eq!(state.running_session_ids(), vec![session_id]);
    assert!(state.continue_failed_turn(session_id, turn_id).is_err());
}

#[test]
fn sessions_run_concurrently_up_to_the_limit() {
    use crate::agent::types::WorkspaceNodeKind;

    let mut state = seeded_test_state();
    let root = state.root_id;
    let mut session_ids = vec![1];
    for index in 0..crate::limits::MAX_CONCURRENT_TURNS {
        let session_id = state
            .create_workspace_node(root, WorkspaceNodeKind::Session, &format!("Task {index}"))
            .unwrap()
            .unwrap();
        session_ids.push(session_id);
    }

    let first = state
        .queue_turn(session_ids[0], "first", Vec::new())
        .unwrap();
    assert!(state
        .queue_turn(session_ids[0], "again", Vec::new())
        .is_err());
    state
        .queue_turn(session_ids[1], "second", Vec::new())
        .unwrap();
    // Opening another session no longer has to wait for running turns.
    state.open_session(session_ids[2]).unwrap();
    for session_id in &session_ids[2..crate::limits::MAX_CONCURRENT_TURNS] {
        state.queue_turn(*session_id, "more", Vec::new()).unwrap();
    }
    let over_limit = state
        .queue_turn(
            session_ids[crate::limits::MAX_CONCURRENT_TURNS],
            "too many",
            Vec::new(),
        )
        .unwrap_err();
    assert!(over_limit.contains("At most"), "{over_limit}");
    assert_eq!(
        state.runtime_view().running_session_ids.len(),
        crate::limits::MAX_CONCURRENT_TURNS
    );

    // Finishing one session frees a slot without touching the others.
    state.complete_turn(session_ids[0]);
    assert!(!state.is_session_running(session_ids[0]));
    assert!(state.is_session_running(session_ids[1]));
    assert!(state.runtime_view().is_running);
    state
        .queue_turn(
            session_ids[crate::limits::MAX_CONCURRENT_TURNS],
            "now fits",
            Vec::new(),
        )
        .unwrap();
    assert!(state.turn_mut(session_ids[0], first).is_some());
}
