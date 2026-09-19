# Requests

## 2026-09-19: Daybreak Blue context metadata

**Request:** Make Daybreak Blue usable through native OAuth across local harnesses, establish its context capacity without expensive long-input probes, and enable Ollama Cloud in Jcode and Prime.

**Jcode source change:** Fetch the ungated OAuth model catalog. For the exact `gpt-daybreak-blue-latest` model, use a positive `max_context_window` when present. Keep other models' context selection unchanged. Fall back to the advertised default when maximum metadata is absent or invalid.

**Evidence:** The authenticated catalog on 2026-09-19 reported a 272,000-token default and an 872,000-token maximum. No long-input acceptance probe was performed. Historical 900K comments were not treated as current capacity evidence.

**Tests:** Three focused parser tests passed under `cargo test --locked -p jcode-base --lib daybreak_catalog --profile selfdev`. They cover the 872K maximum, unchanged unrelated models, invalid/missing maximum metadata, and a future lower maximum.

**Runtime configuration:** Prime, DeepSeek Harness, and Hermes resolve Daybreak through their own OAuth credentials at 872K. Their short tool round trips passed. Ollama Cloud is configured in Jcode and Prime using a protected shared key reference. Existing defaults and worker routing remain unchanged.

**Boundary:** Build and install verification are recorded in the local task receipt. Active sessions are not restarted. The previous source state remains reachable through `daybreak-source-before-20260919`.
