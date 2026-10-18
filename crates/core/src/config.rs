use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{Error, Result, catalog::Format, validate::Syntax};

pub const CONFIG_FILE: &str = "glot.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub source_locale: String,
    pub target_locales: Vec<String>,
    /// Path patterns relative to the project root, with a `{locale}` placeholder.
    pub files: Vec<String>,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub syntax: Syntax,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub qa: QaConfig,
    #[serde(default)]
    pub extract: ExtractConfig,
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// anthropic | openai | openai-compatible | mock
    pub provider: String,
    pub model: String,
    pub fallback_model: Option<String>,
    pub judge_model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub budget_usd: Option<f64>,
    pub style: Option<String>,
    /// Max keys per LLM call.
    pub batch_size: usize,
    /// Provider of `fallback_model` when it differs from `provider` (e.g. `local` first pass, `anthropic` escalation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_api_key_env: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: "claude-haiku-5-5".into(),
            fallback_model: Some("claude-sonnet-5-5".into()),
            judge_model: None,
            api_key_env: None,
            base_url: None,
            budget_usd: None,
            style: None,
            batch_size: 40,
            fallback_provider: None,
            fallback_base_url: None,
            fallback_api_key_env: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QaConfig {
    pub score_threshold: u8,
    pub max_length_ratio: f32,
    /// Written translations whose quality score is below this go to the review queue (`glot queue`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_threshold: Option<u8>,
    /// Risk tag -> key glob patterns always routed to human review, e.g. `legal = ["legal.*", "terms.*"]`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub risk: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for QaConfig {
    fn default() -> Self {
        Self {
            score_threshold: 80,
            max_length_ratio: 1.6,
            review_threshold: None,
            risk: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Translation function names to detect (`t`, `useTranslations`, `i18n.t`, ...).
    pub functions: Vec<String>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            include: vec![
                "src/**/*.{ts,tsx,js,jsx}".into(),
                "lib/**/*.{ts,tsx,js,jsx}".into(),
                "hooks/**/*.{ts,tsx,js,jsx}".into(),
                "app/**/*.{ts,tsx,js,jsx}".into(),
                "components/**/*.{ts,tsx,js,jsx}".into(),
            ],
            exclude: vec![
                "**/node_modules/**".into(),
                "**/*.test.*".into(),
                "**/*.spec.*".into(),
            ],
            functions: vec![
                "t".into(),
                "useTranslations".into(),
                "getTranslations".into(),
                "i18n.t".into(),
                "$t".into(),
                "formatMessage".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
    pub project: String,
    #[serde(default = "default_token_env")]
    pub token_env: String,
}

fn default_token_env() -> String {
    "GLOT_TOKEN".into()
}

impl Config {
    /// Load `glot.toml` from `root`.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&text).map_err(|e| Error::Parse {
            path,
            message: e.to_string(),
        })
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        write_file(&root.join(CONFIG_FILE), &text)
    }

    /// Walk up from `start` until a directory containing `glot.toml` is found.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .find(|d| d.join(CONFIG_FILE).is_file())
            .map(Path::to_path_buf)
    }

    /// Resolve every catalog file for `locale` (each `files` pattern with `{locale}` replaced).
    /// Patterns containing glob wildcards expand to the matching existing files (sorted).
    pub fn catalog_paths(&self, root: &Path, locale: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for pattern in &self.files {
            let pattern = android_values(pattern, locale, &self.source_locale);
            let path = root.join(pattern.replace("{locale}", locale));
            let s = path.to_string_lossy();
            if s.contains(['*', '?', '[']) {
                let mut found: Vec<PathBuf> =
                    glob::glob(&s).into_iter().flatten().flatten().collect();
                found.sort();
                out.extend(found);
            } else {
                out.push(path);
            }
        }
        out
    }

    /// source_locale followed by target_locales.
    pub fn all_locales(&self) -> Vec<String> {
        std::iter::once(&self.source_locale)
            .chain(&self.target_locales)
            .cloned()
            .collect()
    }
}

/// Android resource folders: `values-{locale}` is `values` for the source locale and a resource
/// qualifier otherwise (`pt-BR` => `values-pt-rBR`, `sr-Latn` => `values-b+sr+Latn`).
fn android_values(pattern: &str, locale: &str, source: &str) -> String {
    if !pattern.contains("values-{locale}") {
        return pattern.to_string();
    }
    let dir = if locale == source {
        "values".to_string()
    } else {
        format!("values-{}", android_qualifier(locale))
    };
    pattern.replace("values-{locale}", &dir)
}

/// BCP 47 tag => Android resource qualifier (`fr`, `pt-rBR`, `b+sr+Latn`).
pub fn android_qualifier(locale: &str) -> String {
    let parts: Vec<&str> = locale.split(['-', '_']).collect();
    match parts.as_slice() {
        [l] => l.to_string(),
        [l, r] if r.len() == 2 || (r.len() == 3 && r.chars().all(|c| c.is_ascii_digit())) => {
            format!("{l}-r{}", r.to_uppercase())
        }
        _ => format!("b+{}", parts.join("+")),
    }
}

/// Write `text` to `path`, creating parent directories.
pub(crate) fn write_file(path: &Path, text: &str) -> Result<()> {
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(path, text).map_err(io)
}

/// Read a file, `None` if it does not exist.
pub(crate) fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_minimal_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!("glot-config-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            "source_locale = \"en\"\ntarget_locales = [\"fr\", \"ja\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"m\"\n[qa]\nscore_threshold = 70\n",
        )
        .unwrap();
        let c = Config::load(&dir).unwrap();
        assert_eq!(c.llm.batch_size, 40);
        assert_eq!(c.qa.max_length_ratio, 1.6);
        assert_eq!(c.all_locales(), ["en", "fr", "ja"]);
        assert_eq!(c.catalog_paths(&dir, "fr"), [dir.join("messages/fr.json")]);
        assert_eq!(Config::find_root(&dir.join("a/b")), Some(dir.clone()));
        c.save(&dir).unwrap();
        assert_eq!(Config::load(&dir).unwrap().llm.provider, "mock");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
