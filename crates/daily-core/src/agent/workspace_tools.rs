//! Read-only workspace tools offered to chat agents: list a directory, read a text file, and
//! search file contents. Paths go through the same sandbox as plugin workspace APIs, so
//! traversal, symlink escapes, and the private `.daily` directory are rejected.

use super::sdk::{AgentTool, AgentToolCall, AgentToolExecution, AgentToolResult};
use crate::fs_utils::sandbox::{read_limited_text, resolve_workspace_file_path};
use crate::limits::APP_PRIVATE_DIR_NAME;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const WORKSPACE_LIST_TOOL_ID: &str = "workspace.list";
pub(crate) const WORKSPACE_READ_TOOL_ID: &str = "workspace.read";
pub(crate) const WORKSPACE_SEARCH_TOOL_ID: &str = "workspace.search";

const MAX_LIST_ENTRIES: usize = 500;
const MAX_READ_FILE_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_READ_LINES: usize = 400;
const MAX_READ_LINES: usize = 2_000;
const MAX_OBSERVATION_CHARS: usize = 60_000;
const MAX_SEARCH_MATCHES: usize = 200;
const MAX_SEARCH_FILES: usize = 5_000;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
/// Directories that are almost never useful to search and can be huge.
const SKIPPED_DIRECTORIES: &[&str] = &[".git", "node_modules", "target", "dist", "out", "release"];

/// Builds the workspace tools bound to `root`.
pub(crate) fn workspace_tools(root: &Path) -> Vec<AgentTool> {
    let list_root = root.to_path_buf();
    let read_root = root.to_path_buf();
    let search_root = root.to_path_buf();
    vec![
        AgentTool::new_with_execution(
            WORKSPACE_LIST_TOOL_ID,
            "List workspace directory",
            "Lists files and folders in a workspace directory (relative path, default the workspace root).",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory relative to the workspace root. Defaults to \".\"." }
                }
            }),
            move |call| completed(list_directory(&list_root, string_arg(&call, "path").unwrap_or("."))),
        ),
        AgentTool::new_with_execution(
            WORKSPACE_READ_TOOL_ID,
            "Read workspace file",
            "Reads a UTF-8 text file from the workspace, optionally a range of lines.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to the workspace root." },
                    "startLine": { "type": "integer", "minimum": 1, "description": "First line to return (1-based). Defaults to 1." },
                    "maxLines": { "type": "integer", "minimum": 1, "maximum": MAX_READ_LINES, "description": "Maximum number of lines to return." }
                },
                "required": ["path"]
            }),
            move |call| {
                let path = string_arg(&call, "path").ok_or("workspace.read requires path.")?;
                completed(read_file(
                    &read_root,
                    path,
                    usize_arg(&call, "startLine").unwrap_or(1),
                    usize_arg(&call, "maxLines").unwrap_or(DEFAULT_READ_LINES),
                ))
            },
        ),
        AgentTool::new_with_execution(
            WORKSPACE_SEARCH_TOOL_ID,
            "Search workspace files",
            "Finds lines containing a text (case-insensitive) in workspace text files.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Text to look for." },
                    "path": { "type": "string", "description": "Directory to search, relative to the workspace root. Defaults to \".\"." }
                },
                "required": ["query"]
            }),
            move |call| {
                let query = string_arg(&call, "query").ok_or("workspace.search requires query.")?;
                completed(search_files(
                    &search_root,
                    string_arg(&call, "path").unwrap_or("."),
                    query,
                ))
            },
        ),
    ]
}

fn completed(result: Result<String, String>) -> Result<AgentToolExecution, String> {
    // Tool errors go back to the model as observations so it can correct the path or query.
    let text = result.unwrap_or_else(|error| format!("Error: {error}"));
    Ok(AgentToolExecution::Completed(AgentToolResult::new(
        truncate(text),
    )))
}

fn string_arg<'a>(call: &'a AgentToolCall, key: &str) -> Option<&'a str> {
    call.input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn usize_arg(call: &AgentToolCall, key: &str) -> Option<usize> {
    call.input
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_OBSERVATION_CHARS {
        return text;
    }
    let mut truncated = text.chars().take(MAX_OBSERVATION_CHARS).collect::<String>();
    truncated.push_str("\n[output truncated]");
    truncated
}

fn list_directory(root: &Path, relative: &str) -> Result<String, String> {
    let path = resolve_workspace_file_path(root, relative)?;
    if !path.is_dir() {
        return Err(format!("{relative} is not a directory."));
    }
    let mut entries = fs::read_dir(&path)
        .map_err(|error| format!("Failed to list {relative}: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != APP_PRIVATE_DIR_NAME)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            Some(if file_type.is_dir() {
                format!("{name}/")
            } else {
                let size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                format!("{name} ({size} bytes)")
            })
        })
        .collect::<Vec<_>>();
    entries.sort();
    let total = entries.len();
    entries.truncate(MAX_LIST_ENTRIES);
    let mut output = format!("{relative}: {total} entries\n{}", entries.join("\n"));
    if total > MAX_LIST_ENTRIES {
        output.push_str(&format!(
            "\n[{} more entries not shown]",
            total - MAX_LIST_ENTRIES
        ));
    }
    Ok(output)
}

