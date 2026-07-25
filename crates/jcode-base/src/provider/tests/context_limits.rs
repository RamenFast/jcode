#[test]
fn test_context_limit_spark_vs_codex() {
    // Other provider tests intentionally hydrate the process-wide runtime
    // cache. Inject an empty cache here and below so these tests cover the
    // static classifier rather than whichever runtime fixture ran first.
    assert_eq!(
        jcode_provider_core::context_limit_for_model_with_provider_and_cache(
            "gpt-5.3-codex-spark",
            None,
            |_| None,
        ),
        Some(128_000)
    );
    for model in ["gpt-5.5", "gpt-5.3-codex", "gpt-5.2-codex", "gpt-5-codex"] {
        assert_eq!(
            jcode_provider_core::context_limit_for_model_with_provider_and_cache(
                model,
                None,
                |_| None,
            ),
            Some(272_000)
        );
    }
}

#[test]
fn test_context_limit_gpt_5_4() {
    for model in ["gpt-5.4", "gpt-5.4-pro", "gpt-5.4[1m]"] {
        assert_eq!(
            jcode_provider_core::context_limit_for_model_with_provider_and_cache(
                model,
                None,
                |_| None,
            ),
            Some(1_000_000)
        );
    }
}

#[test]
fn test_context_limit_respects_provider_hint() {
    assert_eq!(
        jcode_provider_core::context_limit_for_model_with_provider_and_cache(
            "gpt-5.4",
            Some("openai"),
            |_| None,
        ),
        Some(1_000_000)
    );
    assert_eq!(
        jcode_provider_core::context_limit_for_model_with_provider_and_cache(
            "gpt-5.4",
            Some("copilot"),
            |_| None,
        ),
        Some(128_000)
    );
    assert_eq!(
        jcode_provider_core::context_limit_for_model_with_provider_and_cache(
            "claude-sonnet-4-6[1m]",
            Some("claude"),
            |_| None,
        ),
        Some(1_048_576)
    );
}

#[test]
fn test_resolve_model_capabilities_uses_provider_hint() {
    let openai = resolve_model_capabilities("gpt-5.4", Some("openai"));
    assert_eq!(openai.provider.as_deref(), Some("openai"));
    // The public resolver intentionally honors the current runtime cache; the
    // exact cache-free fallback is asserted separately above.
    assert_eq!(
        openai.context_window,
        context_limit_for_model_with_provider("gpt-5.4", Some("openai"))
    );

    let copilot = resolve_model_capabilities("gpt-5.4", Some("copilot"));
    assert_eq!(copilot.provider.as_deref(), Some("copilot"));
    assert_eq!(copilot.context_window, Some(128_000));

    let gemini = resolve_model_capabilities("gemini-2.5-pro", Some("gemini"));
    assert_eq!(gemini.provider.as_deref(), Some("gemini"));
    assert_eq!(gemini.context_window, Some(1_000_000));
}
