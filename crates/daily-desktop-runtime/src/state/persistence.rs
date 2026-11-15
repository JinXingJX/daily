use crate::agent::types::{
    AgentAppConfig, AgentHookConfig, ChatSessionState, DialogueMessage, MemoryEntry, MemoryKind,
    MessageRole, TurnStatus, WorkspaceNode, WorkspaceNodeKind,
};
use crate::agent::AgentConfig;
use crate::fs_utils::{atomic_write, read_limited_utf8};
use crate::model::{normalize_provider_model, ModelConfig};
use crate::plugins::PluginState;
use crate::settings::SettingsView;
use crate::state::agent_apps::{
    auth_config_for_id, normalize_auth_config_id, normalize_context_roots, normalize_hook_config,
    validate_agent_input, validate_hook_graph,
};
use crate::state::{
    validate_name, AppState, APP_DIR_NAME, DEFAULT_SESSION_TITLE, INTRO_MEMORY, INTRO_MESSAGE,
    MAX_AGENT_APPS, MAX_SESSION_MEMORY_ENTRIES, MAX_SESSION_MESSAGES, MAX_SESSION_TURNS,
    MAX_STATE_FILE_BYTES, MAX_TURN_EVENT_VIEWS, MAX_TURN_IMAGES, MAX_TURN_STEPS,
    MAX_WORKSPACE_NODES, SESSIONS_DIR_NAME, WORKSPACE_FILE_NAME,
};
use crate::util::unix_timestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFile {
    pub(crate) root_id: u64,
    pub(crate) active_session_id: Option<u64>,
    pub(crate) nodes: Vec<WorkspaceNode>,
    pub(crate) agent_apps: Vec<AgentAppConfig>,
    pub(crate) next_node_id: u64,
    pub(crate) next_session_id: u64,
    pub(crate) next_agent_app_id: u64,
}

pub(crate) fn workspace_dir_path(project_root: &Path) -> PathBuf {
    project_root.join(APP_DIR_NAME)
}

pub(crate) fn workspace_file_path(project_root: &Path) -> PathBuf {
    workspace_dir_path(project_root).join(WORKSPACE_FILE_NAME)
}

pub(crate) fn sessions_dir_path(project_root: &Path) -> PathBuf {
    workspace_dir_path(project_root).join(SESSIONS_DIR_NAME)
}

pub(crate) fn session_file_path(project_root: &Path, session_id: u64) -> PathBuf {
    sessions_dir_path(project_root).join(format!("{session_id}.json"))
}

pub(crate) fn seeded_session(
    id: u64,
    title: String,
    agent_app_id: Option<u64>,
) -> ChatSessionState {
    let mut session = ChatSessionState {
        id,
        title,
        agent_app_id,
        last_agent_app_id: agent_app_id,
        messages: Vec::new(),
        history_summary: None,
        turns: Vec::new(),
        memory: Vec::new(),
        updated_at: unix_timestamp(),
        running_turn_id: None,
        next_message_id: 1,
        next_memory_id: 1,
        next_turn_id: 1,
    };
    push_seed_message(&mut session, MessageRole::Assistant, INTRO_MESSAGE);
    push_seed_memory(
        &mut session,
        0,
        MemoryKind::System,
        "Session initialized",
        INTRO_MEMORY,
    );
    session
}

pub(crate) fn push_seed_message(session: &mut ChatSessionState, role: MessageRole, content: &str) {
    let id = session.next_message_id;
    session.next_message_id += 1;
    session.messages.push(DialogueMessage {
        id,
        turn_id: None,
        role,
        content: content.to_string(),
    });
}

pub(crate) fn push_seed_memory(
    session: &mut ChatSessionState,
    turn_id: u64,
    kind: MemoryKind,
    title: &str,
    content: &str,
) {
    let id = session.next_memory_id;
    session.next_memory_id += 1;
    session.memory.push(MemoryEntry {
        id,
        turn_id,
        kind,
        title: title.to_string(),
        content: content.to_string(),
    });
}

