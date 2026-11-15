# Daily Agent Core

An embeddable Rust framework for multi-agent applications. Daily desktop is one host; a CLI or service can use the same execution engine without Electron, HTTP, credentials, SQLite, plugins, or a workspace directory.

```sh
cargo run -p daily-agent-core --example framework
cargo run -p daily-agent-core --example multi_agent
```

[framework.rs](examples/framework.rs) runs a dependency graph with distinct model bindings, then persists and resumes a managed async tool call. Both examples run offline.

## Tasks, definitions and bindings

`AgentTask::new(name, Arc<dyn Model>, prompt)` constructs a task. Supply tools, context, generation parameters and an optional `RunContext`, then call `run_agent_task` or `run_agent_task_until_pause`. The callback receives progress; returning an error stops execution.

`AgentDefinition` is a serializable specification: instructions, allowed tool IDs, model binding key, delegates, generation parameters and create/handoff flags. `Bindings` supplies executable models, tool closures and agent definitions at runtime. `Model` identifies itself with strings and owns its transport/retry policy; custom models need no built-in provider enum. [daily-agent-providers](../daily-agent-providers) supplies the HTTP implementations.

Register a root delegate with `task.register_agent(definition)`. Add deeper definitions to `task.agent_definitions` and explicitly list their IDs in the parent's `delegates`. Each child can select its own bound model and may only receive tools allowed by its parent. `subagent.<id>` returns the child's output to its caller. With `allow_handoff`, `agent.handoff` transfers execution to a declared delegate and returns its final output as the caller's answer. `agent.create` requires `allow_create` and validates the created definition. Async tool pauses retain the complete recursive delegation stack.

Native model tool calls retain their call IDs, reasoning items and paired tool results. Tools marked `parallel_safe` can run concurrently; other calls execute in order. Blocking tools must cooperate with `ToolCancel`; dropping a future cannot roll back external effects or terminate arbitrary blocking code.

## Supervision, cancellation and budgets

`RunSupervisor` owns managed root runs, with a default capacity of four (`with_capacity` configures it), event subscriptions, listing and cancellation. `RunHandle::wait` joins a run. Dropping the handle or its wait future cancels and aborts its worker. Dropping a direct task or workflow future drops active async work; there are no detached child agents.

`RunContext` shares limits, aggregate usage, IO concurrency and cancellation across descendants. Defaults are 256 logical model requests, 1024 tool calls, four concurrent model/tool IO operations and depth eight. `RunLimits` can also set an absolute deadline and a reported-token ceiling. In-run compaction requests count against the model budget. A provider's internal transport retries remain one logical request; reported-token limits stop subsequent calls after reported usage reaches the ceiling and cannot predict usage missing from a response. They are not a billing guarantee.

Parent cancellation reaches descendants; cancelling one child leaves its siblings running. Events have a run ID, parent ID, agent ID, depth and ordered sequence. `subscribe` offers live broadcast; `events_after` retains the latest 1024 events and flags truncation. Event replay is process-local. Checkpoint sequence numbers survive restoration, but full event history is not stored by this crate.

## Durable execution and explicit recovery

Attach `Arc<dyn CheckpointStore>` to a context or supervisor before execution. `MemoryCheckpointStore` is provided for tests and examples. Production hosts implement the async store contract with atomic compare-and-swap revisions. The desktop host supplies SQLite; core has no database dependency.

`RunCheckpoint` (version 1) records execution position, the pending model round and tool cursor, observations, recursive frames, limits, aggregate counters and a completed outcome. Models, credentials, transports and tool closures are excluded; transcripts and tool outputs remain application data that the host must protect. Rebind the original model/tool IDs before resuming. Provider bindings verify a model/endpoint identity while allowing credentials to change.

Load the latest checkpoint, then use `restore_tree` with its store (or `restore` for a self-contained non-delegating checkpoint):

- `Ready` accepts `ResumeAction::Continue`.
- `ModelInFlight` requires `RetryModel`: the host explicitly accepts a potentially repeated provider call.
- A replay-safe `ToolInFlight` accepts `RetrySafeTool`.
- `WaitingInput` or an uncertain external tool requires `ToolResult` with the original call ID and a resolved result.
- `Completed` is retrieved with `completed_result`; it cannot be executed again.

Persisted children are reconciled before a parent continues. Completed calls and completed children are reused. A fresh context cannot overwrite an existing run ID. No framework can guarantee exactly-once external effects after a crash between dispatch and result persistence; hosts must resolve uncertain operations or use idempotent external operation keys.

## Serial, parallel and DAG workflows

`Workflow::serial`, `Workflow::parallel`, or explicit `WorkflowNode.dependencies` schedule up to 256 nodes. Each ready node receives completed dependency outputs as context. Cycles and missing bindings are rejected before dispatch. Concurrency and budgets are shared; `FailFast` cancels active siblings, while `Collect` keeps independent nodes running and marks dependents blocked.

`WorkflowState` records the graph, node results, limits, budgets and revision. Stores used for durable workflows implement `load_workflow` and `save_workflow` as well as agent checkpoints; the memory and desktop adapters support both. A node's stable run ID is persisted before dispatch. Recovery reconciles it with the child checkpoint, including a child completed just before a graph write failed. Resume a waiting/failed node through `state.resume_node(...)` with an explicit action, then continue the graph with `workflow.run(&bindings, state.execution(store), Some(state))`. Uncertain nodes require that decision; unrelated completed nodes are retained. Dropping an in-flight graph leaves the last stored state available for reconciliation.

## Validation and distribution

```sh
cargo test -p daily-agent-core
cargo clippy -p daily-agent-core --all-targets -- -D warnings
node scripts/check-agent-sdk.mjs
```

From the desktop repository, `npm run package:agent-sdk` produces a source archive containing only the two SDK crates and validates it as a separate Cargo workspace. The `ts` feature enables shared TypeScript derives without automatic file exports.
