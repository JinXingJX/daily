//! Codex model catalog: reads the model list and defaults that the Codex CLI
//! keeps under `CODEX_HOME` (`models_cache.json` and `config.toml`).

use super::auth::codex_home_path;
use crate::fs_utils::read_limited_utf8;
use crate::types::DEFAULT_CODEX_REASONING_EFFORTS;
use serde::Deserialize;
use std::path::Path;

const CODEX_MODELS_CACHE_FILE: &str = "models_cache.json";
const CODEX_CONFIG_FILE: &str = "config.toml";
const MAX_CODEX_MODELS_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CODEX_CONFIG_BYTES: usize = 2 * 1024 * 1024;

/// Models and default selection derived from the local Codex CLI state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodexModelCatalog {
    /// Model ids in daily's `slug` / `slug-<effort>` form, ordered by Codex priority.
    pub models: Vec<String>,
    /// Default model id from `config.toml`, or the first listed model.
    pub default_model: Option<String>,
}

#[derive(Deserialize)]
struct ModelsCacheDocument {
    #[serde(default)]
    models: Vec<CachedModel>,
}

#[derive(Deserialize)]
struct CachedModel {
    slug: String,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    supported_reasoning_levels: Vec<CachedReasoningLevel>,
}

#[derive(Deserialize)]
struct CachedReasoningLevel {
    effort: String,
}

struct CatalogEntry {
    slug: String,
    efforts: Vec<String>,
}

#[derive(Default)]
struct CodexConfigDefaults {
    model: Option<String>,
    reasoning_effort: Option<String>,
}

/// Loads the Codex model catalog for `codex_home` (or the default Codex home).
/// Returns `None` when neither the models cache nor a configured model is available.
pub fn load_codex_model_catalog(codex_home: Option<&str>) -> Option<CodexModelCatalog> {
    let home = codex_home_path(codex_home).ok()?;
    let entries = read_models_cache(&home.join(CODEX_MODELS_CACHE_FILE)).unwrap_or_default();
    let defaults = read_config_defaults(&home.join(CODEX_CONFIG_FILE)).unwrap_or_default();
    build_catalog(entries, defaults)
}

fn build_catalog(
    entries: Vec<CatalogEntry>,
    defaults: CodexConfigDefaults,
) -> Option<CodexModelCatalog> {
    let mut models = Vec::new();
    for entry in &entries {
        push_unique(&mut models, entry.slug.clone());
        for effort in &entry.efforts {
            push_unique(&mut models, format!("{}-{effort}", entry.slug));
        }
    }

    let default_model = match defaults.model {
        Some(slug) => {
            let supported = entries
                .iter()
                .find(|entry| entry.slug == slug)
                .map(|entry| entry.efforts.as_slice());
            let effort = defaults.reasoning_effort.filter(|effort| {
                supported
                    .map(|efforts| efforts.contains(effort))
                    .unwrap_or_else(|| DEFAULT_CODEX_REASONING_EFFORTS.contains(&effort.as_str()))
            });
            Some(match effort {
                Some(effort) => format!("{slug}-{effort}"),
                None => slug,
            })
        }
        None => entries.first().map(|entry| entry.slug.clone()),
    };

    if models.is_empty() && default_model.is_none() {
        return None;
    }
    if let Some(default_model) = &default_model {
        if !models.contains(default_model) {
            models.insert(0, default_model.clone());
        }
    }
    Some(CodexModelCatalog {
        models,
        default_model,
    })
}

fn read_models_cache(path: &Path) -> Option<Vec<CatalogEntry>> {
    if !path.is_file() {
        return None;
    }
    let raw = read_limited_utf8(path, MAX_CODEX_MODELS_CACHE_BYTES, "Codex models cache").ok()?;
    parse_models_cache(&raw)
}