pub(crate) fn validate_and_normalize_loaded_workspace(
    file: &mut WorkspaceFile,
) -> Result<(), String> {
    if file.nodes.len() > MAX_WORKSPACE_NODES {
        return Err(format!(
            "Workspace contains more than {MAX_WORKSPACE_NODES} nodes."
        ));
    }
    if file.agent_apps.len() > MAX_AGENT_APPS {
        return Err(format!(
            "Workspace contains more than {MAX_AGENT_APPS} agents."
        ));
    }

    let mut nodes_by_id = BTreeMap::new();
    for node in &file.nodes {
        if node.id == 0 || nodes_by_id.insert(node.id, node).is_some() {
            return Err("Workspace node ids must be unique non-zero values.".into());
        }
        validate_name(&node.name, "Workspace node name")?;
    }

    if !file.nodes.is_empty() {
        let root = nodes_by_id
            .get(&file.root_id)
            .ok_or_else(|| "Workspace root node was not found.".to_string())?;
        if root.kind != WorkspaceNodeKind::Root
            || root.parent_id.is_some()
            || root.session_id.is_some()
        {
            return Err("Workspace root node is malformed.".into());
        }

        let mut session_ids = BTreeSet::new();
        for node in &file.nodes {
            if node.id != file.root_id && node.kind == WorkspaceNodeKind::Root {
                return Err("Workspace can contain only one root node.".into());
            }
            match node.kind {
                WorkspaceNodeKind::Session if node.session_id.is_none() => {
                    return Err(format!(
                        "Session node {} is missing its session id.",
                        node.id
                    ));
                }
                WorkspaceNodeKind::Session if !node.children.is_empty() => {
                    return Err(format!("Session node {} cannot contain children.", node.id));
                }
                WorkspaceNodeKind::Root | WorkspaceNodeKind::Folder
                    if node.session_id.is_some() =>
                {
                    return Err(format!(
                        "Non-session node {} cannot reference a session.",
                        node.id
                    ));
                }
                _ => {}
            }
            if let Some(session_id) = node.session_id {
                if session_id == 0 || !session_ids.insert(session_id) {
                    return Err("Workspace session ids must be unique non-zero values.".into());
                }
            }

            let mut children = BTreeSet::new();
            for child_id in &node.children {
                if !children.insert(*child_id) {
                    return Err(format!("Workspace node {} lists a child twice.", node.id));
                }
                let child = nodes_by_id.get(child_id).ok_or_else(|| {
                    format!("Workspace node {} references a missing child.", node.id)
                })?;
                if child.parent_id != Some(node.id) {
                    return Err(format!(
                        "Workspace child {} has an inconsistent parent.",
                        child.id
                    ));
                }
            }
            if node.id != file.root_id {
                let parent_id = node
                    .parent_id
                    .ok_or_else(|| format!("Workspace node {} is missing its parent.", node.id))?;
                let parent = nodes_by_id.get(&parent_id).ok_or_else(|| {
                    format!("Workspace node {} references a missing parent.", node.id)
                })?;
                if !matches!(
                    parent.kind,
                    WorkspaceNodeKind::Root | WorkspaceNodeKind::Folder
                ) || !parent.children.contains(&node.id)
                {
                    return Err(format!(
                        "Workspace node {} is not linked from a valid parent.",
                        node.id
                    ));
                }
            }
        }

        let mut reachable = BTreeSet::new();
        let mut stack = vec![file.root_id];
        while let Some(node_id) = stack.pop() {
            if !reachable.insert(node_id) {
                return Err("Workspace node graph contains a cycle.".into());
            }
            let node = nodes_by_id
                .get(&node_id)
                .ok_or_else(|| "Workspace graph references a missing node.".to_string())?;
            stack.extend(node.children.iter().copied());
        }
        if reachable.len() != file.nodes.len() {
            return Err("Workspace node graph contains unreachable nodes.".into());
        }
        if file
            .active_session_id
            .is_some_and(|session_id| !session_ids.contains(&session_id))
        {
            file.active_session_id = session_ids.first().copied();
        }
        file.next_node_id = next_available_id(
            file.next_node_id,
            file.nodes.iter().map(|node| node.id).max(),
        )?;
        file.next_session_id = next_available_id(
            file.next_session_id,
            session_ids.iter().next_back().copied(),
        )?;
    }

    let mut agent_ids = BTreeSet::new();
    for agent in &mut file.agent_apps {
        if agent.id == 0 || !agent_ids.insert(agent.id) {
            return Err("Agent ids must be unique non-zero values.".into());
        }
        validate_agent_input(
            &agent.name,
            &agent.system_prompt,
            &agent.model,
            &agent.mounted_plugin_ids,
            &agent.context_roots,
            &agent.on_start_hook,
            &agent.on_finish_hook,
        )?;
        if !agent.temperature.is_finite() {
            return Err("Agent temperature must be a finite number.".into());
        }
        agent.temperature = agent.temperature.clamp(0.0, 2.0);
        agent.context_roots = normalize_context_roots(std::mem::take(&mut agent.context_roots));
        agent.on_start_hook = normalize_hook_config(agent.on_start_hook.clone(), agent.id);
        agent.on_finish_hook = normalize_hook_config(agent.on_finish_hook.clone(), agent.id);
    }
    validate_hook_graph(&file.agent_apps)?;
    file.next_agent_app_id = next_available_id(
        file.next_agent_app_id,
        file.agent_apps.iter().map(|agent| agent.id).max(),
    )?;
    Ok(())
}

