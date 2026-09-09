# Native xAI OAuth

## Vision

Ben wants xAI models inside Jcode with Jcode-owned tools, sessions, and credentials.
Grok Build as a subprocess is not this outcome. Do not install or invoke it for this provider.
Keep the existing xAI API-key route and all unrelated defaults unchanged.

## Contract

- Provider ID and route prefix: `xai-oauth`. Display label: `xAI OAuth`.
- Login uses the public xAI device grant at `https://auth.x.ai/oauth2/device/code`.
- Token exchange and refresh use `https://auth.x.ai/oauth2/token`.
- Public client: `b1a00492-073a-47ea-816f-4c329264a828`.
- Scopes: `openid profile email offline_access grok-cli:access api:access`.
- xAI owns the client label shown on its approval page. The grant does not authorize spawning the Grok CLI.
- Store credentials only in Jcode's `xai-oauth.json`, atomically with owner-only permissions.
- Refresh before expiry and once after an HTTP 401. Re-read credentials under an exclusive cross-process lock before rotating tokens.
- Preserve the old refresh token if a successful refresh omits a replacement.
- Do not replace good credentials on errors. A revoked grant requires login. A 403 requires account-entitlement investigation, not repeated login.
- Login supports a printed device URL and resumable completion for noninteractive use, as well as the TUI device-code surface.
- Never use `XAI_API_KEY` or another harness's credential store as an OAuth fallback.
- Native inference uses `https://api.x.ai/v1/responses` and live model discovery uses `/v1/models`.
- Reuse Responses serialization and stream parsing, not the OpenAI account/transport runtime.
- Strip foreign encrypted reasoning/compaction payloads. Retain ordinary text and tool history.
- Jcode sends tool definitions, parses tool calls, executes its normal tools, and returns tool results.
- Preserve cancellation, streamed completion errors, and bounded request/stream timeouts.
- Keep xAI-specific authentication confined to xAI HTTPS hosts. Test endpoints use isolated local fixtures, not production credential redirects.
- Model IDs and reasoning support are capabilities to verify. Do not advertise Grok 4.6/max as account-accepted until real requests pass.
- Official Grok 4.6 documentation reports 500,000 context tokens and low/medium/high/xhigh reasoning. Jcode's `max` alias sends `xhigh`, not an unsupported wire value. Other models do not inherit this capability.
- Send one stable `prompt_cache_key` per provider conversation, with a new key for each session fork.

## Checks

1. Uninstalled Grok executable remains absent throughout all native checks.
2. Unit/HTTP fixtures cover pending, slowdown, expiry, denial, token validation, secure persistence, refresh rotation, and re-login errors.
3. CLI provider parsing, auth status, TUI login dispatch, runtime activation, and model routes recognize `xai-oauth` separately from `xai`.
4. Response fixtures cover text, tool arguments/results, usage, stream failure, foreign encrypted-state exclusion, and cancellation.
5. Live OAuth approval, model discovery, short response, tool round trip, and requested effort receive separate receipts.
6. Install the tested build while preserving the previous binary. Do not restart a shared server with active sessions.

## Evidence and valid blocked outcome

Live OIDC discovery on 2026-09-09 confirms device and refresh grants and the requested scopes.
Hermes's public xAI OAuth guide documents direct Responses inference and warns that some accounts receive HTTP 403 after login.
That reference is protocol evidence, not proof of this account's entitlement.

Blocked means the exact grant, entitlement, model, build, or runtime condition prevented a named check.
Keep successful local work and report the smallest resolving action. Do not substitute the Grok CLI or a paid API-key route.

References:
- https://auth.x.ai/.well-known/openid-configuration
- https://hermes-agent.nousresearch.com/docs/guides/xai-grok-oauth
- https://docs.x.ai/developers/grok-4-6

## Use

`jcode login --provider xai-oauth` opens native device authorization.
For an agent or headless terminal, use:

```bash
jcode --no-update login --provider xai-oauth --print-auth-url --no-browser --json
jcode --no-update login --provider xai-oauth --complete --json
jcode --no-update auth-test --provider xai-oauth --model grok-4.6 --json
```

Select `xai-oauth:grok-4.6` in the model picker. The xAI API-key provider remains `xai`.
The native route does not modify the global default provider during login.
Grok 4.6's strongest wire setting is `xhigh`. Jcode accepts `max` as an alias and reports the normalized setting.

## 2026-09-09 verification checkpoint

- Managed Grok executable removed and remains absent.
- Full Jcode CLI/TUI integration passes `cargo check --no-default-features`.
- Nine new native-auth tests pass, including real loopback HTTP refresh, cross-caller serialization, revoked-grant suppression, and exact credential preservation on transient errors.
- Three new Responses tests pass. The complete direct-provider runtime suite passes133 tests with one ignored existing test.
- All17 provider-metadata tests pass.
- Ben approved a direct xAI device grant. Its credentials are held only in Jcode and private diagnostic storage, not source control.
- Direct OAuth `GET /v1/models` returned200 and advertised `grok-4.6`.
- Direct OAuth `POST /v1/responses` returned200 and `XAI_NATIVE_OK` from `grok-4.6` at low effort.
- Installed-build inference, native tool execution, and strongest-effort checks remain pending at this checkpoint.