fn read_file(
    root: &Path,
    relative: &str,
    start_line: usize,
    max_lines: usize,
) -> Result<String, String> {
    let path = resolve_workspace_file_path(root, relative)?;
    if !path.is_file() {
        return Err(format!("{relative} is not a file."));
    }
    let content = read_limited_text(&path, MAX_READ_FILE_BYTES, "workspace file")?;
    let start = start_line.max(1);
    let max_lines = max_lines.clamp(1, MAX_READ_LINES);
    let lines = content.lines().collect::<Vec<_>>();
    let selected = lines
        .iter()
        .enumerate()
        .skip(start - 1)
        .take(max_lines)
        .map(|(index, line)| format!("{:>5}  {line}", index + 1))
        .collect::<Vec<_>>();
    let shown_end = (start - 1 + selected.len()).min(lines.len());
    let mut output = format!(
        "{relative} (lines {start}-{shown_end} of {})\n{}",
        lines.len(),
        selected.join("\n")
    );
    if shown_end < lines.len() {
        output.push_str(&format!(
            "\n[{} more lines; call again with startLine={}]",
            lines.len() - shown_end,
            shown_end + 1
        ));
    }
    Ok(output)
}

fn search_files(root: &Path, relative: &str, query: &str) -> Result<String, String> {
    let start = resolve_workspace_file_path(root, relative)?;
    if !start.is_dir() {
        return Err(format!("{relative} is not a directory."));
    }
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![start];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || name == APP_PRIVATE_DIR_NAME {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if !SKIPPED_DIRECTORIES.iter().any(|skipped| name == *skipped) {
                    stack.push(path);
                }
                continue;
            }
            files_seen += 1;
            if files_seen > MAX_SEARCH_FILES {
                matches.push(format!("[stopped after {MAX_SEARCH_FILES} files]"));
                return Ok(format_matches(query, matches));
            }
            if entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > MAX_SEARCH_FILE_BYTES {
                continue;
            }
            // Binary and non-UTF-8 files are skipped.
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let display = display_path(root, &path);
            for (index, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    let line = line.trim();
                    let line = if line.chars().count() > 240 {
                        format!("{}…", line.chars().take(240).collect::<String>())
                    } else {
                        line.to_string()
                    };
                    matches.push(format!("{display}:{}: {line}", index + 1));
                    if matches.len() >= MAX_SEARCH_MATCHES {
                        matches.push(format!("[stopped after {MAX_SEARCH_MATCHES} matches]"));
                        return Ok(format_matches(query, matches));
                    }
                }
            }
        }
    }
    Ok(format_matches(query, matches))
}

fn format_matches(query: &str, matches: Vec<String>) -> String {
    if matches.is_empty() {
        format!("No matches for \"{query}\".")
    } else {
        format!("Matches for \"{query}\":\n{}", matches.join("\n"))
    }
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &AgentTool, input: Value) -> String {
        match tool
            .execute(AgentToolCall {
                tool_id: tool.id.clone(),
                rationale: String::new(),
                input,
            })
            .unwrap()
        {
            AgentToolExecution::Completed(result) => result.observation,
            AgentToolExecution::Pending(_) => panic!("workspace tools never pause"),
        }
    }

    #[test]
    fn tools_list_read_and_search_inside_the_workspace_only() {
        let root = std::env::temp_dir().join(format!(
            "daily-workspace-tools-{}",
            crate::util::unique_stamp()
        ));
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::create_dir_all(root.join(APP_PRIVATE_DIR_NAME)).unwrap();
        fs::write(root.join("notes/plan.md"), "alpha\nNeedle here\ngamma\n").unwrap();
        fs::write(root.join(APP_PRIVATE_DIR_NAME).join("secret.txt"), "needle").unwrap();
        let root = root.canonicalize().unwrap();
        let tools = workspace_tools(&root);

        let listing = call(&tools[0], json!({}));
        assert!(listing.contains("notes/"));
        assert!(!listing.contains(APP_PRIVATE_DIR_NAME));

        let read = call(
            &tools[1],
            json!({ "path": "notes/plan.md", "startLine": 2, "maxLines": 1 }),
        );
        assert!(read.contains("Needle here"));
        assert!(read.contains("startLine=3"));
        assert!(call(&tools[1], json!({ "path": "../outside" })).starts_with("Error:"));
        assert!(call(&tools[1], json!({ "path": ".daily/secret.txt" })).starts_with("Error:"));

        let found = call(&tools[2], json!({ "query": "needle" }));
        assert!(found.contains("notes/plan.md:2: Needle here"));
        assert!(!found.contains("secret.txt"));

        let _ = fs::remove_dir_all(root);
    }
}
