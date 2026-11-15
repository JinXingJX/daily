use std::fmt;

/// Error surfaced across the desktop protocol. The variant becomes a stable
/// machine-readable `code`; the message is shown to users unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    UnknownCommand(String),
    InvalidParams(String),
    NotFound(String),
    PluginDisabled(String),
    PayloadTooLarge(String),
    Unavailable(String),
    /// Errors not yet classified; kept as the historical `command_failed` code.
    Failed(String),
}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownCommand(_) => "unknown_command",
            Self::InvalidParams(_) => "invalid_params",
            Self::NotFound(_) => "not_found",
            Self::PluginDisabled(_) => "plugin_disabled",
            Self::PayloadTooLarge(_) => "payload_too_large",
            Self::Unavailable(_) => "unavailable",
            Self::Failed(_) => "command_failed",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::UnknownCommand(message)
            | Self::InvalidParams(message)
            | Self::NotFound(message)
            | Self::PluginDisabled(message)
            | Self::PayloadTooLarge(message)
            | Self::Unavailable(message)
            | Self::Failed(message) => message,
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for CoreError {}

impl From<String> for CoreError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for CoreError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_string())
    }
}

impl From<CoreError> for String {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::UnknownCommand(message)
            | CoreError::InvalidParams(message)
            | CoreError::NotFound(message)
            | CoreError::PluginDisabled(message)
            | CoreError::PayloadTooLarge(message)
            | CoreError::Unavailable(message)
            | CoreError::Failed(message) => message,
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn unclassified_errors_keep_the_legacy_code_and_message() {
        let error = CoreError::from("disk full".to_string());
        assert_eq!(error.code(), "command_failed");
        assert_eq!(String::from(error), "disk full");
    }
}
