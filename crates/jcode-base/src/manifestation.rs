use serde::Deserialize;
use std::path::PathBuf;

pub const MANIFESTATIONS_FILE: &str = "MANIFESTATIONS.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifestation {
    pub name: String,
    pub character: String,
}

#[derive(Debug, Deserialize)]
struct ManifestationLedger {
    #[serde(default)]
    manifestations: Vec<ManifestationEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestationEntry {
    name: String,
    character: String,
    #[serde(default)]
    provider_keys: Vec<String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    model_prefixes: Vec<String>,
}

impl ManifestationEntry {
    fn matches(&self, provider_key: Option<&str>, model: &str) -> bool {
        if self.provider_keys.is_empty() && self.models.is_empty() && self.model_prefixes.is_empty()
        {
            return false;
        }

        let provider_matches = self.provider_keys.is_empty()
            || provider_key.is_some_and(|provider_key| {
                let provider_key = canonical_provider_key(provider_key);
                self.provider_keys
                    .iter()
                    .any(|candidate| canonical_provider_key(candidate) == provider_key)
            });
        let model = model.trim().to_ascii_lowercase();
        let model_matches = (self.models.is_empty() && self.model_prefixes.is_empty())
            || self
                .models
                .iter()
                .any(|candidate| candidate.trim().eq_ignore_ascii_case(&model))
            || self
                .model_prefixes
                .iter()
                .any(|prefix| model.starts_with(&prefix.trim().to_ascii_lowercase()));

        provider_matches && model_matches
    }
}

fn canonical_provider_key(provider_key: &str) -> String {
    jcode_provider_core::AuthRoute::parse(provider_key)
        .map(|route| route.session_provider_key().to_string())
        .unwrap_or_else(|| provider_key.trim().to_ascii_lowercase())
}

pub fn ledger_path() -> Option<PathBuf> {
    crate::storage::jcode_dir()
        .ok()
        .map(|dir| dir.join(MANIFESTATIONS_FILE))
}

pub fn resolve(provider_key: Option<&str>, model: &str) -> Option<Manifestation> {
    let path = ledger_path()?;
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            crate::logging::warn(&format!(
                "Failed to read manifestation ledger '{}': {}. Fix: make the file readable or remove it.",
                path.display(),
                error
            ));
            return None;
        }
    };
    let ledger: ManifestationLedger = match toml::from_str(&content) {
        Ok(ledger) => ledger,
        Err(error) => {
            crate::logging::warn(&format!(
                "Failed to parse manifestation ledger '{}': {}. Fix: correct the TOML syntax.",
                path.display(),
                error
            ));
            return None;
        }
    };

    ledger
        .manifestations
        .into_iter()
        .find(|entry| entry.matches(provider_key, model))
        .and_then(|entry| {
            let name = entry.name.trim().to_string();
            let character = entry.character.trim().to_string();
            (!name.is_empty() && !character.is_empty()).then_some(Manifestation { name, character })
        })
}

pub fn active_model_context_lines(provider_key: Option<&str>, model: Option<&str>) -> Vec<String> {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Vec::new();
    };
    let provider_key = provider_key
        .map(str::trim)
        .filter(|provider_key| !provider_key.is_empty())
        .unwrap_or("unknown");
    let mut lines = vec![
        format!("Active provider: {provider_key}"),
        format!("Active model: {model}"),
    ];

    if let Some(manifestation) = resolve(Some(provider_key), model) {
        lines.push(format!("Manifestation: {}", manifestation.name));
        lines.push(format!(
            "Manifestation character: {}",
            manifestation.character
        ));
        if let Some(path) = ledger_path() {
            lines.push(format!("Manifestation ledger: {}", path.display()));
        }
        lines.push(String::new());
        lines.push(format!(
            "Identity: Refer to yourself as {} while this model is active. Jcode is the harness. {} names this model lineage.",
            manifestation.name, manifestation.name
        ));
    } else {
        lines.push("Manifestation: unassigned".to_string());
        if let Some(path) = ledger_path() {
            lines.push(format!("Manifestation ledger: {}", path.display()));
        }
        lines.push(String::new());
        lines.push(
            "Identity: Refer to yourself as Jcode until the ledger assigns this model lineage."
                .to_string(),
        );
    }

    lines
}

pub fn model_change_notice(provider_key: Option<&str>, model: &str) -> String {
    let lines = active_model_context_lines(provider_key, Some(model));
    format!(
        "<system-reminder>\n# Active Model Changed\n\n{}\n</system-reminder>",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ledger(content: &str) -> ManifestationLedger {
        toml::from_str(content).expect("manifestation ledger parses")
    }

    #[test]
    fn ledger_matches_specific_models_and_provider_lineages() {
        let ledger = parse_ledger(
            r#"
                [[manifestations]]
                name = "Solaris"
                character = "the sun"
                provider_keys = ["openai"]
                models = ["gpt-5.6-sol"]

                [[manifestations]]
                name = "Sounder"
                character = "the depth reader"
                provider_keys = ["deepseek"]
            "#,
        );

        assert!(ledger.manifestations[0].matches(Some("openai-oauth"), "gpt-5.6-sol"));
        assert!(!ledger.manifestations[0].matches(Some("openai"), "gpt-5.6-luna"));
        assert!(ledger.manifestations[1].matches(Some("deepseek"), "deepseek-v4-pro"));
    }

    #[test]
    fn empty_matchers_never_claim_every_model() {
        let ledger = parse_ledger(
            r#"
                [[manifestations]]
                name = "Broken"
                character = "must not match"
            "#,
        );

        assert!(!ledger.manifestations[0].matches(Some("openai"), "gpt-5.6-sol"));
    }
}
