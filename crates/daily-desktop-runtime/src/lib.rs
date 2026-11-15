//! The Daily desktop backend, built on `daily-agent-core` and `daily-agent-providers`.
//!
//! Two entry points:
//!
//! - [`DailyRuntime`] is the whole desktop backend for one workspace: sessions, settings,
//!   plugins and the command table the app server exposes.
//! - [`sdk`] is the compatibility facade for existing desktop-configured agent tasks.
//!   New embedders should use `daily-agent-core` directly; its `multi_agent` example runs
//!   without HTTP, credentials, plugins, or a desktop workspace.

mod agent;
mod agent_checkpoints;
mod error;
mod fs_utils;
mod limits;
mod model;
mod net;
mod plugins;
mod runtime;
mod settings;
mod state;
mod tasks;
mod util;

#[cfg(all(test, feature = "ts"))]
mod protocol_exports;

pub use agent::sdk;
pub use error::{CoreError, CoreResult};
pub use runtime::{DailyRuntime, DESKTOP_COMMANDS};
