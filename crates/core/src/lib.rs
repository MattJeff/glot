//! glot-core: pure, deterministic building blocks. No network, no async.
pub mod catalog;
pub mod config;
pub mod context;
pub mod formats;
pub mod glossary;
pub mod hash;
pub mod lock;
pub mod markets;
pub mod measure;
pub mod plural;
pub mod policy;
pub mod render;
pub mod validate;
pub mod variants;
pub use validate::branches;

pub type Locale = String;
pub type Key = String;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error on {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("parse error in {path}: {message}")]
    Parse {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
