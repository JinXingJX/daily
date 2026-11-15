//! Provider-neutral model contract. Implement `Model` to embed any model or transport.
mod error;
mod types;
pub use error::{ModelError, ModelErrorKind};
pub use types::*;

/// Identifies a bound model without restricting providers to a built-in registry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelIdentity {
    pub provider: String,
    pub model: String,
}

/// Receives streamed text. Returning an error asks the model to stop.
pub type StreamCallback<'a> = dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send + 'a;

/// A model already bound to its transport and credentials by the embedding application.
/// Implementations own retry policy and must not replay output already streamed to callers.
#[async_trait::async_trait]
pub trait Model: Send + Sync {
    fn identity(&self) -> ModelIdentity;
    fn supports_images(&self) -> bool {
        false
    }
    fn supports_streaming(&self) -> bool {
        false
    }
    async fn complete(
        &self,
        request: &ModelCompletionRequest,
        on_chunk: Option<
            &mut (dyn FnMut(crate::model::ModelStreamChunk) -> Result<(), String> + Send),
        >,
    ) -> Result<ModelCompletionResponse, ModelError>;
}
