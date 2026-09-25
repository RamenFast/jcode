use super::*;

#[derive(Debug, Clone)]
pub(super) struct PendingProviderFailover {
    pub(super) prompt: crate::provider::ProviderFailoverPrompt,
    pub(super) deadline: Instant,
    pub(super) remote_offer: Option<super::PendingFallbackOffer>,
}

impl App {
    fn provider_failover_route_selection(
        &self,
        prompt: &crate::provider::ProviderFailoverPrompt,
    ) -> Option<crate::provider::RouteSelection> {
        let target = prompt.to_model.as_deref()?.trim();
        let matching_route = self.fallback_candidate_routes().into_iter().find(|route| {
            crate::provider::MultiProvider::model_switch_request_for_session_route(
                &route.model,
                None,
                Some(&route.api_method),
            )
            .eq_ignore_ascii_case(target)
        });
        if let Some(route) = matching_route {
            return Some(crate::provider::RouteSelection::from_model_route(&route));
        }

        let model = target.strip_prefix("openrouter:")?.trim();
        if model.is_empty() {
            return None;
        }
        Some(crate::provider::RouteSelection {
            model: model.to_string(),
            runtime_key: crate::provider::RuntimeKey::OpenRouter,
            api_method: "openrouter".to_string(),
            provider_label: "auto".to_string(),
            detail: String::new(),
        })
    }

    fn provider_failover_offer(
        &self,
        prompt: &crate::provider::ProviderFailoverPrompt,
        remote_resend: Option<super::FallbackResendPayload>,
    ) -> Option<super::PendingFallbackOffer> {
        Some(super::PendingFallbackOffer {
            selection: self.provider_failover_route_selection(prompt)?,
            target_label: prompt.to_label.clone(),
            from_label: prompt.from_label.clone(),
            remote_resend: if self.is_remote { remote_resend } else { None },
        })
    }

    pub(super) fn current_remote_fallback_resend(&self) -> Option<super::FallbackResendPayload> {
        self.is_remote
            .then(|| {
                self.rate_limit_pending_message.as_ref().map(|pending| {
                    super::FallbackResendPayload {
                        content: pending.content.clone(),
                        images: pending.images.clone(),
                        is_system: pending.is_system,
                        auto_retry: pending.auto_retry,
                        system_reminder: pending.system_reminder.clone(),
                        raw_input: self.last_submitted_input.clone(),
                    }
                })
            })
            .flatten()
    }

    fn arm_provider_failover_approval(
        &mut self,
        prompt: &crate::provider::ProviderFailoverPrompt,
        remote_resend: Option<super::FallbackResendPayload>,
    ) -> bool {
        let Some(offer) = self.provider_failover_offer(prompt, remote_resend) else {
            return false;
        };
        let key_label = crate::tui::keybind::fallback_switch_key_label();
        self.push_display_message(DisplayMessage::system(format!(
            "⚠ OpenRouter approval required. Jcode will not send this prompt to OpenRouter automatically.\n\nPress {} to approve switching to {} and resend {}. No action occurs unless you approve.\n\nReason: {}",
            key_label,
            prompt.to_label,
            Self::format_failover_input_summary(prompt),
            prompt.reason,
        )));
        self.set_status_notice(format!(
            "Approval required: press {} for OpenRouter",
            key_label
        ));
        self.pending_fallback_offer = Some(offer);
        true
    }

    pub(super) fn handle_provider_failover_prompt_with_payload(
        &mut self,
        prompt: crate::provider::ProviderFailoverPrompt,
        remote_resend: Option<super::FallbackResendPayload>,
    ) {
        if prompt.requires_user_approval() {
            if self.arm_provider_failover_approval(&prompt, remote_resend) {
                return;
            }
            self.push_display_message(DisplayMessage::system(format!(
                "⚠ OpenRouter approval is required, but Jcode could not prepare the route. Use /model to select {} manually. No prompt was resent.",
                prompt.to_label,
            )));
            self.set_status_notice("OpenRouter fallback not prepared");
            return;
        }

        let input_summary = Self::format_failover_input_summary(&prompt);
        let manual_message = format!(
            "⚠ {} became unavailable - jcode did not resend your prompt to {} automatically.\n\nReason: {}\n\nRetrying elsewhere would send {}.\n\nTo switch manually now, use /model and pick a model from {}, then resend. {}",
            prompt.from_label,
            prompt.to_label,
            prompt.reason,
            input_summary,
            prompt.to_label,
            Self::failover_config_hint(),
        );

        match crate::config::Config::load()
            .provider
            .cross_provider_failover
        {
            crate::config::CrossProviderFailoverMode::Manual => {
                self.push_display_message(DisplayMessage::system(manual_message));
                self.set_status_notice(format!(
                    "{} unavailable; switch manually if desired",
                    prompt.from_label
                ));
            }
            crate::config::CrossProviderFailoverMode::Countdown => {
                let remote_offer = if self.is_remote {
                    self.provider_failover_offer(&prompt, remote_resend)
                } else {
                    None
                };
                if self.is_remote && remote_offer.is_none() {
                    self.push_display_message(DisplayMessage::system(format!(
                        "{}\n\nJcode could not resolve the exact remote route, so it did not start the countdown.",
                        manual_message,
                    )));
                    self.set_status_notice("Provider fallback route unavailable");
                    return;
                }
                self.pending_provider_failover = Some(PendingProviderFailover {
                    prompt: prompt.clone(),
                    deadline: Instant::now() + Duration::from_secs(3),
                    remote_offer,
                });
                self.push_display_message(DisplayMessage::system(format!(
                    "⚠ {} became unavailable - jcode will switch to {} in 3 seconds unless you cancel.\n\nReason: {}\n\nRetrying would send {}. Press Esc to cancel.\n\n{}",
                    prompt.from_label,
                    prompt.to_label,
                    prompt.reason,
                    input_summary,
                    Self::failover_config_hint(),
                )));
                self.set_status_notice(format!(
                    "Provider auto-switch → {} in 3s (Esc to cancel)",
                    prompt.to_label
                ));
            }
        }
    }
}
