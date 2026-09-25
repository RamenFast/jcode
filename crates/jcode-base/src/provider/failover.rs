use super::*;
use jcode_provider_core::{FailoverDecision, ProviderFailoverPrompt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConfiguredModelFailoverTarget {
    pub provider: ActiveProvider,
    pub model: String,
    pub label: String,
}

impl MultiProvider {
    pub(super) fn provider_is_configured(&self, provider: ActiveProvider) -> bool {
        self.reconcile_auth_if_provider_missing(provider)
    }

    pub(super) fn provider_precheck_unavailable_reason(
        &self,
        provider: ActiveProvider,
    ) -> Option<String> {
        match provider {
            ActiveProvider::Claude if self.is_claude_usage_exhausted() => Some(
                crate::provider::account_failover::usage_exhausted_reason(provider),
            ),
            _ => None,
        }
    }

    pub(super) fn build_failover_prompt(
        &self,
        from: ActiveProvider,
        to: ActiveProvider,
        reason: String,
        estimated_input_chars: usize,
        estimated_input_tokens: usize,
    ) -> ProviderFailoverPrompt {
        ProviderFailoverPrompt {
            from_provider: Self::provider_key(from).to_string(),
            from_label: Self::provider_label(from).to_string(),
            to_provider: Self::provider_key(to).to_string(),
            to_label: Self::provider_label(to).to_string(),
            to_model: None,
            reason,
            estimated_input_chars,
            estimated_input_tokens,
        }
    }

    pub(super) fn build_model_failover_prompt(
        &self,
        from: ActiveProvider,
        target: &ConfiguredModelFailoverTarget,
        reason: String,
        estimated_input_chars: usize,
        estimated_input_tokens: usize,
    ) -> ProviderFailoverPrompt {
        ProviderFailoverPrompt {
            from_provider: Self::provider_key(from).to_string(),
            from_label: Self::provider_label(from).to_string(),
            to_provider: Self::provider_key(target.provider).to_string(),
            to_label: target.label.clone(),
            to_model: Some(target.model.clone()),
            reason,
            estimated_input_chars,
            estimated_input_tokens,
        }
    }

    pub(super) fn build_next_failover_error(
        &self,
        from: ActiveProvider,
        legacy_candidate: ActiveProvider,
        configured: &Option<Option<ConfiguredModelFailoverTarget>>,
        reason: Option<&str>,
        estimated_input_chars: usize,
        estimated_input_tokens: usize,
    ) -> Option<Option<anyhow::Error>> {
        let reason = reason?.to_string();
        let prompt = match configured {
            Some(Some(target)) => Some(self.build_model_failover_prompt(
                from,
                target,
                reason,
                estimated_input_chars,
                estimated_input_tokens,
            )),
            Some(None) => None,
            None => Some(self.build_failover_prompt(
                from,
                legacy_candidate,
                reason,
                estimated_input_chars,
                estimated_input_tokens,
            )),
        };
        Some(prompt.map(|prompt| anyhow::anyhow!(prompt.to_error_message())))
    }

    fn configured_route_target(route: &str) -> Option<(ActiveProvider, &str)> {
        if let Some((provider, _prefix, model)) = explicit_model_provider_prefix(route) {
            return Some((provider, model));
        }
        if let Some((_profile, model)) = Self::openai_compatible_model_prefix(route) {
            return Some((ActiveProvider::OpenRouter, model));
        }
        if let Some((profile, model)) = route.split_once(':')
            && crate::config::config()
                .providers
                .contains_key(profile.trim())
            && !model.trim().is_empty()
        {
            return Some((ActiveProvider::OpenRouter, model.trim()));
        }
        None
    }

    fn configured_route_label(route: &str) -> String {
        let (prefix, model) = route.split_once(':').unwrap_or((route, ""));
        let provider = match prefix {
            "claude-oauth" => "Anthropic OAuth".to_string(),
            "claude-api" => "Anthropic API".to_string(),
            "openai-oauth" => "ChatGPT OAuth".to_string(),
            "openai-api" => "OpenAI API".to_string(),
            profile_id => crate::provider_catalog::openai_compatible_profile_by_id(profile_id)
                .map(|profile| profile.display_name.to_string())
                .unwrap_or_else(|| profile_id.to_string()),
        };
        if model.is_empty() {
            provider
        } else {
            format!("{provider} ({model})")
        }
    }

    /// Return `None` when no route chain applies, `Some(None)` when the active
    /// route is the final entry, or `Some(Some(target))` for the next route.
    pub(super) fn configured_model_failover_target(
        &self,
        active: ActiveProvider,
    ) -> Option<Option<ConfiguredModelFailoverTarget>> {
        let routes = crate::config::config().provider.fallback_models.as_ref()?;
        Self::configured_model_failover_target_from_routes(routes, active, &self.model())
    }

    fn configured_model_failover_target_from_routes(
        routes: &[String],
        active: ActiveProvider,
        current_model: &str,
    ) -> Option<Option<ConfiguredModelFailoverTarget>> {
        let routes = routes
            .iter()
            .map(|route| route.trim())
            .filter(|route| !route.is_empty())
            .collect::<Vec<_>>();
        if routes.is_empty() {
            return None;
        }

        let current_index = routes.iter().position(|route| {
            Self::configured_route_target(route).is_some_and(|(provider, model)| {
                provider == active && model.eq_ignore_ascii_case(current_model)
            })
        })?;
        let Some(next_route) = routes.get(current_index + 1) else {
            return Some(None);
        };
        let (provider, _model) = Self::configured_route_target(next_route)?;
        Some(Some(ConfiguredModelFailoverTarget {
            provider,
            model: (*next_route).to_string(),
            label: Self::configured_route_label(next_route),
        }))
    }

    pub(super) fn fallback_sequence(active: ActiveProvider) -> Vec<ActiveProvider> {
        jcode_provider_core::fallback_sequence(active)
    }

    pub(super) fn summarize_error(err: &anyhow::Error) -> String {
        err.to_string()
            .lines()
            .next()
            .unwrap_or("unknown error")
            .trim()
            .to_string()
    }

    pub(super) fn classify_failover_error(err: &anyhow::Error) -> FailoverDecision {
        jcode_provider_core::classify_failover_error_message(&err.to_string())
    }

    pub(super) fn additional_no_provider_guidance(&self) -> Vec<String> {
        [ActiveProvider::Claude, ActiveProvider::OpenAI]
            .into_iter()
            .filter_map(crate::provider::account_failover::account_switch_guidance)
            .collect()
    }

    pub(super) fn no_provider_available_error(&self, notes: &[String]) -> anyhow::Error {
        let mut msg = "No tokens/providers left: no usable provider right now. Anthropic/OpenAI usage may be exhausted and GitHub Copilot is not authenticated or currently unavailable.".to_string();
        if !notes.is_empty() {
            msg.push_str(" Details: ");
            msg.push_str(&notes.join(" | "));
        }
        let extra_guidance = self.additional_no_provider_guidance();
        if !extra_guidance.is_empty() {
            msg.push(' ');
            msg.push_str(&extra_guidance.join(" "));
        }
        msg.push_str(" Use `/usage` to check limits and `/login <provider>` to re-authenticate.");
        anyhow::anyhow!(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_routes() -> Vec<String> {
        vec![
            "claude-oauth:claude-opus-5".to_string(),
            "openai-oauth:gpt-5.6-sol".to_string(),
            "zai:glm-5".to_string(),
            "openrouter:anthropic/claude-opus-5".to_string(),
        ]
    }

    #[test]
    fn configured_chain_advances_to_chatgpt_oauth() {
        let target = MultiProvider::configured_model_failover_target_from_routes(
            &configured_routes(),
            ActiveProvider::Claude,
            "claude-opus-5",
        )
        .expect("configured chain")
        .expect("next route");

        assert_eq!(target.provider, ActiveProvider::OpenAI);
        assert_eq!(target.model, "openai-oauth:gpt-5.6-sol");
        assert_eq!(target.label, "ChatGPT OAuth (gpt-5.6-sol)");
    }

    #[test]
    fn configured_chain_advances_from_chatgpt_to_zai() {
        let target = MultiProvider::configured_model_failover_target_from_routes(
            &configured_routes(),
            ActiveProvider::OpenAI,
            "gpt-5.6-sol",
        )
        .expect("configured chain")
        .expect("next route");

        assert_eq!(target.provider, ActiveProvider::OpenRouter);
        assert_eq!(target.model, "zai:glm-5");
        assert_eq!(target.label, "Z.AI (glm-5)");
    }

    #[test]
    fn configured_chain_advances_from_zai_to_openrouter() {
        let target = MultiProvider::configured_model_failover_target_from_routes(
            &configured_routes(),
            ActiveProvider::OpenRouter,
            "glm-5",
        )
        .expect("configured chain")
        .expect("next route");

        assert_eq!(target.provider, ActiveProvider::OpenRouter);
        assert_eq!(target.model, "openrouter:anthropic/claude-opus-5");
        assert_eq!(target.label, "OpenRouter (anthropic/claude-opus-5)");
    }

    #[test]
    fn configured_chain_stops_after_final_openrouter_route() {
        let target = MultiProvider::configured_model_failover_target_from_routes(
            &configured_routes(),
            ActiveProvider::OpenRouter,
            "anthropic/claude-opus-5",
        );

        assert_eq!(target, Some(None));
    }
}
