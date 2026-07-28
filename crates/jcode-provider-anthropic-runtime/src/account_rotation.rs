//! Rate-limit rotation across the stored Anthropic OAuth accounts.
//!
//! Anthropic reports a spent subscription window as a 429 on the streaming
//! request itself, and that error is delivered *inside* the response stream.
//! By then `complete_split` has already returned `Ok(stream)` to the caller, so
//! the provider-level machinery in `jcode_base::provider` cannot see the
//! failure: both the cross-provider fallback chain
//! (`MultiProvider::complete_with_failover`) and the same-provider account
//! failover (`try_same_provider_account_failover`) only guard the synchronous
//! stream-establishment call, and a 429 never reaches either.
//!
//! Rotating from inside the runtime's retry loop is therefore the only place a
//! second Anthropic subscription can rescue an in-flight turn.

use super::CachedCredentials;
use jcode_base::auth;
use jcode_base::auth::oauth;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Number of stored Anthropic OAuth accounts jcode can rotate between.
pub(super) fn anthropic_account_count() -> usize {
    auth::claude::list_accounts()
        .map(|accounts| accounts.len())
        .unwrap_or(0)
}

/// Whether jcode may transparently move to another stored Anthropic account
/// when the current one is rate limited
/// (`[provider].same_provider_account_failover`, default true).
pub(super) fn account_rotation_enabled() -> bool {
    jcode_base::config::Config::load()
        .provider
        .same_provider_account_failover
}

/// Move to the next stored Anthropic OAuth account that has not been tried for
/// this request and return `(label, access_token)` for it.
///
/// `tried_labels` accumulates every account already attempted, so a single
/// request walks each account at most once. Returns `None` when no untried
/// account has a usable token, which lets the caller fall through to the normal
/// transient-retry and hard-failure handling.
pub(super) async fn rotate_to_next_anthropic_account(
    tried_labels: &mut Vec<String>,
    credentials: &Arc<RwLock<Option<CachedCredentials>>>,
) -> Option<(String, String)> {
    let accounts = auth::claude::list_accounts().ok()?;

    for account in accounts {
        if tried_labels.iter().any(|tried| tried == &account.label) {
            continue;
        }
        tried_labels.push(account.label.clone());

        // A stored token that has already expired cannot rescue the turn on its
        // own, so refresh it first rather than trading a 429 for a 401. The
        // refresh is single-flighted and persists the rotated tokens.
        //
        // Carry the refreshed expiry and refresh token, not the stored ones: a
        // refresh rotates both, and caching the stale pair would leave the
        // credential cache describing a token that no longer exists (and
        // looking expired the moment it was written).
        let now_ms = chrono::Utc::now().timestamp_millis();
        let (usable_token, usable_refresh, usable_expires) = if account.expires > now_ms
            && !account.access.is_empty()
        {
            (
                account.access.clone(),
                account.refresh.clone(),
                account.expires,
            )
        } else if account.refresh.is_empty() {
            continue;
        } else {
            match oauth::refresh_claude_tokens_for_account(&account.refresh, &account.label).await {
                Ok(refreshed) => (
                    refreshed.access_token,
                    refreshed.refresh_token,
                    refreshed.expires_at,
                ),
                Err(err) => {
                    jcode_base::logging::warn(&format!(
                        "Anthropic account '{}' could not be refreshed during rate-limit rotation: {err:#}",
                        account.label
                    ));
                    continue;
                }
            }
        };

        // Point the rest of the process at this account so later turns (and the
        // forced-refresh path) follow the account actually serving traffic
        // instead of going back to the limited one.
        auth::claude::set_active_account_override(Some(account.label.clone()));
        {
            let mut cached = credentials.write().await;
            *cached = Some(CachedCredentials {
                access_token: usable_token.clone(),
                refresh_token: usable_refresh,
                expires_at: usable_expires,
            });
        }

        return Some((account.label, usable_token));
    }

    None
}
