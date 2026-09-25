fn remote_openrouter_route(model: &str) -> crate::provider::ModelRoute {
    crate::provider::ModelRoute {
        model: model.to_string(),
        provider: "auto".to_string(),
        api_method: "openrouter".to_string(),
        available: true,
        detail: String::new(),
        cheapness: None,
    }
}

fn prepare_remote_failover_app(model: &str, provider: &str) -> App {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_model = Some(model.to_string());
    app.remote_provider_name = Some(provider.to_string());
    app.rate_limit_pending_message = Some(PendingRemoteMessage {
        content: "retry me".to_string(),
        images: vec![],
        is_system: false,
        system_reminder: None,
        auto_retry: false,
        retry_attempts: 0,
        retry_at: None,
    });
    app.last_submitted_input = Some("retry me".to_string());
    app.is_processing = true;
    app.status = ProcessingStatus::Streaming;
    app
}

#[test]
fn test_openrouter_failover_requires_approval_before_switch_and_resend() {
    with_temp_jcode_home(|| {
        write_test_config("[provider]\ncross_provider_failover = \"countdown\"\n");
        let (mut app, set_model_calls) = create_openrouter_spec_capture_test_app();
        let prompt = crate::provider::ProviderFailoverPrompt {
            from_provider: "openrouter".to_string(),
            from_label: "Z.AI".to_string(),
            to_provider: "openrouter".to_string(),
            to_label: "OpenRouter (openai/gpt-5.4)".to_string(),
            to_model: Some("openrouter:openai/gpt-5.4".to_string()),
            reason: "Z.AI usage exhausted".to_string(),
            estimated_input_chars: 32_000,
            estimated_input_tokens: 8_000,
        };

        app.handle_turn_error(failover_error_message(&prompt));

        assert!(app.pending_provider_failover.is_none());
        assert!(app.pending_fallback_offer.is_some());
        assert!(!app.pending_turn);
        assert!(set_model_calls.lock().unwrap().is_empty());
        let notice = app.display_messages.last().expect("approval notice");
        assert!(notice.content.contains("OpenRouter approval required"));
        assert!(notice.content.contains("will not send this prompt"));

        assert!(app.apply_pending_fallback_offer());
        assert!(app.pending_turn);
        assert_eq!(
            set_model_calls.lock().unwrap().as_slice(),
            &["openai/gpt-5.4".to_string()]
        );
    });
}

#[test]
fn remote_configured_failover_countdown_stages_exact_route_and_resend() {
    with_temp_jcode_home(|| {
        write_test_config("[provider]\ncross_provider_failover = \"countdown\"\n");
        let mut app = prepare_remote_failover_app("claude-opus-5", "Anthropic");
        app.remote_model_options = vec![openai_oauth_route("gpt-5.6-sol")];
        let prompt = crate::provider::ProviderFailoverPrompt {
            from_provider: "claude".to_string(),
            from_label: "Anthropic OAuth".to_string(),
            to_provider: "openai".to_string(),
            to_label: "ChatGPT OAuth (gpt-5.6-sol)".to_string(),
            to_model: Some("openai-oauth:gpt-5.6-sol".to_string()),
            reason: "Anthropic usage exhausted".to_string(),
            estimated_input_chars: 8_000,
            estimated_input_tokens: 2_000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.handle_server_event(
            crate::protocol::ServerEvent::Error {
                id: 71,
                message: prompt.to_error_message(),
                retry_after_secs: None,
            },
            &mut remote,
        );

        let pending = app
            .pending_provider_failover
            .as_mut()
            .expect("remote countdown should be armed");
        assert!(pending.remote_offer.is_some());
        pending.deadline = Instant::now() - Duration::from_secs(1);
        assert!(app.maybe_progress_provider_failover_countdown());
        let selection = app
            .pending_route_selection
            .as_ref()
            .expect("countdown should stage the remote route");
        assert_eq!(selection.api_method, "openai-oauth");
        assert_eq!(selection.model, "gpt-5.6-sol");
        assert_eq!(
            app.pending_fallback_resend
                .as_ref()
                .map(|payload| payload.content.as_str()),
            Some("retry me")
        );
    });
}

#[test]
fn remote_openrouter_failover_waits_for_user_approval() {
    with_temp_jcode_home(|| {
        write_test_config("[provider]\ncross_provider_failover = \"countdown\"\n");
        let mut app = prepare_remote_failover_app("glm-5", "Z.AI");
        app.remote_model_options = vec![remote_openrouter_route(
            "anthropic/claude-opus-5",
        )];
        let prompt = crate::provider::ProviderFailoverPrompt {
            from_provider: "openrouter".to_string(),
            from_label: "Z.AI (glm-5)".to_string(),
            to_provider: "openrouter".to_string(),
            to_label: "OpenRouter (anthropic/claude-opus-5)".to_string(),
            to_model: Some("openrouter:anthropic/claude-opus-5".to_string()),
            reason: "Z.AI usage exhausted".to_string(),
            estimated_input_chars: 8_000,
            estimated_input_tokens: 2_000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.handle_server_event(
            crate::protocol::ServerEvent::Error {
                id: 72,
                message: prompt.to_error_message(),
                retry_after_secs: None,
            },
            &mut remote,
        );

        assert!(app.pending_provider_failover.is_none());
        assert!(app.pending_route_selection.is_none());
        assert!(app.rate_limit_pending_message.is_none());
        let offer = app
            .pending_fallback_offer
            .as_ref()
            .expect("OpenRouter approval offer should be armed");
        assert_eq!(offer.selection.api_method, "openrouter");
        assert_eq!(
            offer.remote_resend.as_ref().map(|payload| payload.content.as_str()),
            Some("retry me")
        );
        assert!(
            app.display_messages()
                .iter()
                .any(|message| message.content.contains("OpenRouter approval required"))
        );

        assert!(app.apply_pending_fallback_offer());
        assert_eq!(
            app.pending_route_selection
                .as_ref()
                .map(|selection| selection.api_method.as_str()),
            Some("openrouter")
        );
    });
}
