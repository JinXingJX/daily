use crate::fs_utils::sandbox::resolve_workspace_file_path;
use crate::limits::{
    APP_PRIVATE_DIR_NAME, DEFAULT_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
    FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
};
use crate::net::ssrf::{is_blocked_network_ip, validate_plugin_network_url};
use crate::plugins::rpc::plugin_rpc_payload_limit;
use crate::plugins::rpc::storage::{plugin_storage_compare_and_swap, read_plugin_storage};
use crate::plugins::rpc::workspace::plugin_workspace_list;
use crate::util::unique_stamp;
use reqwest::Url;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

#[test]
fn workspace_paths_reject_app_private_state() {
    let root = Path::new("/tmp/daily-workspace-root");

    assert!(resolve_workspace_file_path(root, ".daily/plugins.json").is_err());
    assert!(resolve_workspace_file_path(root, "./.daily/plugin-storage").is_err());
    assert!(resolve_workspace_file_path(root, "notes/.daily/draft.md").is_ok());
}

#[test]
fn workspace_list_hides_app_private_directory() {
    let root = std::env::temp_dir().join(format!("daily-workspace-list-test-{}", unique_stamp()));
    fs::create_dir_all(root.join(APP_PRIVATE_DIR_NAME)).unwrap();
    fs::write(root.join("note.txt"), "visible").unwrap();

    let result = plugin_workspace_list(&root, json!({ "path": "." })).unwrap();
    let entries = result
        .get("entries")
        .and_then(Value::as_array)
        .expect("workspace list returns entries");

    assert!(entries
        .iter()
        .all(|entry| entry.get("name").and_then(Value::as_str) != Some(APP_PRIVATE_DIR_NAME)));
    assert!(entries
        .iter()
        .any(|entry| entry.get("name").and_then(Value::as_str) == Some("note.txt")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_write_rpc_payloads_use_larger_limit() {
    let payload = json!({
        "path": "large-draft.json",
        "content": "x".repeat(DEFAULT_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES),
    });
    let serialized_len = serde_json::to_string(&payload).unwrap().len();

    assert!(serialized_len > plugin_rpc_payload_limit("agent.run"));
    assert!(serialized_len < plugin_rpc_payload_limit("plugin.files.writeText"));
    assert_eq!(
        plugin_rpc_payload_limit("workspace.writeText"),
        FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES
    );
}

#[test]
fn plugin_storage_compare_and_swap_is_atomic_at_the_storage_boundary() {
    let root = std::env::temp_dir().join(format!("daily-storage-cas-{}", unique_stamp()));
    fs::create_dir_all(&root).unwrap();
    let first_lock = json!({ "owner": "first", "expiresAt": 10 });
    let second_lock = json!({ "owner": "second", "expiresAt": 10 });

    let first = plugin_storage_compare_and_swap(
        &root,
        "test.plugin",
        json!({ "key": "lock", "expected": null, "value": first_lock }),
    )
    .unwrap();
    assert_eq!(first.get("swapped"), Some(&Value::Bool(true)));

    let second = plugin_storage_compare_and_swap(
        &root,
        "test.plugin",
        json!({ "key": "lock", "expected": null, "value": second_lock }),
    )
    .unwrap();
    assert_eq!(second.get("swapped"), Some(&Value::Bool(false)));
    assert_eq!(second.get("value"), Some(&first_lock));

    let removed = plugin_storage_compare_and_swap(
        &root,
        "test.plugin",
        json!({ "key": "lock", "expected": first_lock, "remove": true }),
    )
    .unwrap();
    assert_eq!(removed.get("swapped"), Some(&Value::Bool(true)));
    assert!(read_plugin_storage(&root, "test.plugin")
        .unwrap()
        .is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn plugin_network_urls_reject_local_targets_and_credentials() {
    for raw in [
        "http://127.0.0.1:3000/private",
        "http://[::1]/private",
        "http://[::ffff:127.0.0.1]/private",
        "http://[64:ff9b::7f00:1]/private",
        "http://100.64.0.1/private",
        "http://169.254.169.254/latest/meta-data",
        "https://service.local/data",
        "https://user:secret@example.com/data",
    ] {
        let url = Url::parse(raw).unwrap();
        assert!(
            validate_plugin_network_url(&url).is_err(),
            "{raw} should be rejected"
        );
    }
    assert!(validate_plugin_network_url(
        &Url::parse("https://news.google.com/rss/search?q=markets").unwrap()
    )
    .is_ok());
    assert!(is_blocked_network_ip("2002:7f00:0001::".parse().unwrap()));
    assert!(!is_blocked_network_ip(
        "2606:4700:4700::1111".parse().unwrap()
    ));
}

#[cfg(unix)]
#[test]
fn workspace_paths_reject_symbolic_link_escape() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("daily-symlink-test-{}", unique_stamp()));
    let outside = std::env::temp_dir().join(format!("daily-symlink-outside-{}", unique_stamp()));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("escape")).unwrap();

    assert!(resolve_workspace_file_path(&root, "escape/secret.txt").is_err());

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn rpc_specs_lock_only_storage_methods() {
    use crate::plugins::rpc::rpc_method_spec;

    assert!(rpc_method_spec("storage.set").unwrap().storage_lock);
    assert!(
        rpc_method_spec("storage.compareAndSwap")
            .unwrap()
            .storage_lock
    );
    for method in [
        "plugin.files.writeText",
        "workspace.writeText",
        "agent.task.cancel",
        "agent.session.create",
        "network.fetch",
    ] {
        assert!(!rpc_method_spec(method).unwrap().storage_lock, "{method}");
    }
    assert!(rpc_method_spec("process.spawn").is_none());
}
