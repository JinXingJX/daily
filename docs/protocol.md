# Sidecar protocol (v1)

Electron talks to `daily-app-server` over the child process's stdin and stdout. Each frame is one line of UTF-8 JSON, at most 32 MiB in either direction. Logs go to stderr.

The sidecar is started as:

```bash
daily-app-server --project-root /absolute/path/to/workspace
```

## Frames from the sidecar

### `ready`

Sent once after the workspace loads. Electron rejects the sidecar if the version or root differs from what it expects.

```json
{ "kind": "ready", "protocolVersion": 1, "workspaceRoot": "/Users/me/project" }
```

### `response`

Answers one request, matched by `id`.

```json
{ "kind": "response", "id": "7", "ok": true, "result": { } }
{ "kind": "response", "id": "8", "ok": false, "error": { "code": "invalid_params", "message": "…" } }
```

### `event`

Pushed at any time.

| `event` | Payload |
| --- | --- |
| `app://state` | `{ "kind": "snapshot", "snapshot": AppSnapshot }` or `{ "kind": "turn_delta", "delta": TurnDelta }` |
| `plugin://agent-progress` | Event view patches for a plugin task, tagged with the plugin, view, and `streamId` |

### `fatal`

The sidecar cannot continue (for example the workspace failed to load). Electron shows the message and offers a restart.

```json
{ "kind": "fatal", "error": { "code": "startup_failed", "message": "…" } }
```

## Frames to the sidecar

```json
{ "kind": "request", "protocolVersion": 1, "id": "7", "method": "open_session", "params": { "input": { "sessionId": 3 } } }
```

`params.input` holds the command's input; the TypeScript type for each input and result is generated into `src/generated/protocol/`. Requests are handled concurrently.

## Commands

The allowlist is generated from the `desktop_commands!` table in `crates/daily-core/src/runtime/mod.rs` into `electron/shared/commands.generated.ts`. Electron forwards only these names.

| Group | Commands |
| --- | --- |
| State and settings | `get_app_snapshot`, `reload_settings`, `save_settings_file_content`, `save_auth_settings_content`, `delete_auth_settings_content` |
| Codex login | `get_codex_auth_status`, `start_codex_login`, `save_codex_auth_settings_content` |
| Antigravity login | `get_antigravity_auth_status`, `start_antigravity_login`, `get_antigravity_model_catalog`, `refresh_antigravity_models` |
| Workspace tree | `create_workspace_node`, `rename_workspace_node`, `move_workspace_node`, `delete_workspace_node`, `open_session` |
| Agents | `create_agent_app`, `update_agent_app`, `delete_agent_app`, `bind_session_agent`, `list_context_roots` |
| Plugins | `list_plugins`, `install_plugin`, `install_local_dev_plugin`, `uninstall_plugin`, `enable_plugin`, `disable_plugin`, `save_plugin_settings`, `reload_plugins`, `plugin_view_asset`, `plugin_host_rpc`, `plugin_rpc_result` |
| Turns | `run_session_turn`, `resume_run`, `retry_session_turn`, `continue_session_turn`, `stop_session_turn` |

Commands that change state return the new `AppSnapshot` and also broadcast it as an `app://state` event. Turn commands return as soon as the turn is queued; progress arrives as `turn_delta` events.

`plugin_host_rpc` carries a plugin call: `{ "pluginId", "viewId", "method", "payload" }`. Methods are listed in `PLUGIN_RPC_METHODS` (`crates/daily-core/src/plugins/rpc/mod.rs`, generated into `packages/plugin-sdk/src/methods.generated.ts`).

## Error codes

| Code | Meaning |
| --- | --- |
| `unknown_command` | The method is not a desktop command |
| `invalid_params` | The input did not match the command's schema |
| `not_found` | A referenced session, agent, plugin, view, or RPC method does not exist |
| `plugin_disabled` | The plugin is installed but disabled |
| `payload_too_large` | A request or plugin payload exceeds its limit |
| `unavailable` | Runtime state is temporarily unavailable |
| `command_failed` | Any other failure; the message explains it |
| `invalid_request` | The frame was not valid JSON or not a v1 request |
| `response_too_large` | The result would exceed the frame limit |
| `startup_failed` | Sent in a `fatal` frame when the sidecar cannot start |

Electron adds `command_not_allowed` for names outside the allowlist and `desktop_error` for transport failures.

## Timeouts

Electron gives most requests 30 seconds. Plugin `model.invoke`, `agent.run`, and `agent.task.start/resume/retry` have no client deadline because they resolve only when the run ends; the sidecar bounds each model request, plugins can stop or cancel runs, and a sidecar exit rejects all pending requests.
