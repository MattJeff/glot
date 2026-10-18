use serde::{Deserialize, Serialize};

use crate::Key;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub path: String,
    pub line: u32,
}

/// Everything we know about a key, fed to the LLM prompt and exposed through MCP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyContext {
    pub key: Key,
    /// Human note (glot.lock `notes`, server description).
    pub description: Option<String>,
    pub refs: Vec<SourceRef>,
    /// button | heading | label | placeholder | aria | title | link | toast | text
    pub ui_role: Option<String>,
    pub component: Option<String>,
    pub max_length: Option<usize>,
    /// Sibling keys (same prefix) with their source values.
    pub neighbors: Vec<(Key, String)>,
    pub screenshot: Option<String>,
}
