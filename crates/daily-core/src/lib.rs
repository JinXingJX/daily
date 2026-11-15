//! Compatibility entry point. New agent applications should use `daily-agent-core` and
//! `daily-agent-providers`; desktop hosts use `daily-desktop-runtime`.
pub use daily_desktop_runtime::{sdk, CoreError, CoreResult, DailyRuntime, DESKTOP_COMMANDS};
