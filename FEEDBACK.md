# FEEDBACK.md — Jcode originating intent and session feedback ledger

<!-- DO NOT DELETE. Ben's permission is required to delete this file. -->
<!-- Scope: THIS repo only (the Jcode coding-agent harness, its server, clients, tools, and swarm runtime). -->

This ledger is the originating product intent for Jcode. Working agents must read
and apply the entries relevant to their current task before choosing behavior or
tradeoffs. Preserve Ben's words as constraints rather than silently replacing
them with generic best practices.

Rules:
- One line per atomic item: `- YYYY-MM-DD [kind] description` where kind is one
  of `ask` · `correction` · `bugfix` · `feature` · `ui` · `claim`.
- On a duplicate/repeat, do not add a new line. Tighten the existing line if
  needed, prefix a counter such as `2x`, and update its date.
- Newest entries go at the bottom.
- Apply relevant entries to active work, but do not invent unrelated work from
  stale entries or reinterpret a specific correction as blanket permission for
  destructive changes outside the user's request.

## Ledger

- 2026-07-25 [ask] Autonomously inspect Jcode personally, without subagents, and identify the three main areas needing heavy rework or polish to align the app with its intended outcomes.
- 2026-07-25 [ask] Exercise self-pacing, retain the overall opened goal throughout the work, and test that the app preserves full functionality.
- 2026-07-25 [correction] Jcode agents should retain the highest practical level of access over Ben's device; do not add repetitive approval gates or restrict an agent merely because an action is powerful.
- 2026-07-25 [ui] Powerful or unusually destructive actions should remain permissive but provide clear, non-blocking UI feedback while they happen.
- 2026-07-25 [ask] Fully repair reload, detach, and dead-terminal continuity paths so interrupted work and healthy live sessions are not lost or misclassified.
- 2026-07-25 [ask] Make swarm terminal-state reporting truthful enough that vanished workers cannot look like successful completions.
- 2026-07-25 [correction] Treat this FEEDBACK.md as Jcode's originating intent and keep implementation choices self-evidently aligned with it.
- 2026-07-30 [ask] Write every string jcode injects into agent attention to Ben's context standards: the system prompt, the todo quality gates, and the auto-poke continuation. Authority is the ben-context-standards skill.
- 2026-07-30 [correction] A synthetic gate must mark itself as not a user message, state its effect on the store, name the next action, and disclose no score or threshold. Keep a LEGACY_* alias whenever a detection anchor changes so existing transcripts still replay.
