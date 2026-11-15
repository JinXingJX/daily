//! Typed model errors, so callers can tell a rate limit from a bad key from a full context
//! window and decide whether to retry, shrink the context or stop.
use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelErrorKind {
    /// The credential is missing, invalid or not allowed to use the model.
    Auth,
    RateLimited,
    /// The provider is temporarily out of capacity.
    Overloaded,
    /// The request does not fit the model's context window.
    ContextOverflow,
    /// The provider rejected the request; retrying the same body cannot succeed.
    InvalidRequest,
    Timeout,
    /// The connection failed or broke.
    Network,
    /// The provider failed (5xx).
    Server,
    /// The model declined to answer.
    Refused,
    /// The caller's stream callback stopped the request.
    Aborted,
    /// The provider's response could not be understood.
    Protocol,
    Other,
}

#[derive(Clone, Debug)]
pub struct ModelError {
    pub kind: ModelErrorKind,
    /// Shown to users unchanged.
    pub message: String,
    pub status: Option<u16>,
    /// How long the provider asked to wait before retrying.
    pub retry_after: Option<Duration>,
}

impl ModelError {
    pub fn new(kind: ModelErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
            retry_after: None,
        }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ModelErrorKind::Protocol, message)
    }

    pub fn aborted(message: impl Into<String>) -> Self {
        Self::new(ModelErrorKind::Aborted, message)
    }

    /// Stable machine-readable name of the kind.
    pub fn code(&self) -> &'static str {
        match self.kind {
            ModelErrorKind::Auth => "model_auth",
            ModelErrorKind::RateLimited => "model_rate_limited",
            ModelErrorKind::Overloaded => "model_overloaded",
            ModelErrorKind::ContextOverflow => "model_context_overflow",
            ModelErrorKind::InvalidRequest => "model_invalid_request",
            ModelErrorKind::Timeout => "model_timeout",
            ModelErrorKind::Network => "model_network",
            ModelErrorKind::Server => "model_server_error",
            ModelErrorKind::Refused => "model_refused",
            ModelErrorKind::Aborted => "aborted",
            ModelErrorKind::Protocol => "model_protocol",
            ModelErrorKind::Other => "model_error",
        }
    }

    /// Whether sending the same request again can succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ModelErrorKind::RateLimited
                | ModelErrorKind::Overloaded
                | ModelErrorKind::Timeout
                | ModelErrorKind::Network
                | ModelErrorKind::Server
        )
    }

    /// Classifies a non-success HTTP response. `label` names the provider in the message.
    pub fn from_status(
        label: &str,
        status: u16,
        body: &str,
        retry_after: Option<Duration>,
    ) -> Self {
        let lower = body.to_ascii_lowercase();
        let context_overflow = [
            "context_length_exceeded",
            "context length",
            "context window",
            "prompt is too long",
            "too many tokens",
            "exceeds the maximum number of tokens",
            "input token count",
            "maximum prompt length",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        let kind = match status {
            _ if context_overflow && matches!(status, 400 | 413 | 422) => {
                ModelErrorKind::ContextOverflow
            }
            401 | 403 => ModelErrorKind::Auth,
            408 => ModelErrorKind::Timeout,
            413 => ModelErrorKind::ContextOverflow,
            429 => ModelErrorKind::RateLimited,
            503 | 529 => ModelErrorKind::Overloaded,
            500..=599 => ModelErrorKind::Server,
            400..=499 => ModelErrorKind::InvalidRequest,
            _ => ModelErrorKind::Other,
        };
        Self {
            kind,
            message: format!("{label} request failed with status {status}: {body}"),
            status: Some(status),
            retry_after,
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

impl From<ModelError> for String {
    fn from(error: ModelError) -> Self {
        error.message
    }
}

impl From<String> for ModelError {
    fn from(message: String) -> Self {
        Self::new(ModelErrorKind::Other, message)
    }
}

impl From<&str> for ModelError {
    fn from(message: &str) -> Self {
        Self::new(ModelErrorKind::Other, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_kinds_and_retryability() {
        let kind = |status, body| ModelError::from_status("Test", status, body, None).kind;
        assert_eq!(kind(401, ""), ModelErrorKind::Auth);
        assert_eq!(kind(429, ""), ModelErrorKind::RateLimited);
        assert_eq!(kind(529, ""), ModelErrorKind::Overloaded);
        assert_eq!(kind(500, ""), ModelErrorKind::Server);
        assert_eq!(kind(400, "bad field"), ModelErrorKind::InvalidRequest);
        assert_eq!(
            kind(400, "{\"error\":{\"code\":\"context_length_exceeded\"}}"),
            ModelErrorKind::ContextOverflow
        );
        assert_eq!(
            kind(400, "prompt is too long: 250000 tokens > 200000 maximum"),
            ModelErrorKind::ContextOverflow
        );
        assert!(ModelError::from_status("Test", 429, "", None).is_retryable());
        assert!(!ModelError::from_status("Test", 400, "", None).is_retryable());
        assert!(!ModelError::aborted("Stopped.").is_retryable());
        let error = ModelError::from_status("Test", 401, "no key", None);
        assert_eq!(error.code(), "model_auth");
        assert_eq!(
            String::from(error),
            "Test request failed with status 401: no key"
        );
    }
}
