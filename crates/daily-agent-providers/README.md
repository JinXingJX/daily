# Daily Agent Providers

HTTP transports for [daily-agent-core](../daily-agent-core): OpenAI-compatible Chat Completions, Responses API, Anthropic Messages, and Gemini. Existing native tool calls, streaming, opaque reasoning data, usage, typed errors, and transport retries are preserved.

`ModelConfig` contains resolved endpoint/model/credential parameters. Desktop authentication lists, settings precedence, and UI views remain in `daily-desktop-runtime`. Construct a `ProviderModel` to bind a transport and configuration to the core's `Model` interface. Hosts may supply a configured `reqwest::Client`, including their proxy and certificate policy.

```rust,ignore
use daily_agent_core::{AgentTask, run_agent_task};
use daily_agent_providers::{ModelConfig, ModelProviderKind, ProviderModel};
use std::sync::Arc;

let config = ModelConfig::for_provider(
    ModelProviderKind::OpenAiCompatible, endpoint, model_id, resolved_credential,
);
let model = ProviderModel::new(http_client, config);
let task = AgentTask::new("assistant", Arc::new(model), user_prompt);
let result = run_agent_task(task, |_| Ok(())).await?;
```

The `login` feature enables the unofficial Codex CLI and Antigravity login adapters, their credential discovery/refresh, and authentication status helpers. It is off by default; Daily desktop explicitly enables it. API-key Responses calls use the shared Responses protocol engine without compiling the CLI credential module in a default build. See [subscription adapters](../../docs/subscription-adapters.md) before enabling login adapters. The `ts` feature provides derives; shared exports are owned by the desktop protocol generator.

The repository's deterministic local simulation can exercise the SDK/provider path without real credentials:

```sh
postman simulation run postman/simulations/task-runtime/task-runtime.sim.yaml --port 4968 --no-history --output ndjson
```

In another terminal:

```sh
cargo run -p daily-agent-providers --example local_model -- http://127.0.0.1:4968/model/v1
```

Run standalone/default-feature tests separately from the desktop workspace, since Cargo feature unification enables `login` when the desktop is built:

```sh
cargo test -p daily-agent-core -p daily-agent-providers
cargo clippy -p daily-agent-core -p daily-agent-providers --all-targets -- -D warnings
node scripts/check-agent-sdk.mjs
```

`check:agent-sdk` runs an external Cargo consumer offline using cached dependencies, verifies that desktop/database dependencies and login features are absent, and executes the offline delegation example. Run the SDK tests first to populate the dependency cache.

Provider identities include the provider kind, model name and an endpoint fingerprint. Checkpoint restoration can rebind refreshed credentials, but rejects a different model or endpoint. Neither raw endpoint URLs nor credentials are stored in the identity.
