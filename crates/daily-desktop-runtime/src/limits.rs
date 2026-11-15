pub(crate) const APP_PRIVATE_DIR_NAME: &str = ".daily";
/// Chat turns that may run at once across all sessions of a workspace.
pub(crate) const MAX_CONCURRENT_TURNS: usize = 4;
pub(crate) const DEFAULT_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES: usize = 512 * 1024;
pub(crate) const FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const NETWORK_FETCH_RESPONSE_LIMIT_BYTES: usize = 2 * 1024 * 1024;
/// Largest plugin file returned as base64; stays under the 32 MiB protocol frame.
pub(crate) const MAX_PLUGIN_BINARY_READ_BYTES: u64 = 20 * 1024 * 1024;
pub(crate) const MAX_PLUGIN_FILE_COPY_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_IMAGE_INPUT_BYTES: u64 = 15 * 1024 * 1024;
pub(crate) const MAX_IMAGE_REFERENCES: usize = 16;
pub(crate) const MAX_IMAGE_RESPONSE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const PLUGIN_STORAGE_LIMIT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_DIRECTORY_ENTRIES: usize = 10_000;
pub(crate) const MAX_CONTEXT_ROOTS: usize = 5_000;
pub(crate) const MAX_NETWORK_REDIRECTS: usize = 5;
pub(crate) const MAX_TURN_TEXT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_TURN_IMAGES: usize = 4;
pub(crate) const MAX_CA_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_ASYNC_TOOL_RESULT_BYTES: usize = 512 * 1024;
pub(crate) const MAX_PLUGIN_PROMPT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PLUGIN_SYSTEM_PROMPT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PLUGIN_CONTEXT_BLOCKS: usize = 64;
pub(crate) const MAX_PLUGIN_HEADER_VALUE_BYTES: usize = 8 * 1024;
pub(crate) const MAX_PENDING_PLUGIN_RUN_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_PLUGIN_URL_BYTES: usize = 2_048;
pub(crate) const MAX_PLUGIN_AGENT_SESSION_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_PLUGIN_AGENT_SESSIONS: usize = 2_000;
/// Hard cap on model requests in one plugin agent task, across pauses.
pub(crate) const PLUGIN_AGENT_MAX_ITERATIONS: usize = 1_000;
pub(crate) const PLUGIN_AGENT_DEFAULT_ITERATIONS: usize = 8;
/// Replayed session history above this estimated token count is folded into an LLM summary.
pub(crate) const PLUGIN_SESSION_CONTEXT_BUDGET_TOKENS: usize = 48_000;
pub(crate) const PLUGIN_SESSION_KEEP_RECENT_ENTRIES: usize = 6;
/// In-run tool observations above this estimated token count are folded into an LLM summary.
pub(crate) const PLUGIN_RUN_OBSERVATION_BUDGET_TOKENS: usize = 64_000;
/// The same for the tool results of one chat turn.
pub(crate) const CHAT_OBSERVATION_BUDGET_TOKENS: usize = 48_000;
pub(crate) const CHAT_KEEP_RECENT_OBSERVATIONS: usize = 6;
/// Earlier chat turns replayed to the model, newest first, up to this estimated token count.
pub(crate) const CHAT_HISTORY_BUDGET_TOKENS: usize = 32_000;
/// Hard cap on model requests in one chat turn.
pub(crate) const CHAT_MAX_ITERATIONS: usize = 1_000;
/// Largest output budget a request may ask for. Adapters lower it to the model's own limit
/// when the provider rejects the value.
pub(crate) const MAX_OUTPUT_TOKENS: u32 = 128_000;
pub(crate) const PLUGIN_RUN_KEEP_RECENT_OBSERVATIONS: usize = 4;
