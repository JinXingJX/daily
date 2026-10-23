use super::types::MemoryEntry;

#[derive(Clone)]
pub struct WorkingMemoryItem {
    pub kind: String,
    pub title: String,
    pub content: String,
}

impl WorkingMemoryItem {
    pub fn new(
        kind: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            content: content.into(),
        }
    }
}

pub fn recent_working_memory(memory: &[MemoryEntry], limit: usize) -> Vec<WorkingMemoryItem> {
    memory
        .iter()
        .rev()
        .take(limit.max(1))
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|item| WorkingMemoryItem::new(item.kind.label(), item.title, item.content))
        .collect()
}
