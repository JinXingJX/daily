# Durable plugin tasks

Task execution belongs to a workspace, not to a page. The Rust sidecar owns a
SQLite database (`.daily/tasks.sqlite3`); plugin handlers start from `activate(host)` in the app renderer. Switching views does not stop the plugin.

## Contract

- `tasks.submit({kind, scope, requestKey, input, exclusiveKey?})` commits a queued
  task before acknowledging it. Reusing a request key with identical parameters
  returns the same task; conflicting parameters are rejected.
- `tasks.get({taskId})` and `tasks.list({scope?, kind?})` return authoritative
  snapshots. `tasks.events({after, limit?})` returns ordered, durable changes and
  a cursor. Consumers replay from the last cursor; live notifications are hints.
- Activated plugins claim registered kinds using `tasks.claim({kinds})`. A claim
  has a fresh attempt id and lease token. Only that claim may heartbeat, publish
  progress/checkpoints, or finish the attempt.
- `tasks.update` saves progress and checkpoints. `tasks.finish` atomically saves
  the result and changes status to `completed`, `waiting_input`, or `failed`.
- `tasks.cancel` requests cancellation; removing a view never cancels a task.
  `tasks.retry` explicitly queues a failed/interrupted/cancelled task; `tasks.resume`
  queues a task waiting for input. Checkpoints survive both operations.
- `tasks.call` journals a model/image/agent operation under `(taskId, operationKey)`
  before running it in the sidecar. Reattaching to the same operation returns its
  recorded result. An operation interrupted by a sidecar restart is never blindly
  replayed: its external outcome may be unknown.

Only enabled plugins can access their own tasks. Worker mutations require a
valid plugin identity and an unexpired lease. Input, checkpoints and operation results
are bounded. Snapshots and their event cursor are committed in the same database
transaction. An expired lease or runtime restart marks a running attempt
`interrupted`; it does not pretend that an external side effect was rolled back.

## Lifecycle

Each workspace has one renderer. Plugins start task handlers from activation, independently of view mounting and route visibility. Switching views retains the UI and leaves tasks running. Reloading or crashing the renderer stops its handlers; unfinished leases expire as `interrupted`. On reopening, the user can explicitly continue from the saved checkpoint. Journaled sidecar operations retain their results without replaying completed external calls. Closing a workspace or quitting ends its processes; durable tasks remain for the next launch.

Task snapshots are separate from view preferences (draft text, selected tab,
scroll position). Keeping a view alive is optional and never required for task
correctness. Plugin handlers must persist domain results before completing and
make finalization idempotent, since interruption may occur between domain writes
and the task completion transaction.

## Agent SDK checkpoints

The generic multi-agent engine has a separate `CheckpointStore` contract. Daily implements it in `crates/daily-desktop-runtime/src/agent_checkpoints.rs`, backed by `.daily/agent-runs.sqlite3`. It stores versioned agent and workflow snapshots with revision fencing. Chat and plugin checkpoints reference a run ID; credentials and tool closures are rebound by the host. Completed tool calls and child results are retained; interrupted external tools require explicit resolution. This store does not replace the task queue, its leases, operation journal or durable event cursor. See the [SDK recovery contract](../crates/daily-agent-core/README.md#durable-execution-and-explicit-recovery).

## Verification

Run the normal workspace/renderer/Electron tests and production builds. The
optional macOS Electron scenario also needs the sibling Writer plugin and the
Postman CLI. It creates a temporary workspace and profile and uses a dummy
credential only against a local stand-in:

```sh
npm run backend:build
npm run build
postman simulation run postman/simulations/task-runtime/task-runtime.sim.yaml --port 4968 --no-history --output ndjson
```

In another terminal, run `npm run test:task-runtime`. It switches away from a running Writer task, verifies retained view state and completion, reloads the app renderer, then continues an interrupted attempt from its checkpoint after its lease expires. The scenario checks that recovery adds no duplicate model calls or conversation messages and writes a task-center screenshot and Electron log to a temporary directory. `DAILY_TASK_QA_MODEL_URL` overrides the stand-in's base URL. This exercises the app against the local stand-in, not a real provider.
