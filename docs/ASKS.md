# Requests

## 2026-09-19: Daybreak Blue context metadata

**Request:** Make Daybreak Blue usable through native OAuth across local harnesses, establish its context capacity without expensive long-input probes, and enable Ollama Cloud in Jcode and Prime.

**Jcode source change:** Fetch the ungated OAuth model catalog. For the exact `gpt-daybreak-blue-latest` model, use a positive `max_context_window` when present. Keep other models' context selection unchanged. Fall back to the advertised default when maximum metadata is absent or invalid.

**Evidence:** The authenticated catalog on 2026-09-19 reported a 272,000-token default and an 872,000-token maximum. No long-input acceptance probe was performed. Historical 900K comments were not treated as current capacity evidence.

**Tests:** Three focused parser tests passed under `cargo test --locked -p jcode-base --lib daybreak_catalog --profile selfdev`. They cover the 872K maximum, unchanged unrelated models, invalid/missing maximum metadata, and a future lower maximum.

**Runtime configuration:** Prime, DeepSeek Harness, and Hermes resolve Daybreak through their own OAuth credentials at 872K. Their short tool round trips passed. Ollama Cloud is configured in Jcode and Prime using a protected shared key reference. Existing defaults and worker routing remain unchanged.

**Boundary:** Build and install verification are recorded in the local task receipt. Active sessions are not restarted. The previous source state remains reachable through `daybreak-source-before-20260919`.

## 2026-09-24: OAuth compatibility and session recovery

Request: recover the interrupted Prime session, expose configured OAuth models,
restart Hermes, repair Jcode Anthropic access, explain the updater error, and
attribute the unexpected Ollama GLM usage. Prime handles this without workers.

Jcode acceptance: reproduce the Opus 5.5 client-version rejection first, update
all three Anthropic client-version fields together, then pass native auth and
tool smoke checks. Preserve auth stores, configuration, existing custom changes,
and session data. Build and install through the current/source channel. Preserve
the old binary for rollback. Never replace this custom build with a release that
does not contain its commits. Fetch the missing official release tag and verify
the updater keeps the divergent development build without its misleading 404.

Failure conditions: an unsupported provider request remains a failure even if
compilation passes. Do not reload a busy shared server. Report any verification
or installation limit with its evidence.

Verification: `auth-before.json` reproduced the 400 client-version error.
The selfdev build passed. Native Opus 5.5 auth-test then passed credential,
refresh, response, and real bash-tool round-trip checks (`auth-after.json`).
Fetching official tag v0.88.0 fixed local ancestry lookup; `jcode update` now
retains the custom build without a 404. Broad guardrails expose unrelated
formatting debt. Receipts: `../checks/oauth-recovery-20260924/`.