pub(crate) fn next_available_id(current: u64, maximum: Option<u64>) -> Result<u64, String> {
    let minimum = maximum
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "Persisted id counter is exhausted.".to_string())?;
    Ok(current.max(minimum).max(1))
}

pub(crate) fn sanitize_loaded_session(session: &mut ChatSessionState) -> Result<(), String> {
    validate_name(&session.title, "Session title")?;
    if session.messages.len() > MAX_SESSION_MESSAGES
        || session.turns.len() > MAX_SESSION_TURNS
        || session.memory.len() > MAX_SESSION_MEMORY_ENTRIES
    {
        return Err(format!(
            "Session {} exceeds the supported item count limits.",
            session.id
        ));
    }

    let message_ids = session
        .messages
        .iter()
        .map(|message| message.id)
        .collect::<BTreeSet<_>>();
    if message_ids.len() != session.messages.len() || message_ids.contains(&0) {
        return Err(format!(
            "Session {} contains invalid or duplicate message ids.",
            session.id
        ));
    }
    let turn_ids = session
        .turns
        .iter()
        .map(|turn| turn.id)
        .collect::<BTreeSet<_>>();
    if turn_ids.len() != session.turns.len() || turn_ids.contains(&0) {
        return Err(format!(
            "Session {} contains invalid or duplicate turn ids.",
            session.id
        ));
    }
    let memory_ids = session
        .memory
        .iter()
        .map(|entry| entry.id)
        .collect::<BTreeSet<_>>();
    if memory_ids.len() != session.memory.len() || memory_ids.contains(&0) {
        return Err(format!(
            "Session {} contains invalid or duplicate memory ids.",
            session.id
        ));
    }
    for turn in &session.turns {
        if turn.images.len() > MAX_TURN_IMAGES
            || turn.steps.len() > MAX_TURN_STEPS
            || turn.event_views.len() > MAX_TURN_EVENT_VIEWS
        {
            return Err(format!("Turn {} exceeds supported item limits.", turn.id));
        }
        for image in &turn.images {
            image.clone().into_model_image()?;
        }
    }

    session.next_message_id = next_available_id(
        session.next_message_id,
        session.messages.iter().map(|message| message.id).max(),
    )?;
    session.next_turn_id = next_available_id(
        session.next_turn_id,
        session.turns.iter().map(|turn| turn.id).max(),
    )?;
    session.next_memory_id = next_available_id(
        session.next_memory_id,
        session.memory.iter().map(|entry| entry.id).max(),
    )?;
    backfill_message_turn_ids(session);
    session.running_turn_id = None;
    for turn in &mut session.turns {
        if matches!(turn.status, TurnStatus::Queued | TurnStatus::Running) {
            turn.status = TurnStatus::Failed;
            turn.strategy = "The app restarted while this turn was still running.".into();
            turn.partial_response.clear();
            if turn.response.is_empty() {
                turn.response = "The app restarted while this turn was still running.".into();
            }
        }
    }
    Ok(())
}

pub(crate) fn backfill_message_turn_ids(session: &mut ChatSessionState) {
    let mut message_index = 0;

    for turn in &session.turns {
        while message_index < session.messages.len()
            && !matches!(session.messages[message_index].role, MessageRole::User)
        {
            message_index += 1;
        }

        if message_index >= session.messages.len() {
            break;
        }

        if session.messages[message_index].turn_id.is_none() {
            session.messages[message_index].turn_id = Some(turn.id);
        }
        message_index += 1;

        while message_index < session.messages.len()
            && matches!(session.messages[message_index].role, MessageRole::Assistant)
        {
            if session.messages[message_index].turn_id.is_none() {
                session.messages[message_index].turn_id = Some(turn.id);
            }
            message_index += 1;
        }
    }
}

