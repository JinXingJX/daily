use crate::agent;
use crate::agent::{
    AgentStore, AppSnapshot, BindSessionAgentInput, ContextRootOption, CreateAgentAppInput,
    CreateWorkspaceNodeInput, DeleteAgentAppInput, DeleteWorkspaceNodeInput,
    MoveWorkspaceNodeInput, OpenSessionInput, RenameWorkspaceNodeInput,
};
use crate::limits::MAX_CONTEXT_ROOTS;
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::mutate_state;
use std::fs;
use std::path::Path;

pub(crate) fn create_workspace_node(
    app: RuntimeContext,
    input: CreateWorkspaceNodeInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.create_workspace_node(input.parent_id, input.kind, &input.name)?;
        Ok(())
    })
}

pub(crate) fn rename_workspace_node(
    app: RuntimeContext,
    input: RenameWorkspaceNodeInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.rename_workspace_node(input.node_id, &input.name)
    })
}

pub(crate) fn move_workspace_node(
    app: RuntimeContext,
    input: MoveWorkspaceNodeInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.move_workspace_node(input.node_id, input.parent_id, input.index)
    })
}

pub(crate) fn delete_workspace_node(
    app: RuntimeContext,
    input: DeleteWorkspaceNodeInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.delete_workspace_node(input.node_id)
    })
}

pub(crate) fn open_session(
    app: RuntimeContext,
    input: OpenSessionInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.open_session(input.session_id))
}

pub(crate) fn create_agent_app(
    app: RuntimeContext,
    input: CreateAgentAppInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.create_agent_app(input)?;
        Ok(())
    })
}

pub(crate) fn update_agent_app(
    app: RuntimeContext,
    input: agent::UpdateAgentAppInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.update_agent_app(input))
}

pub(crate) fn delete_agent_app(
    app: RuntimeContext,
    input: DeleteAgentAppInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.delete_agent_app(input.id))
}

pub(crate) fn bind_session_agent(
    app: RuntimeContext,
    input: BindSessionAgentInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        store.bind_session_agent(input.session_id, input.agent_app_id)
    })
}

pub(crate) fn list_context_roots(state: &AgentStore) -> Result<Vec<ContextRootOption>, String> {
    let mut roots = vec![ContextRootOption {
        path: ".".into(),
        label: "Project Root".into(),
    }];
    let mut dirs = Vec::new();
    collect_project_dirs(&state.project_root, &state.project_root, 0, &mut dirs)?;
    dirs.sort_by(|left, right| left.path.cmp(&right.path));
    roots.extend(dirs);
    Ok(roots)
}

pub(crate) fn collect_project_dirs(
    root: &Path,
    current: &Path,
    depth: usize,
    out: &mut Vec<ContextRootOption>,
) -> Result<(), String> {
    if depth > 3 {
        return Ok(());
    }
    for entry in fs::read_dir(current)
        .map_err(|error| format!("Failed to read directory {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("Failed to inspect directory entry: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("Failed to inspect directory entry type: {error}"))?
            .is_symlink()
        {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() || should_skip_dir(&path) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("Failed to normalize context root: {error}"))?;
        let relative_display = relative.display().to_string().replace('\\', "/");
        if out.len() >= MAX_CONTEXT_ROOTS {
            return Err(format!(
                "Workspace contains more than {MAX_CONTEXT_ROOTS} context roots within the scan depth."
            ));
        }
        out.push(ContextRootOption {
            path: relative_display.clone(),
            label: relative_display.clone(),
        });
        collect_project_dirs(root, &path, depth + 1, out)?;
    }
    Ok(())
}

pub(crate) fn should_skip_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".git" | ".daily" | "node_modules" | "dist" | "out" | "target"
    )
}
