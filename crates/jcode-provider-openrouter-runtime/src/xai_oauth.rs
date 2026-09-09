//! Direct xAI Responses transport. Jcode owns the tool loop.
use super::*;
use anyhow::bail;
use futures::stream;
use jcode_base::auth::xai_oauth as auth;
use jcode_provider_openai::stream::OpenAIResponsesStream;
use std::time::Duration;

pub(super) fn validate_api_url(url: &reqwest::Url) -> Result<()> {
    if url.scheme() != "https"
        || url.host_str() != Some("api.x.ai")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !(url.path() == "/v1" || url.path().starts_with("/v1/"))
    {
        bail!(
            "Native xAI OAuth only sends credentials to https://api.x.ai/v1. Remove the endpoint override."
        );
    }
    Ok(())
}

fn request_body(
    model: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    system: &str,
    effort: Option<&str>,
) -> Value {
    let mut messages = messages.to_vec();
    for message in &mut messages {
        message.content.retain(|block| {
            !matches!(
                block,
                ContentBlock::OpenAICompaction { .. }
                    | ContentBlock::OpenAIReasoning { .. }
                    | ContentBlock::AnthropicThinking { .. }
            )
        });
    }
    let input = jcode_provider_openai::build_responses_input(&messages);
    let mut request = serde_json::json!({
        "model": model, "input": input, "instructions": system, "stream": true, "store": false,
    });
    if !tools.is_empty() {
        request["tools"] = serde_json::json!(jcode_provider_openai::build_tools(tools));
        request["tool_choice"] = serde_json::json!("auto");
    }
    if let Some(effort) = effort.filter(|_| model == "grok-4.6") {
        let effort = if effort == "max" { "xhigh" } else { effort };
        request["reasoning"] = serde_json::json!({"effort": effort});
    }
    request
}

pub(super) async fn catalog(raw_url: &str) -> Result<reqwest::Response> {
    let url = reqwest::Url::parse(raw_url)?;
    validate_api_url(&url)?;
    let client = auth::client()?;
    let mut token = auth::access_token(None).await?;
    for attempt in 0..2 {
        let response = client.get(url.clone()).bearer_auth(&token).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            token = auth::access_token(Some(&token)).await?;
            continue;
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            bail!("xAI OAuth catalog denied (HTTP 403). Check account OAuth API entitlement with xAI. Repeated login will not resolve this denial.");
        }
        if !status.is_success() { bail!("xAI OAuth catalog returned HTTP {status}. Check account access and provider availability."); }
        return Ok(response);
    }
    bail!("xAI OAuth catalog rejected refreshed credentials. Run `jcode login --provider xai-oauth`.")
}

pub(super) async fn complete(
    provider: &OpenRouterProvider,
    messages: &[Message],
    tools: &[ToolDefinition],
    system: &str,
) -> Result<EventStream> {
    let model = provider.model.read().await.clone();
    let effort = provider.reasoning_effort();
    let mut request = request_body(&model, messages, tools, system, effort.as_deref());
    request["prompt_cache_key"] = serde_json::json!(provider.conversation_id);
    let url = reqwest::Url::parse(&format!(
        "{}/responses",
        provider.api_base.trim_end_matches('/')
    ))?;
    validate_api_url(&url)?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(180))
        .user_agent(concat!("jcode/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut token = auth::access_token(None).await?;
    for attempt in 0..2 {
        let response = tokio::time::timeout(
            Duration::from_secs(60),
            client
                .post(url.clone())
                .bearer_auth(&token)
                .json(&request)
                .send(),
        )
        .await
        .context("xAI request timed out. Retry after checking provider availability.")??;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            token = auth::access_token(Some(&token)).await?;
            continue;
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            bail!(
                "xAI OAuth inference denied (HTTP 403). Check this account's OAuth API entitlement with xAI. Do not repeat login or substitute an API key."
            );
        }
        if !status.is_success() {
            bail!(
                "xAI Responses returned HTTP {status} for model '{model}'. Check model access, reasoning effort, and provider availability. No tool was executed."
            );
        }
        let parsed = Box::pin(OpenAIResponsesStream::new(response.bytes_stream()));
        return Ok(Box::pin(stream::unfold(
            (parsed, false),
            |(mut parsed, done)| async move {
                if done {
                    return None;
                }
                match tokio::time::timeout(Duration::from_secs(180), parsed.next()).await {
                    Ok(Some(event)) => {
                        let done = event.is_err()
                            || matches!(
                                &event,
                                Ok(StreamEvent::MessageEnd { .. } | StreamEvent::Error { .. })
                            );
                        Some((event, (parsed, done)))
                    }
                    Ok(None) => Some((
                        Err(anyhow::anyhow!(
                            "xAI stream ended without a completion event. Retry the interrupted turn."
                        )),
                        (parsed, true),
                    )),
                    Err(_) => Some((
                        Err(anyhow::anyhow!(
                            "xAI stream idle timeout after 180 seconds. Retry after checking provider availability."
                        )),
                        (parsed, true),
                    )),
                }
            },
        )));
    }
    bail!("xAI OAuth rejected refreshed credentials. Run `jcode login --provider xai-oauth`.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_request_preserves_text_and_excludes_foreign_encryption() {
        let mut message = Message::user("hello");
        message.content.push(ContentBlock::OpenAICompaction {
            encrypted_content: "foreign-secret".into(),
        });
        let request = request_body("grok-4.6", &[message], &[], "instruction", Some("max"));
        assert!(!request.to_string().contains("foreign-secret"));
        assert!(request.to_string().contains("hello"));
        assert_eq!(request["reasoning"]["effort"], "xhigh");
        assert_eq!(request["store"], false);
        assert!(request.get("previous_response_id").is_none());
    }
    #[test]
    fn native_auth_rejects_other_hosts_and_cleartext() {
        assert!(
            validate_api_url(&reqwest::Url::parse("https://api.x.ai/v1/responses").unwrap())
                .is_ok()
        );
        for url in [
            "https://api.x.ai.evil.test/v1",
            "http://api.x.ai/v1",
            "https://api.x.ai/private",
            "https://user@api.x.ai/v1",
        ] {
            assert!(validate_api_url(&reqwest::Url::parse(url).unwrap()).is_err());
        }
    }
    #[tokio::test]
    async fn responses_text_and_completion_use_native_parser() {
        let bytes = bytes::Bytes::from_static(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"native\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":4,\"output_tokens\":1}}}\n\n");
        let parsed = OpenAIResponsesStream::new(stream::iter([Ok::<_, reqwest::Error>(bytes)]));
        let events: Vec<_> = parsed.collect().await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Ok(StreamEvent::TextDelta(text)) if text == "native"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Ok(StreamEvent::MessageEnd { .. })))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Ok(StreamEvent::TokenUsage { .. })))
        );
    }
}