impl AppState {
    pub fn seeded(
        project_root: &Path,
        agent_config: AgentConfig,
        model_config: ModelConfig,
        settings: SettingsView,
    ) -> Result<Self, String> {
        let path = workspace_file_path(project_root);
        let plugins = PluginState::load(project_root)?;
        let mut state = if path.exists() {
            Self::load_from_disk(project_root, agent_config, model_config, settings, plugins)?
        } else {
            let state = Self::seed_new(agent_config, model_config, settings, plugins);
            state.persist(project_root)?;
            state
        };
        state.workspace_root = Some(project_root.to_path_buf());
        Ok(state)
    }

    pub(crate) fn seed_new(
        agent_config: AgentConfig,
        model_config: ModelConfig,
        settings: SettingsView,
        plugins: PluginState,
    ) -> Self {
        let agent_app = AgentAppConfig {
            id: 1,
            name: "Default Agent".into(),
            system_prompt: agent_config.system_prompt.clone(),
            provider_kind: model_config.provider_kind,
            auth_config_id: model_config.auth_config_id.clone(),
            model: normalize_provider_model(model_config.provider_kind, &model_config.model),
            temperature: model_config.temperature,
            mounted_plugin_ids: plugins
                .canonicalize_agent_mounts(vec!["builtin.chart".into(), "builtin.weather".into()]),
            context_roots: Vec::new(),
            on_start_hook: AgentHookConfig::default(),
            on_finish_hook: AgentHookConfig::default(),
        };

        let session = seeded_session(1, DEFAULT_SESSION_TITLE.into(), Some(agent_app.id));
        let nodes = vec![
            WorkspaceNode {
                id: 1,
                kind: WorkspaceNodeKind::Root,
                name: "Workspace".into(),
                parent_id: None,
                children: vec![2],
                session_id: None,
            },
            WorkspaceNode {
                id: 2,
                kind: WorkspaceNodeKind::Session,
                name: session.title.clone(),
                parent_id: Some(1),
                children: Vec::new(),
                session_id: Some(session.id),
            },
        ];

        Self {
            root_id: 1,
            active_session_id: Some(session.id),
            nodes,
            sessions: vec![Arc::new(session)],
            agent_apps: vec![agent_app],
            plugins,
            workspace_root: None,
            settings,
            agent_config,
            model_config,
            runtime_status: "Ready".into(),
            next_node_id: 3,
            next_session_id: 2,
            next_agent_app_id: 2,
            persisted_sessions: PersistedSessions::default(),
        }
    }

    pub(crate) fn load_from_disk(
        project_root: &Path,
        agent_config: AgentConfig,
        model_config: ModelConfig,
        settings: SettingsView,
        plugins: PluginState,
    ) -> Result<Self, String> {
        let path = workspace_file_path(project_root);
        let raw = read_limited_utf8(&path, MAX_STATE_FILE_BYTES, "workspace state")?;
        let mut file: WorkspaceFile = serde_json::from_str(&raw).map_err(|error| {
            format!("Failed to parse workspace file {}: {error}", path.display())
        })?;
        validate_and_normalize_loaded_workspace(&mut file)?;

        let sessions_dir = sessions_dir_path(project_root);
        let mut sessions = Vec::new();
        for node in &file.nodes {
            if let Some(session_id) = node.session_id {
                let session_path = session_file_path(project_root, session_id);
                let raw = read_limited_utf8(&session_path, MAX_STATE_FILE_BYTES, "session state")?;
                let mut session: ChatSessionState =
                    serde_json::from_str(&raw).map_err(|error| {
                        format!(
                            "Failed to parse session file {}: {error}",
                            session_path.display()
                        )
                    })?;
                if session.id != session_id {
                    return Err(format!(
                        "Session file {} declares id {}, expected {}.",
                        session_path.display(),
                        session.id,
                        session_id
                    ));
                }
                sanitize_loaded_session(&mut session)?;
                sessions.push(Arc::new(session));
            }
        }

        if !sessions_dir.exists() {
            fs::create_dir_all(&sessions_dir).map_err(|error| {
                format!(
                    "Failed to create sessions directory {}: {error}",
                    sessions_dir.display()
                )
            })?;
        }

        let persisted_sessions = PersistedSessions::default();
        for session in &sessions {
            persisted_sessions.record(session);
        }
        let mut state = Self {
            persisted_sessions,
            root_id: file.root_id,
            active_session_id: file.active_session_id,
            nodes: file.nodes,
            sessions,
            agent_apps: file.agent_apps,
            plugins,
            workspace_root: None,
            settings,
            agent_config,
            model_config,
            runtime_status: "Ready".into(),
            next_node_id: file.next_node_id,
            next_session_id: file.next_session_id,
            next_agent_app_id: file.next_agent_app_id,
        };
        state.canonicalize_agent_plugin_mounts();
        state.ensure_seeded_defaults();
        Ok(state)
    }

