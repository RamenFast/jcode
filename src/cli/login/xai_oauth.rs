use super::*;

pub(super) async fn login(options: &LoginOptions) -> Result<LoginFlowOutcome> {
    if options.has_provided_input() || (options.complete && options.print_auth_url) {
        anyhow::bail!(
            "xAI uses device authorization. Use --print-auth-url, then --complete. Do not supply a callback URL or authorization code."
        );
    }
    let authorization = if options.complete {
        auth::xai_oauth::pending_login()?
    } else {
        auth::xai_oauth::begin_login().await?
    };
    let url = authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&authorization.verification_uri);
    if !options.complete {
        if options.json {
            println!(
                "{}",
                serde_json::json!({
                    "status":"pending", "tool":"jcode", "version":env!("CARGO_PKG_VERSION"),
                    "ts":chrono::Utc::now().to_rfc3339(), "provider":"xai-oauth", "auth_url":url,
                    "user_code":authorization.user_code, "expires_at":authorization.expires_at,
                    "resume_command":"jcode login --provider xai-oauth --complete --json"
                })
            );
        } else {
            eprintln!(
                "xAI OAuth\nOpen: {url}\nConfirm code: {}\nJcode owns this login and its tools. No Grok CLI is used.",
                authorization.user_code
            );
        }
        if !auth::browser_suppressed(options.no_browser) {
            super::maybe_open_browser(url, options.no_browser);
        }
    }
    if options.print_auth_url {
        return Ok(LoginFlowOutcome::Deferred);
    }
    auth::xai_oauth::complete_login(&authorization).await?;
    if options.json {
        println!(
            "{}",
            serde_json::json!({"status":"authenticated", "tool":"jcode", "version":env!("CARGO_PKG_VERSION"),
            "ts":chrono::Utc::now().to_rfc3339(), "provider":"xai-oauth", "inference_validated":false})
        );
    } else {
        eprintln!("xAI OAuth credentials saved. Checking model access next.");
    }
    Ok(LoginFlowOutcome::Completed)
}
