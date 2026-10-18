//! glot-llm: LLM providers, guarded translation pipeline, judge, pricing, and the local `engine::Workspace`.
pub mod engine;
pub mod judge;
pub mod mock;
pub mod pricing;
pub mod provider;
pub mod translate;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing API key: set {0}")]
    MissingApiKey(String),
    #[error("provider error ({provider}): {message}")]
    Provider { provider: String, message: String },
    #[error("budget exceeded: spent ${spent:.4} of ${budget:.2}")]
    Budget { spent: f64, budget: f64 },
    #[error(transparent)]
    Core(#[from] glot_core::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
