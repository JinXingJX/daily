use crate::agent::types::{WorkspaceNode, WorkspaceNodeKind};
use crate::state::persistence::seeded_session;
use crate::state::{
    normalize_name, validate_name, AppState, DEFAULT_SESSION_TITLE, MAX_WORKSPACE_NODES,
};
use crate::util::unix_timestamp;
use std::sync::Arc;

impl WorkspaceNodeKind {
    fn default_name(self) -> &'static str {
        match self {
            Self::Root => "Workspace",
            Self::Folder => "New Folder",
            Self::Session => DEFAULT_SESSION_TITLE,
        }
    }
}

impl AppState {
    pub fn node(&self, node_id: u64) -> Option<&WorkspaceNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    pub fn node_mut(&mut self, node_id: u64) -> Option<&mut WorkspaceNode> {
        self.nodes.iter_mut().find(|node| node.id == node_id)
    }

    pub fn create_workspace_node(
        &mut self,
        parent_id: u64,
        kind: WorkspaceNodeKind,
        name: &str,
    ) -> Result<Option<u64>, String> {
        if self.nodes.len() >= MAX_WORKSPACE_NODES {
            return Err(format!(
                "A workspace can contain at most {MAX_WORKSPACE_NODES} nodes."
            ));
        }
        validate_name(name, "Workspace node name")?;
        let parent_kind = self
            .node(parent_id)
            .map(|node| node.kind)
            .ok_or_else(|| "Parent workspace node was not found.".to_string())?;
        if !matches!(
            parent_kind,
            WorkspaceNodeKind::Root | WorkspaceNodeKind::Folder
        ) {
            return Err("Only the root or a folder can accept child nodes.".into());
        }
        if kind == WorkspaceNodeKind::Root {
            return Err("The root node cannot be created manually.".into());
        }

        let node_id = self.next_node_id;
        self.next_node_id += 1;
        let trimmed_name = normalize_name(name, kind.default_name());
        let session_id = if kind == WorkspaceNodeKind::Session {
            let session_id = self.next_session_id;
            self.next_session_id += 1;
            let session = seeded_session(
                session_id,
                trimmed_name.clone(),
                self.agent_apps.first().map(|agent| agent.id),
            );
            self.sessions.push(Arc::new(session));
            self.active_session_id = Some(session_id);
            Some(session_id)
        } else {
            None
        };

        let node = WorkspaceNode {
            id: node_id,
            kind,
            name: trimmed_name,
            parent_id: Some(parent_id),
            children: Vec::new(),
            session_id,
        };
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children.push(node_id);
        }
        self.nodes.push(node);
        Ok(session_id)
    }

    pub fn rename_workspace_node(&mut self, node_id: u64, name: &str) -> Result<(), String> {
        validate_name(name, "Workspace node name")?;
        let (kind, session_id) = {
            let node = self
                .node(node_id)
                .ok_or_else(|| "Workspace node was not found.".to_string())?;
            (node.kind, node.session_id)
        };
        let trimmed = normalize_name(name, kind.default_name());

        if let Some(node) = self.node_mut(node_id) {
            node.name = trimmed.clone();
        }
        if let Some(session_id) = session_id {
            if let Some(session) = self.session_mut(session_id) {
                session.title = trimmed;
                session.updated_at = unix_timestamp();
            }
        }
        Ok(())
    }

    pub fn move_workspace_node(
        &mut self,
        node_id: u64,
        parent_id: u64,
        index: Option<usize>,
    ) -> Result<(), String> {
        if node_id == self.root_id {
            return Err("The root node cannot be moved.".into());
        }
        let parent_kind = self
            .node(parent_id)
            .map(|node| node.kind)
            .ok_or_else(|| "Destination folder was not found.".to_string())?;
        if !matches!(
            parent_kind,
            WorkspaceNodeKind::Root | WorkspaceNodeKind::Folder
        ) {
            return Err("Workspace nodes can only be moved under the root or a folder.".into());
        }
        if self.is_descendant(parent_id, node_id) {
            return Err("A node cannot be moved into its own descendant.".into());
        }

        let old_parent_id = self
            .node(node_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| "Workspace node is missing a parent.".to_string())?;
        if let Some(parent) = self.node_mut(old_parent_id) {
            parent.children.retain(|child_id| *child_id != node_id);
        }
        if let Some(parent) = self.node_mut(parent_id) {
            let insert_at = index
                .unwrap_or(parent.children.len())
                .min(parent.children.len());
            parent.children.insert(insert_at, node_id);
        }
        if let Some(node) = self.node_mut(node_id) {
            node.parent_id = Some(parent_id);
        }
        Ok(())
    }

    pub fn delete_workspace_node(&mut self, node_id: u64) -> Result<(), String> {
        if node_id == self.root_id {
            return Err("The root node cannot be deleted.".into());
        }
        let parent_id = self
            .node(node_id)
            .and_then(|node| node.parent_id)
            .ok_or_else(|| "Workspace node is missing a parent.".to_string())?;
        let mut session_ids = Vec::new();
        let descendants = self.collect_descendants(node_id)?;
        for descendant_id in &descendants {
            if let Some(session_id) = self.node(*descendant_id).and_then(|node| node.session_id) {
                session_ids.push(session_id);
            }
        }
        if session_ids
            .iter()
            .any(|session_id| self.is_session_running(*session_id))
        {
            return Err("Stop the active turn before deleting its session.".into());
        }
        self.nodes.retain(|node| !descendants.contains(&node.id));
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children.retain(|child_id| *child_id != node_id);
        }
        self.sessions
            .retain(|session| !session_ids.contains(&session.id));
        if let Some(active_session_id) = self.active_session_id {
            if session_ids.contains(&active_session_id) {
                self.active_session_id = self.first_session_id();
            }
        }
        Ok(())
    }

    pub fn open_session(&mut self, session_id: u64) -> Result<(), String> {
        if self.session(session_id).is_none() {
            return Err("Session was not found.".into());
        }
        self.active_session_id = Some(session_id);
        Ok(())
    }

    pub fn session_node_id(&self, session_id: u64) -> Option<u64> {
        self.nodes
            .iter()
            .find(|node| node.session_id == Some(session_id))
            .map(|node| node.id)
    }

    pub fn first_session_id(&self) -> Option<u64> {
        self.nodes
            .iter()
            .find(|node| node.kind == WorkspaceNodeKind::Session)
            .and_then(|node| node.session_id)
    }

    pub(crate) fn collect_descendants(&self, node_id: u64) -> Result<Vec<u64>, String> {
        let mut stack = vec![node_id];
        let mut result = Vec::new();
        while let Some(current) = stack.pop() {
            let node = self
                .node(current)
                .ok_or_else(|| "Workspace node was not found during deletion.".to_string())?;
            result.push(current);
            for child_id in node.children.iter().rev() {
                stack.push(*child_id);
            }
        }
        Ok(result)
    }

    pub(crate) fn is_descendant(&self, node_id: u64, ancestor_id: u64) -> bool {
        let mut current_id = Some(node_id);
        while let Some(id) = current_id {
            if id == ancestor_id {
                return true;
            }
            current_id = self.node(id).and_then(|node| node.parent_id);
        }
        false
    }
}