fn parse_models_cache(raw: &str) -> Option<Vec<CatalogEntry>> {
    let document = serde_json::from_str::<ModelsCacheDocument>(raw).ok()?;
    let mut models = document
        .models
        .into_iter()
        .filter(|model| !model.slug.trim().is_empty())
        .filter(|model| model.visibility.as_deref().unwrap_or("list") == "list")
        .collect::<Vec<_>>();
    // Stable sort keeps the server order for equal priorities.
    models.sort_by_key(|model| model.priority.unwrap_or(i64::MAX));
    Some(
        models
            .into_iter()
            .map(|model| CatalogEntry {
                slug: model.slug.trim().to_string(),
                // Only efforts the request builder can parse from a model suffix.
                efforts: DEFAULT_CODEX_REASONING_EFFORTS
                    .iter()
                    .filter(|effort| {
                        model
                            .supported_reasoning_levels
                            .iter()
                            .any(|level| level.effort == **effort)
                    })
                    .map(|effort| (*effort).to_string())
                    .collect(),
            })
            .collect(),
    )
}

fn read_config_defaults(path: &Path) -> Option<CodexConfigDefaults> {
    if !path.is_file() {
        return None;
    }
    let raw = read_limited_utf8(path, MAX_CODEX_CONFIG_BYTES, "Codex config").ok()?;
    Some(parse_config_defaults(&raw))
}

/// Reads the top-level `model` and `model_reasoning_effort` string keys.
/// Only the root table matters here, so parsing stops at the first table header.
fn parse_config_defaults(raw: &str) -> CodexConfigDefaults {
    let mut defaults = CodexConfigDefaults::default();
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_toml_string(value) else {
            continue;
        };
        match key.trim() {
            "model" => defaults.model = Some(value),
            "model_reasoning_effort" => defaults.reasoning_effort = Some(value),
            _ => {}
        }
    }
    defaults
}

fn parse_toml_string(value: &str) -> Option<String> {
    let value = value.trim();
    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = &value[1..];
    let end = rest.find(quote)?;
    let parsed = rest[..end].trim();
    (!parsed.is_empty()).then(|| parsed.to_string())
}

fn push_unique(models: &mut Vec<String>, model: String) {
    if !models.contains(&model) {
        models.push(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CACHE: &str = r#"{
      "fetched_at": "2026-10-08T00:00:00Z",
      "models": [
        {"slug": "beta", "visibility": "list", "priority": 2,
         "supported_reasoning_levels": [{"effort": "low"}, {"effort": "max"}]},
        {"slug": "hidden", "visibility": "hide", "priority": 0,
         "supported_reasoning_levels": [{"effort": "low"}]},
        {"slug": "alpha", "visibility": "list", "priority": 1,
         "supported_reasoning_levels": [{"effort": "medium"}, {"effort": "xhigh"}]}
      ]
    }"#;

    #[test]
    fn models_cache_keeps_listed_models_in_priority_order() {
        let entries = parse_models_cache(CACHE).unwrap();
        let catalog = build_catalog(entries, CodexConfigDefaults::default()).unwrap();

        assert_eq!(
            catalog.models,
            ["alpha", "alpha-medium", "alpha-xhigh", "beta", "beta-low"]
        );
        assert_eq!(catalog.default_model.as_deref(), Some("alpha"));
    }

    #[test]
    fn config_defaults_select_model_and_supported_effort() {
        let defaults = parse_config_defaults(
            "# comment\nmodel = \"beta\"\nmodel_reasoning_effort = 'low' # inline\n\n[projects.\"/x\"]\nmodel = \"ignored\"\n",
        );
        let catalog = build_catalog(parse_models_cache(CACHE).unwrap(), defaults).unwrap();

        assert_eq!(catalog.default_model.as_deref(), Some("beta-low"));
    }

    #[test]
    fn unsupported_effort_falls_back_to_bare_model() {
        let defaults = CodexConfigDefaults {
            model: Some("beta".into()),
            reasoning_effort: Some("xhigh".into()),
        };
        let catalog = build_catalog(parse_models_cache(CACHE).unwrap(), defaults).unwrap();

        assert_eq!(catalog.default_model.as_deref(), Some("beta"));
    }

    #[test]
    fn configured_model_missing_from_cache_is_prepended() {
        let defaults = CodexConfigDefaults {
            model: Some("custom".into()),
            reasoning_effort: Some("high".into()),
        };
        let catalog = build_catalog(Vec::new(), defaults).unwrap();

        assert_eq!(catalog.models, ["custom-high"]);
        assert_eq!(catalog.default_model.as_deref(), Some("custom-high"));
    }

    #[test]
    fn empty_sources_yield_no_catalog() {
        assert!(build_catalog(Vec::new(), CodexConfigDefaults::default()).is_none());
    }
}