    pub fn persist(&self, project_root: &Path) -> Result<(), String> {
        let workspace_dir = workspace_dir_path(project_root);
        let sessions_dir = sessions_dir_path(project_root);
        fs::create_dir_all(&sessions_dir).map_err(|error| {
            format!(
                "Failed to create workspace directory {}: {error}",
                workspace_dir.display()
            )
        })?;

        let workspace = WorkspaceFile {
            root_id: self.root_id,
            active_session_id: self.active_session_id,
            nodes: self.nodes.clone(),
            agent_apps: self.agent_apps.clone(),
            next_node_id: self.next_node_id,
            next_session_id: self.next_session_id,
            next_agent_app_id: self.next_agent_app_id,
        };

        let workspace_payload = serde_json::to_string_pretty(&workspace)
            .map_err(|error| format!("Failed to serialize workspace file: {error}"))?;
        if workspace_payload.len() > MAX_STATE_FILE_BYTES {
            return Err(format!(
                "Workspace state exceeds the {MAX_STATE_FILE_BYTES} byte limit."
            ));
        }
        let session_payloads = self
            .sessions
            .iter()
            .filter(|session| !self.persisted_sessions.is_current(session))
            .map(|session| {
                let payload = serde_json::to_string_pretty(&**session).map_err(|error| {
                    format!("Failed to serialize session {}: {error}", session.id)
                })?;
                if payload.len() > MAX_STATE_FILE_BYTES {
                    return Err(format!(
                        "Session {} exceeds the {MAX_STATE_FILE_BYTES} byte limit.",
                        session.id
                    ));
                }
                Ok((session, payload))
            })
            .collect::<Result<Vec<_>, String>>()?;

        self.plugins.persist(project_root)?;
        for (session, payload) in &session_payloads {
            let session_id = session.id;
            if let Err(error) = atomic_write(
                &session_file_path(project_root, session_id),
                format!("{payload}\n"),
            ) {
                // The caller may roll back memory while disk keeps this write;
                // forget what was written so the next persist rewrites everything.
                self.persisted_sessions.clear();
                return Err(format!("Failed to write session {session_id}: {error}"));
            }
            self.persisted_sessions.record(session);
        }
        atomic_write(
            &workspace_file_path(project_root),
            format!("{workspace_payload}\n"),
        )
        .map_err(|error| format!("Failed to write workspace file: {error}"))?;

        if sessions_dir.exists() {
            for entry in fs::read_dir(&sessions_dir).map_err(|error| {
                format!(
                    "Failed to read sessions directory {}: {error}",
                    sessions_dir.display()
                )
            })? {
                let entry =
                    entry.map_err(|error| format!("Failed to inspect session file: {error}"))?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let keep = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.parse::<u64>().ok())
                    .map(|session_id| self.sessions.iter().any(|session| session.id == session_id))
                    .unwrap_or(true);
                if !keep {
                    fs::remove_file(&path).map_err(|error| {
                        format!(
                            "Failed to remove stale session file {}: {error}",
                            path.display()
                        )
                    })?;
                }
            }
        }

