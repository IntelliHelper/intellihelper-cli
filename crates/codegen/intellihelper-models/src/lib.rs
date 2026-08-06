//! Default model IDs loaded from `default_models.json` at runtime.
//! Edit that JSON file to change them.
//!
//! There is **no** first-party hosted model catalog yet: `models` is empty and
//! `default` may be `""`. Users configure models via `~/.intellihelper/config.toml`
//! (`[model.*]` + `[models] default = "..."`). When a first-party catalog ships,
//! fill `models` and set `default` to a listed id.
//!
//! At runtime each model is resolved via:
//!   CLI flag > ENV var > config.toml > remote settings > these defaults

use std::sync::LazyLock;

/// The raw JSON, embedded at compile time. Re-exported through the
/// `intellihelper_shell::models` facade and consumed by `agent::config`, so it must
/// be `pub` (was `pub(crate)` when this lived inside the shell crate).
pub const DEFAULT_MODELS_JSON: &str = include_str!("../default_models.json");

#[derive(serde::Deserialize)]
struct DefaultModels {
    default: String,
    /// Falls back to `default` if not specified in JSON.
    web_search: Option<String>,
    /// Falls back to `default` if not specified in JSON.
    image_description: Option<String>,
    /// Falls back to `default` if not specified in JSON.
    session_summary: Option<String>,
    models: Vec<DefaultModelEntry>,
}

#[derive(serde::Deserialize)]
struct DefaultModelEntry {
    model: String,
}

static DEFAULTS: LazyLock<DefaultModels> = LazyLock::new(|| {
    let defaults: DefaultModels = serde_json::from_str(DEFAULT_MODELS_JSON)
        .expect("default_models.json: invalid JSON or missing 'default' field");

    // When a catalog is present, `default` must refer to an entry. An empty
    // `default` is allowed when `models` is empty (no first-party models yet).
    if !defaults.default.is_empty() {
        let model_ids: Vec<&str> = defaults.models.iter().map(|m| m.model.as_str()).collect();
        assert!(
            model_ids.contains(&defaults.default.as_str()),
            "default_models.json: 'default' is '{}' but 'models' array only has {model_ids:?}",
            defaults.default,
        );
    } else {
        assert!(
            defaults.models.is_empty(),
            "default_models.json: empty 'default' requires an empty 'models' array \
             (got {} entries)",
            defaults.models.len(),
        );
    }

    defaults
});

/// Primary model for coding tasks and general fallback.
///
/// Empty string when no first-party default is configured — callers should
/// prefer user/`config.toml` model selection.
pub fn default_model() -> &'static str {
    &DEFAULTS.default
}

/// Model for web search tool synthesis. Falls back to default model.
/// Empty when unset and default is empty.
pub fn default_web_search_model() -> &'static str {
    match DEFAULTS.web_search.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => &DEFAULTS.default,
    }
}

/// Model for image describe. Falls back to default model.
/// Empty when unset and default is empty.
pub fn default_image_description_model() -> &'static str {
    match DEFAULTS.image_description.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => &DEFAULTS.default,
    }
}

/// Model for session title generation. Falls back to default model.
/// Empty when unset and default is empty.
pub fn default_session_summary_model() -> &'static str {
    match DEFAULTS.session_summary.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => &DEFAULTS.default,
    }
}