        Ok(())
    }

    pub fn persist_session(&self, project_root: &Path, session_id: u64) -> Result<(), String> {
        let session = self
            .session(session_id)
            .ok_or_else(|| "Session was not found.".to_string())?;
        let payload = serde_json::to_string_pretty(session)
            .map_err(|error| format!("Failed to serialize session {session_id}: {error}"))?;
        if payload.len() > MAX_STATE_FILE_BYTES {
            return Err(format!(
                "Session {session_id} exceeds the {MAX_STATE_FILE_BYTES} byte limit."
            ));
        }
        match atomic_write(
            &session_file_path(project_root, session_id),
            format!("{payload}\n"),
        ) {
            Ok(()) => {
                if let Some(session) = self
                    .sessions
                    .iter()
                    .find(|session| session.id == session_id)
                {
                    self.persisted_sessions.record(session);
                }
                Ok(())
            }
            Err(error) => {
                self.persisted_sessions.clear();
                Err(format!("Failed to write session {session_id}: {error}"))
            }
        }
    }

    pub(crate) fn ensure_seeded_defaults(&mut self) {
        if self.agent_apps.is_empty() {
            self.agent_apps.push(AgentAppConfig {
                id: self.next_agent_app_id.max(1),
                name: "Default Agent".into(),
                system_prompt: self.agent_config.system_prompt.clone(),
                provider_kind: self.model_config.provider_kind,
                auth_config_id: self.model_config.auth_config_id.clone(),
                model: normalize_provider_model(
                    self.model_config.provider_kind,
                    &self.model_config.model,
                ),
                temperature: self.model_config.temperature,
                mounted_plugin_ids: self.plugins.canonicalize_agent_mounts(vec![
                    "builtin.chart".into(),
                    "builtin.weather".into(),
                ]),
                context_roots: Vec::new(),
                on_start_hook: AgentHookConfig::default(),
                on_finish_hook: AgentHookConfig::default(),
            });
            self.next_agent_app_id = self.agent_apps[0].id + 1;
        }

        if self.nodes.is_empty() || self.sessions.is_empty() {
            let seeded = Self::seed_new(
                self.agent_config.clone(),
                self.model_config.clone(),
                self.settings.clone(),
                self.plugins.clone(),
            );
            self.root_id = seeded.root_id;
            self.active_session_id = seeded.active_session_id;
            self.nodes = seeded.nodes;
            self.sessions = seeded.sessions;
            self.agent_apps = seeded.agent_apps;
            self.next_node_id = seeded.next_node_id;
            self.next_session_id = seeded.next_session_id;
            self.next_agent_app_id = seeded.next_agent_app_id;
        } else if self.active_session_id.is_none() {
            self.active_session_id = self.first_session_id();
        }
        for agent in &mut self.agent_apps {
            agent.auth_config_id =
                normalize_auth_config_id(Some(&agent.auth_config_id), &self.model_config);
            let auth_config = auth_config_for_id(&self.model_config, &agent.auth_config_id);
            agent.provider_kind = auth_config.provider_kind;
            if agent.model.trim().is_empty() {
                agent.model =
                    normalize_provider_model(auth_config.provider_kind, &auth_config.default_model);
            } else {
                agent.model = normalize_provider_model(auth_config.provider_kind, &agent.model);
            }
        }
        self.canonicalize_agent_plugin_mounts();
    }

    pub(crate) fn canonicalize_agent_plugin_mounts(&mut self) {
        for agent in &mut self.agent_apps {
            agent.mounted_plugin_ids = self
                .plugins
                .canonicalize_agent_mounts(agent.mounted_plugin_ids.clone());
        }
    }
}

/// Sessions known to match their on-disk file, keyed by id and compared by
/// `Arc` identity: any mutation goes through `Arc::make_mut`, which yields a new
/// allocation while this cache holds the previous one. Shared across clones so
/// a rollback of `AppState` cannot resurrect stale entries.
#[derive(Clone, Default)]
pub(crate) struct PersistedSessions(Arc<Mutex<HashMap<u64, Arc<ChatSessionState>>>>);

impl PersistedSessions {
    pub(crate) fn is_current(&self, session: &Arc<ChatSessionState>) -> bool {
        self.0
            .lock()
            .map(|cache| {
                cache
                    .get(&session.id)
                    .is_some_and(|persisted| Arc::ptr_eq(persisted, session))
            })
            .unwrap_or(false)
    }

    pub(crate) fn record(&self, session: &Arc<ChatSessionState>) {
        if let Ok(mut cache) = self.0.lock() {
            cache.insert(session.id, session.clone());
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut cache) = self.0.lock() {
            cache.clear();
        }
    }
}
