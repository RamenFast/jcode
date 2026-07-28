use super::*;
use crate::provider::{EventStream, Provider};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct MockSummaryProvider;

#[async_trait::async_trait]
impl Provider for MockSummaryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "mock-summary"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(MockSummaryProvider)
    }

    async fn complete_simple(&self, prompt: &str, _system: &str) -> Result<String> {
        Ok(format!("summary({} chars)", prompt.len()))
    }
}

fn make_text_message(role: Role, text: &str) -> Message {
    Message {
        role,
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }
}

#[test]
fn test_new_manager() {
    let manager = CompactionManager::new();
    assert_eq!(manager.compacted_count, 0);
    assert!(manager.active_summary.is_none());
    assert!(!manager.is_compacting());
}

#[test]
fn test_notify_message_added() {
    let mut manager = CompactionManager::new();
    manager.notify_message_added();
    manager.notify_message_added();
    assert_eq!(manager.total_turns, 2);
}

#[test]
fn test_restored_messages_do_not_trigger_compaction_immediately() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("restored {}", i)));
    }
    manager.seed_restored_messages(messages.len());
    manager.update_observed_input_tokens(900);

    assert!(
        !manager.should_compact_with(&messages),
        "restored history should not compact until a new message is added"
    );
}

#[test]
fn test_new_message_after_restore_reenables_compaction() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("restored {}", i)));
    }
    manager.seed_restored_messages(messages.len());
    manager.update_observed_input_tokens(900);
    assert!(!manager.should_compact_with(&messages));

    messages.push(make_text_message(Role::User, "new turn after restore"));
    manager.notify_message_added();

    assert!(
        manager.should_compact_with(&messages),
        "compaction should resume once a genuinely new message is added"
    );
}

#[test]
fn test_token_estimate() {
    let manager = CompactionManager::new();
    // 100 chars = ~25 tokens (plus 18k overhead for full budget)
    let messages = vec![make_text_message(Role::User, &"x".repeat(100))];
    let estimate = manager.token_estimate_with(&messages);
    // With DEFAULT_TOKEN_BUDGET and 18k overhead: 25 + 18000 = 18025
    assert!((18_000..19_000).contains(&estimate));
}

#[test]
fn test_should_compact() {
    let mut manager = CompactionManager::new().with_budget(100); // Very small budget

    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("Message {} with some content", i),
        ));
        manager.notify_message_added();
    }

    assert!(manager.should_compact_with(&messages));
}

#[test]
fn test_context_usage_prefers_observed_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let messages = vec![make_text_message(Role::User, "short message")];
    manager.notify_message_added();
    manager.update_observed_input_tokens(900);

    assert!(manager.context_usage_with(&messages) >= 0.90);
    assert!(manager.effective_token_count_with(&messages) >= 900);
}

#[test]
fn test_should_compact_uses_observed_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);

    let mut messages = Vec::new();
    for _ in 0..12 {
        messages.push(make_text_message(Role::User, "x"));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(850);

    assert!(manager.should_compact_with(&messages));
}

#[test]
fn test_messages_for_api_no_summary() {
    let mut manager = CompactionManager::new();
    let messages = vec![
        make_text_message(Role::User, "Hello"),
        make_text_message(Role::Assistant, "Hi!"),
    ];
    manager.notify_message_added();
    manager.notify_message_added();

    let msgs = manager.messages_for_api_with(&messages);
    assert_eq!(msgs.len(), 2);
}

#[tokio::test]
async fn test_force_compact_applies_summary() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("Turn {} {}", i, "x".repeat(120)),
        ));
        manager.notify_message_added();
    }

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager
        .force_compact_with(&messages, provider)
        .expect("manual compaction should start");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        manager.check_and_apply_compaction();
        if manager.stats().has_summary {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        manager.stats().has_summary,
        "summary should be applied after compaction task completes"
    );

    // After compaction, compacted_count should be > 0
    assert!(manager.compacted_count > 0);

    let msgs = manager.messages_for_api_with(&messages);
    assert!(msgs.len() < 30);
    let first = msgs.first().expect("summary message missing");
    assert_eq!(first.role, Role::User);
    match &first.content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
        }
        _ => panic!("expected text summary block"),
    }
}

// ── ensure_context_fits tests ──────────────────────────────

#[tokio::test]
async fn test_guard_below_80_does_nothing() {
    let mut manager = CompactionManager::new().with_budget(10_000);
    let mut messages = Vec::new();
    for i in 0..15 {
        messages.push(make_text_message(Role::User, &format!("msg {}", i)));
        manager.notify_message_added();
    }
    // Char estimate is tiny, observed tokens well below 80%
    manager.update_observed_input_tokens(5_000);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert_eq!(
        action,
        CompactionAction::None,
        "should do nothing below 80%"
    );
    assert!(
        !manager.is_compacting(),
        "should NOT start background compaction below 80%"
    );
    assert_eq!(manager.compacted_count, 0);
}

#[tokio::test]
async fn test_guard_between_80_and_95_starts_background_only() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("msg {}", i)));
        manager.notify_message_added();
    }
    // 85% usage — above 80% threshold but below 95% critical
    manager.update_observed_input_tokens(850);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert_eq!(
        action,
        CompactionAction::BackgroundStarted {
            trigger: "reactive".to_string()
        },
        "should start background compaction at 85%"
    );
    assert!(
        manager.is_compacting(),
        "SHOULD start background compaction at 85%"
    );
    assert_eq!(
        manager.compacted_count, 0,
        "compacted_count should stay 0 (no hard compact)"
    );
}

/// Regression: a hard compact that runs while a background (reactive)
/// compaction is in flight must abort the background task and discard its
/// stale `pending_cutoff`. Otherwise, when the background task completes,
/// `check_and_apply_compaction_with` adds the stale cutoff on top of the
/// already-advanced `compacted_count`, double-compacting and wiping out all
/// live messages (observed as "kept 0 recent messages").
#[tokio::test]
async fn test_hard_compact_aborts_inflight_background_compaction() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "z".repeat(60)),
        ));
        manager.notify_message_added();
    }

    // Start a background reactive compaction (85% usage, below critical).
    manager.update_observed_input_tokens(850);
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager.maybe_start_compaction_with(&messages, provider);
    assert!(
        manager.is_compacting(),
        "background compaction should be in flight"
    );
    let inflight_cutoff = manager.pending_cutoff;
    assert!(inflight_cutoff > 0, "background task should have a cutoff");

    // Now pressure spikes to critical and we hard-compact synchronously while
    // the background task is still pending.
    let dropped = manager
        .hard_compact_with(&messages)
        .expect("hard compact should succeed");
    assert!(dropped > 0);

    // The in-flight background compaction must have been aborted/discarded.
    assert!(
        !manager.is_compacting(),
        "hard compact must abort the in-flight background compaction"
    );
    assert_eq!(
        manager.pending_cutoff, 0,
        "stale pending_cutoff must be reset"
    );

    let compacted_after_hard = manager.compacted_count;

    // Simulate the (now-aborted) background task completion path. With the fix
    // there is no pending task, so this is a no-op and must NOT advance
    // compacted_count again.
    manager.check_and_apply_compaction_with(&messages);
    assert_eq!(
        manager.compacted_count, compacted_after_hard,
        "completing after abort must not double-advance compacted_count"
    );

    // Live messages must survive: active_messages_count stays positive.
    assert!(
        manager.active_messages_count() > 0,
        "must keep recent messages live, not wipe everything to 0"
    );
    let active = manager.active_messages(&messages);
    assert!(
        !active.is_empty(),
        "active message slice must not be empty after hard compact"
    );
}

/// Defense-in-depth: if `compacted_count` advances while a background
/// compaction is in flight (so its `pending_cutoff` becomes stale), applying
/// the completed result must NOT over-advance `compacted_count` and wipe the
/// live tail. The stale result should be discarded instead.
#[tokio::test]
async fn test_stale_background_result_discarded_when_context_shrinks() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "q".repeat(60)),
        ));
        manager.notify_message_added();
    }

    manager.update_observed_input_tokens(850);
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager.maybe_start_compaction_with(&messages, provider);
    assert!(manager.is_compacting());
    let pending = manager.pending_cutoff;
    assert!(pending > 0);

    // Simulate an interleaving mutation that advances compacted_count out from
    // under the in-flight task (e.g. a hard compact via a different path),
    // leaving only a small active tail.
    manager.compacted_count = messages.len() - 3;

    // Drain the background task to completion, then apply.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        manager.check_and_apply_compaction_with(&messages);
        if !manager.is_compacting() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!manager.is_compacting(), "task should have been drained");

    // The stale result must have been discarded: compacted_count stays where
    // the interleaving mutation left it, and the live tail survives.
    assert_eq!(
        manager.compacted_count,
        messages.len() - 3,
        "stale pending_cutoff must not advance compacted_count further"
    );
    assert!(
        manager.active_messages(&messages).len() >= 3,
        "live tail must survive a discarded stale compaction"
    );
    assert_eq!(manager.pending_cutoff, 0, "pending_cutoff must be reset");
}

#[tokio::test]
async fn test_guard_at_95_triggers_hard_compact() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("message {} with padding {}", i, "x".repeat(50)),
        ));
        manager.notify_message_added();
    }
    // 96% usage — above critical threshold
    manager.update_observed_input_tokens(960);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert!(
        matches!(action, CompactionAction::HardCompacted(_)),
        "SHOULD hard-compact at 96%"
    );
    assert!(
        manager.compacted_count > 0,
        "compacted_count should increase after hard compact"
    );
    assert!(
        manager.active_summary.is_some(),
        "should have an emergency summary"
    );
}

#[tokio::test]
async fn test_guard_at_100_percent_drops_messages() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "y".repeat(80)),
        ));
        manager.notify_message_added();
    }
    // Over 100% — simulates the exact bug scenario
    manager.update_observed_input_tokens(1_050);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert!(
        matches!(action, CompactionAction::HardCompacted(_)),
        "MUST hard-compact when over 100%"
    );

    let api_messages = manager.messages_for_api_with(&messages);
    assert!(
        api_messages.len() < messages.len(),
        "API messages should be fewer after hard compact"
    );
    // First message should be the emergency summary
    match &api_messages[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected text summary block"),
    }
}

// ── hard_compact_with edge cases ────────────────────────────────

#[test]
fn test_hard_compact_too_few_messages() {
    let mut manager = CompactionManager::new().with_budget(100);
    let messages = vec![
        make_text_message(Role::User, "hello"),
        make_text_message(Role::Assistant, "hi"),
    ];
    manager.notify_message_added();
    manager.notify_message_added();

    let result = manager.hard_compact_with(&messages);
    assert!(
        result.is_err(),
        "should fail with only 2 messages (MIN_TURNS_TO_KEEP)"
    );
}

#[test]
fn test_hard_compact_preserves_recent_turns() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..25 {
        messages.push(make_text_message(Role::User, &format!("turn {}", i)));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(950);

    let dropped = manager
        .hard_compact_with(&messages)
        .expect("should compact");
    assert!(dropped > 0, "should drop some messages");
    assert!(dropped < 25, "should not drop ALL messages");

    let api_messages = manager.messages_for_api_with(&messages);
    // Should have summary + recent turns
    assert!(
        api_messages.len() >= 2,
        "should keep at least MIN_TURNS_TO_KEEP + summary"
    );
    assert!(
        api_messages.len() <= 15,
        "should have dropped a significant number"
    );
}

// ── safe_compaction_cutoff: tool call/result pair integrity ─────────

#[test]
fn test_safe_cutoff_preserves_tool_pairs() {
    // Messages: [user, assistant(tool_use), user(tool_result), assistant, user]
    // If cutoff tries to split between tool_use and tool_result, it should back up
    let messages = vec![
        make_text_message(Role::User, "do something"),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_1".to_string(),
                content: "file1.txt\nfile2.txt".to_string(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "I see the files"),
        make_text_message(Role::User, "thanks"),
    ];

    // Try to cut between tool_use (index 1) and tool_result (index 2)
    let cutoff = safe_compaction_cutoff(&messages, 2);
    // Should move back to include the tool_use at index 1
    assert!(
        cutoff <= 1,
        "cutoff should back up to include tool_use (got {})",
        cutoff
    );
}

#[test]
fn test_safe_cutoff_no_tool_pairs() {
    let messages = vec![
        make_text_message(Role::User, "hello"),
        make_text_message(Role::Assistant, "hi"),
        make_text_message(Role::User, "how are you"),
        make_text_message(Role::Assistant, "fine"),
    ];

    let cutoff = safe_compaction_cutoff(&messages, 2);
    assert_eq!(cutoff, 2, "no tool pairs, cutoff should stay unchanged");
}

#[test]
fn test_safe_cutoff_handles_chained_tool_dependencies_without_rescan() {
    let messages = vec![
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_a".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file": "a.txt"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::User, "intermediate"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "tool_a".to_string(),
                    content: "a contents".to_string(),
                    is_error: Some(false),
                },
                ContentBlock::ToolUse {
                    id: "tool_b".to_string(),
                    name: "grep".to_string(),
                    input: serde_json::json!({"pattern": "foo"}),
                    thought_signature: None,
                },
            ],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_b".to_string(),
                content: "foo".to_string(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "done"),
    ];

    let cutoff = safe_compaction_cutoff(&messages, 3);
    assert_eq!(
        cutoff, 0,
        "cutoff should walk back through nested tool dependencies until the kept suffix is self-contained"
    );
}

// ── emergency_truncate_with ─────────────────────────────────────

#[test]
fn test_emergency_truncate_large_tool_results() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let big_result = "x".repeat(10_000); // Way over EMERGENCY_TOOL_RESULT_MAX_CHARS (4000)
    let mut messages = vec![
        make_text_message(Role::User, "run something"),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "cat bigfile"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_1".to_string(),
                content: big_result.clone(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "that's a big file"),
    ];
    for _ in &messages {
        manager.notify_message_added();
    }

    let truncated = manager.emergency_truncate_with(&mut messages);
    assert_eq!(truncated, 1, "should truncate exactly 1 tool result");

    // Check the truncated content
    if let ContentBlock::ToolResult { content, .. } = &messages[2].content[0] {
        assert!(
            content.len() < big_result.len(),
            "content should be shorter"
        );
        assert!(
            content.contains("truncated for context recovery"),
            "should have truncation marker"
        );
    } else {
        panic!("expected tool result");
    }
}

#[test]
fn test_emergency_truncate_skips_small_results() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "small output".to_string(),
            is_error: Some(false),
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    manager.notify_message_added();

    let truncated = manager.emergency_truncate_with(&mut messages);
    assert_eq!(truncated, 0, "should not truncate small results");
}

// ── Double compaction ───────────────────────────────────────────

#[test]
fn test_hard_compact_twice() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "z".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(480);

    // First hard compact
    let dropped1 = manager
        .hard_compact_with(&messages)
        .expect("first compact should work");
    assert!(dropped1 > 0);
    let count_after_first = manager.compacted_count;

    // Simulate more messages arriving after first compact
    for i in 30..45 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "z".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);

    // Second hard compact
    let dropped2 = manager
        .hard_compact_with(&messages)
        .expect("second compact should work");
    assert!(dropped2 > 0);
    assert!(
        manager.compacted_count > count_after_first,
        "compacted_count should increase"
    );

    // Summary should mention both compactions
    let api_messages = manager.messages_for_api_with(&messages);
    assert!(api_messages.len() < messages.len());
    match &api_messages[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected summary"),
    }
}

#[test]
fn test_hard_compact_clamps_pathological_compacted_count() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "x".repeat(200)),
        ));
        manager.notify_message_added();
    }

    // Reproduce the #175 bad state: bookkeeping says more messages were
    // compacted than exist in the current message vector. Before the fix,
    // active_messages() returned the full transcript in this state, so each
    // hard compaction appended another emergency marker and increased
    // compacted_count even further past messages.len().
    manager.compacted_count = 100;
    manager.active_summary = Some(Summary {
        text: "# Existing summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 100,
        original_turn_count: 100,
    });
    manager.active_chars.invalidate();

    for _ in 0..3 {
        let _ = manager.hard_compact_with(&messages);
    }

    assert_eq!(
        manager.compacted_count,
        messages.len(),
        "hard compaction must clamp compacted_count to the available messages"
    );
    let summary_markers = manager
        .active_summary
        .as_ref()
        .map(|summary| summary.text.matches("[Emergency compaction]").count())
        .unwrap_or(0);
    assert_eq!(
        summary_markers, 0,
        "pathological state should not append repeated emergency markers"
    );

    let api_messages = manager.messages_for_api_with(&messages);
    assert_eq!(
        api_messages.len(),
        1,
        "all current messages should remain covered by the existing summary until new turns arrive"
    );
}

#[test]
fn test_hard_compact_reduces_api_payload_and_reports_saved_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..40 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "payload ".repeat(80)),
        ));
        manager.notify_message_added();
    }

    let pre_api_messages = manager.messages_for_api_with(&messages);
    let pre_chars: usize = pre_api_messages.iter().map(message_char_count).sum();
    let pre_tokens = manager.effective_token_count_with(&messages);

    manager
        .hard_compact_with(&messages)
        .expect("hard compaction should recover oversized context");

    let post_api_messages = manager.messages_for_api_with(&messages);
    let post_chars: usize = post_api_messages.iter().map(message_char_count).sum();
    let post_tokens = manager.effective_token_count_with(&messages);
    let event = manager
        .take_compaction_event()
        .expect("hard compaction should publish an event");

    assert!(
        post_api_messages.len() < pre_api_messages.len(),
        "hard compaction should send fewer messages"
    );
    assert!(
        post_chars < pre_chars,
        "hard compaction should reduce outgoing payload chars: pre={pre_chars}, post={post_chars}"
    );
    assert!(
        post_tokens <= pre_tokens,
        "hard compaction must not increase effective tokens: pre={pre_tokens}, post={post_tokens}"
    );
    assert!(
        event.tokens_saved.unwrap_or(0) > 0,
        "event should attribute positive token savings: {event:?}"
    );
}

#[test]
fn test_invalid_compacted_count_does_not_resurrect_full_transcript_after_new_turn() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("old turn {} {}", i, "x".repeat(120)),
        ));
        manager.notify_message_added();
    }

    manager.compacted_count = 500;
    manager.active_summary = Some(Summary {
        text: "# Existing summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 500,
        original_turn_count: 500,
    });
    manager.active_chars.invalidate();

    let before_new_turn = manager.messages_for_api_with(&messages);
    assert_eq!(before_new_turn.len(), 1);
    assert_eq!(manager.compacted_count(), messages.len());

    messages.push(make_text_message(Role::User, "new turn after restore"));
    manager.notify_message_added();

    let after_new_turn = manager.messages_for_api_with(&messages);
    assert_eq!(
        after_new_turn.len(),
        2,
        "request should contain summary plus only the new active turn"
    );
    match &after_new_turn[1].content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "new turn after restore"),
        _ => panic!("expected new active text turn"),
    }
}

// ── messages_for_api_with after compaction ──────────────────────

#[test]
fn test_messages_for_api_with_summary_prepended() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("turn {}", i)));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);

    manager
        .hard_compact_with(&messages)
        .expect("should compact");

    let api_msgs = manager.messages_for_api_with(&messages);
    // First message should be the summary
    assert_eq!(api_msgs[0].role, Role::User);
    match &api_msgs[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.starts_with("## Previous Conversation Summary"));
        }
        _ => panic!("expected text"),
    }
    // Remaining should be recent turns from original messages
    assert!(api_msgs.len() < messages.len());
}

#[test]
fn test_persisted_state_round_trip_preserves_compacted_view() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "x".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);
    manager
        .hard_compact_with(&messages)
        .expect("should compact before persisting");

    let persisted = manager
        .persisted_state()
        .expect("compaction state should be exportable");
    let expected = manager.messages_for_api_with(&messages);

    let mut restored = CompactionManager::new().with_budget(500);
    restored.restore_persisted_state(&persisted, messages.len());
    let restored_msgs = restored.messages_for_api_with(&messages);

    assert_eq!(restored.compacted_count, persisted.compacted_count);
    assert_eq!(restored_msgs.len(), expected.len());
    match &restored_msgs[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected restored summary block"),
    }
}

// ── context_usage accuracy ──────────────────────────────────────

#[test]
fn test_context_usage_with_both_estimate_and_observed() {
    let mut manager = CompactionManager::new().with_budget(200_000);
    // Build messages totalling ~50k chars = ~12.5k token estimate
    let mut messages = Vec::new();
    for i in 0..50 {
        messages.push(make_text_message(
            Role::User,
            &format!("{} {}", i, "a".repeat(1000)),
        ));
        manager.notify_message_added();
    }

    // Without observed tokens, usage should be based on char estimate
    let usage_no_observed = manager.context_usage_with(&messages);
    assert!(
        usage_no_observed < 0.2,
        "char estimate should be low: {}",
        usage_no_observed
    );

    // With observed tokens at 160k, should use observed (higher) value
    manager.update_observed_input_tokens(160_000);
    let usage_with_observed = manager.context_usage_with(&messages);
    assert!(
        usage_with_observed >= 0.79,
        "should use observed tokens: {}",
        usage_with_observed
    );
}

#[test]
fn test_context_usage_after_compaction_resets_observed() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("msg {} pad {}", i, "x".repeat(50)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(960);

    // Hard compact should reset observed_input_tokens
    manager
        .hard_compact_with(&messages)
        .expect("should compact");
    assert!(
        manager.observed_input_tokens.is_none(),
        "observed_input_tokens should be cleared after hard compact"
    );

    // After compaction, usage should be based on char estimate of remaining messages only
    let post_usage = manager.context_usage_with(&messages);
    // The remaining messages are small, so usage should be well below the critical threshold
    assert!(
        post_usage < CRITICAL_THRESHOLD,
        "post-compaction usage should be below critical: {}",
        post_usage
    );
}

#[test]
fn test_recover_within_budget_drops_messages_without_truncation() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("msg {} pad {}", i, "x".repeat(40)),
        ));
        manager.notify_message_added();
    }
    // Push well over budget so recovery triggers.
    manager.update_observed_input_tokens(2_000);

    let recovery = manager.recover_within_budget(&mut messages);
    assert!(
        recovery.dropped.unwrap_or(0) > 0,
        "should drop old messages"
    );
    // Dropping turns alone should fit the small remaining tail, so no
    // truncation escalation is needed.
    assert_eq!(
        recovery.truncated, 0,
        "should not truncate when dropping turns fits the budget"
    );
    assert!(recovery.did_anything());
    assert!(
        manager.context_usage_with(&messages) <= 1.0,
        "context should be back under budget after recovery"
    );
}

#[test]
fn test_recover_within_budget_truncates_when_tail_still_too_large() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    // Build tool-use/tool-result pairs whose results are each individually
    // larger than the whole budget. After hard compaction drops down to the
    // minimum kept tail, the surviving tool result is still far over budget, so
    // recovery must escalate to truncation (which only acts on tool results).
    for i in 0..10 {
        let id = format!("tool_{i}");
        messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: id.clone(),
                name: "bash".to_string(),
                input: serde_json::json!({ "command": "cat big.log" }),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
        manager.notify_message_added();
        messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: id,
                content: format!("huge {} {}", i, "y".repeat(20_000)),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(50_000);

    let recovery = manager.recover_within_budget(&mut messages);
    assert!(recovery.did_anything());
    assert!(
        recovery.truncated > 0,
        "should escalate to truncation when the remaining tail is still too large"
    );
}

#[test]
fn test_recover_within_budget_summary_line_variants() {
    let dropped_only = EmergencyRecovery {
        pre_usage: 1.6,
        dropped: Some(7),
        truncated: 0,
    };
    let line = dropped_only.summary_line(dropped_only.pre_usage);
    assert!(line.contains("dropped 7 old messages"));
    assert!(line.contains("160%"));
    assert!(!line.contains("truncated"));

    let dropped_and_truncated = EmergencyRecovery {
        pre_usage: 2.0,
        dropped: Some(3),
        truncated: 2,
    };
    let line = dropped_and_truncated.summary_line(dropped_and_truncated.pre_usage);
    assert!(line.contains("dropped 3 old messages"));
    assert!(line.contains("truncated 2 tool result(s)"));

    let truncate_only = EmergencyRecovery {
        pre_usage: 1.2,
        dropped: None,
        truncated: 5,
    };
    let line = truncate_only.summary_line(truncate_only.pre_usage);
    assert!(line.contains("shortened 5 large tool result(s)"));
    assert!(!line.contains("dropped"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Compaction aggression: how much conversation survives a compaction.
//
// Before these tests existed, the kept tail was the fixed constant
// RECENT_TURNS_TO_KEEP (10 messages) regardless of context budget. On a 1M
// budget that meant ~800k tokens -> ~20k tokens, a 98% loss, observed live in
// ~/.jcode/logs. The tail is now sized against the budget. `retention_ratio_matrix`
// is the climbable metric: it prints R = post_tokens / budget per cell.
// ─────────────────────────────────────────────────────────────────────────────

/// Roughly `tokens` worth of message text (CHARS_PER_TOKEN chars per token).
fn make_sized_message(role: Role, tokens: usize, tag: usize) -> Message {
    let text = format!("[msg {tag}] {}", "x".repeat(tokens * CHARS_PER_TOKEN));
    make_text_message(role, &text)
}

/// A transcript of alternating turns totalling about `total_tokens`.
fn make_transcript(total_tokens: usize, tokens_per_msg: usize) -> Vec<Message> {
    let count = (total_tokens / tokens_per_msg).max(1);
    (0..count)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            make_sized_message(role, tokens_per_msg, i)
        })
        .collect()
}

fn manager_with_budget(budget: usize) -> CompactionManager {
    let mut manager = CompactionManager::new().with_budget(budget);
    manager.set_compaction_config(crate::config::CompactionConfig::default());
    manager
}

/// Tokens the transcript would occupy from `cutoff` onward, as compaction
/// accounts for it (summary text replaces the compacted prefix).
fn tokens_after_cutoff(messages: &[Message], cutoff: usize, budget: usize) -> usize {
    let kept_chars: usize = messages[cutoff..].iter().map(message_char_count).sum();
    jcode_compaction_core::estimate_compaction_tokens_from_chars(kept_chars, budget)
}

/// THE METRIC. Prints `budget | trigger | pre | post | R | kept_msgs` and
/// asserts the retention ratio lands in the target band.
#[test]
fn retention_ratio_matrix() {
    let budgets = [200_000usize, 400_000, 1_000_000];
    println!(
        "\n{:>10} | {:<9} | {:>9} | {:>9} | {:>6} | {:>9}",
        "budget", "trigger", "pre", "post", "R", "kept_msgs"
    );

    for budget in budgets {
        // A transcript sitting just past the compaction trigger.
        let total = (budget as f32 * 0.85) as usize;
        let messages = make_transcript(total, 2_000);
        let manager = manager_with_budget(budget);
        let pre = tokens_after_cutoff(&messages, 0, budget);

        for trigger in ["reactive", "manual", "hard"] {
            let cutoff = match trigger {
                "reactive" => manager.standard_cutoff(&messages),
                "manual" => safe_compaction_cutoff(
                    &messages,
                    keep_cutoff_for_char_budget(
                        &messages,
                        manager.keep_tail_target_chars_with(&messages) / 2,
                        manager.min_keep_turns(),
                    ),
                ),
                _ => {
                    let mut m = manager_with_budget(budget);
                    let dropped = m.hard_compact_with(&messages).expect("hard compact");
                    dropped
                }
            };
            let post = tokens_after_cutoff(&messages, cutoff, budget);
            let kept = messages.len() - cutoff;
            let r = post as f32 / budget as f32;
            println!("{budget:>10} | {trigger:<9} | {pre:>9} | {post:>9} | {r:>6.3} | {kept:>9}");

            assert!(
                post < budget,
                "{trigger} @ {budget}: post {post} must fit the budget"
            );
            let (lo, hi) = if trigger == "hard" {
                (0.20f32, 0.55f32)
            } else if trigger == "manual" {
                // Manual compaction deliberately reclaims about twice as much.
                (0.10f32, 0.40f32)
            } else {
                (0.30f32, 0.60f32)
            };
            assert!(
                r >= lo && r <= hi,
                "{trigger} @ budget {budget}: retention {r:.3} outside [{lo},{hi}] \
                 (pre={pre} post={post} kept={kept}); compaction is too aggressive \
                 if below, too timid if above"
            );
        }
    }
}

/// (b) A bigger context window must keep proportionally more conversation.
#[test]
fn keep_tail_scales_with_budget() {
    let small = 200_000usize;
    let large = 1_000_000usize;
    let messages = make_transcript(900_000, 2_000);

    let small_post = {
        let m = manager_with_budget(small);
        tokens_after_cutoff(&messages, m.standard_cutoff(&messages), small)
    };
    let large_post = {
        let m = manager_with_budget(large);
        tokens_after_cutoff(&messages, m.standard_cutoff(&messages), large)
    };

    assert!(
        large_post > small_post * 3,
        "1M budget kept {large_post} tokens but 200k kept {small_post}; \
         the tail must scale with the budget, not be a fixed message count"
    );
}

/// (d) Compacting must not leave the transcript still over the trigger, or the
/// next turn would immediately compact again (thrash).
#[test]
fn kept_tail_below_trigger_threshold() {
    for budget in [200_000usize, 400_000, 1_000_000] {
        let messages = make_transcript((budget as f32 * 0.9) as usize, 2_000);
        let manager = manager_with_budget(budget);
        let cutoff = manager.standard_cutoff(&messages);
        let post = tokens_after_cutoff(&messages, cutoff, budget);
        let r = post as f32 / budget as f32;
        assert!(
            r < COMPACTION_THRESHOLD,
            "budget {budget}: post-compaction usage {r:.3} is still at/above the \
             {COMPACTION_THRESHOLD} trigger, so compaction would fire again immediately"
        );
    }
}

/// (e) Tool calls and their results must stay together at the new, much larger
/// kept tails.
#[test]
fn new_cutoff_preserves_tool_pairs() {
    let mut messages = Vec::new();
    for i in 0..200 {
        let id = format!("tool_{i}");
        messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: id.clone(),
                name: "read".to_string(),
                input: serde_json::json!({ "path": format!("f{i}") }),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
        messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: id,
                content: "y".repeat(4_000),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
    }

    for budget in [200_000usize, 1_000_000] {
        let manager = manager_with_budget(budget);
        let cutoff = manager.standard_cutoff(&messages);
        let mut seen_ids = std::collections::HashSet::new();
        for msg in &messages[cutoff..] {
            for block in &msg.content {
                match block {
                    ContentBlock::ToolUse { id, .. } => {
                        seen_ids.insert(id.clone());
                    }
                    ContentBlock::ToolResult { tool_use_id, .. } => assert!(
                        seen_ids.contains(tool_use_id),
                        "budget {budget}: kept tool result {tool_use_id} lost its tool call \
                         at cutoff {cutoff}"
                    ),
                    _ => {}
                }
            }
        }
    }
}

/// (f) Tiny budgets must still behave like the old fixed-tail logic, so
/// constrained models and existing tests are unaffected.
#[test]
fn min_keep_floor_preserves_legacy_behavior() {
    let budget = 1_000usize;
    let messages = make_transcript(10_000, 200);
    let manager = manager_with_budget(budget);
    let kept = messages.len() - manager.standard_cutoff(&messages);
    assert!(
        kept >= MIN_TURNS_TO_KEEP && kept <= RECENT_TURNS_TO_KEEP + 2,
        "tiny budget kept {kept} messages; expected the legacy-sized floor"
    );
}

/// (g) One tool result larger than the whole budget must still force the
/// emergency descent to the minimum tail.
#[test]
fn hard_compact_descends_when_single_result_huge() {
    let budget = 200_000usize;
    let mut messages = make_transcript(50_000, 2_000);
    messages.push(make_text_message(
        Role::Assistant,
        &"z".repeat(budget * CHARS_PER_TOKEN * 2),
    ));
    messages.push(make_text_message(Role::User, "and now continue"));

    let mut manager = manager_with_budget(budget);
    let cutoff = manager.hard_compact_with(&messages).expect("hard compact");
    let kept = messages.len() - cutoff;
    assert!(
        kept <= RECENT_TURNS_TO_KEEP,
        "a single over-budget message must still force a small tail, kept {kept}"
    );
}

/// (h) The config dials actually move the cutoff.
#[test]
fn compaction_config_knobs_change_cutoff() {
    let budget = 1_000_000usize;
    let messages = make_transcript(800_000, 2_000);

    let mut stingy = manager_with_budget(budget);
    let mut cfg = crate::config::CompactionConfig::default();
    cfg.keep_fraction = 0.10;
    stingy.set_compaction_config(cfg);

    let mut generous = manager_with_budget(budget);
    let mut cfg = crate::config::CompactionConfig::default();
    cfg.keep_fraction = 0.55;
    generous.set_compaction_config(cfg);

    let stingy_kept = messages.len() - stingy.standard_cutoff(&messages);
    let generous_kept = messages.len() - generous.standard_cutoff(&messages);
    assert!(
        generous_kept > stingy_kept,
        "keep_fraction had no effect: {generous_kept} vs {stingy_kept}"
    );

    // keep_fraction is clamped so a wild value can never park the transcript
    // at or above the compaction trigger.
    let mut wild = manager_with_budget(budget);
    let mut cfg = crate::config::CompactionConfig::default();
    cfg.keep_fraction = 5.0;
    wild.set_compaction_config(cfg);
    let post = tokens_after_cutoff(&messages, wild.standard_cutoff(&messages), budget);
    assert!(
        (post as f32 / budget as f32) < COMPACTION_THRESHOLD,
        "an out-of-range keep_fraction must still be clamped below the trigger"
    );

    // min_keep_turns is a floor even when the fraction is ~0.
    let mut floored = manager_with_budget(budget);
    let mut cfg = crate::config::CompactionConfig::default();
    cfg.keep_fraction = 0.0;
    cfg.min_keep_turns = 25;
    floored.set_compaction_config(cfg);
    let kept = messages.len() - floored.standard_cutoff(&messages);
    assert_eq!(kept, 25, "min_keep_turns floor not honoured");
}

/// (i) Overhead-dominated windows: when the system prompt and tool definitions
/// alone consume most of the context, there is genuinely little room left for
/// conversation and the tail must collapse to the floor rather than pretending
/// otherwise. Regression for a live 32k-window session where tools+system cost
/// ~25k: the kept tail must not be sized against the raw budget.
#[test]
fn overhead_dominated_window_falls_back_to_floor() {
    let budget = 32_000usize;
    let messages = make_transcript(20_000, 1_000);
    let mut manager = manager_with_budget(budget);
    // Provider reports far more than the chars imply: the gap is tools/system.
    manager.update_observed_input_tokens(30_000);

    let cutoff = manager.standard_cutoff_with(&messages, &messages);
    let kept = messages.len() - cutoff;
    assert!(
        kept >= MIN_TURNS_TO_KEEP,
        "must always keep the minimum tail, kept {kept}"
    );

    // And the result must still leave headroom under the trigger.
    let overhead = 30_000usize
        .saturating_sub(messages.iter().map(message_char_count).sum::<usize>() / CHARS_PER_TOKEN);
    let kept_tokens: usize = messages[cutoff..]
        .iter()
        .map(message_char_count)
        .sum::<usize>()
        / CHARS_PER_TOKEN;
    let total = kept_tokens + overhead;
    assert!(
        total < budget,
        "post-compaction total {total} must fit the {budget} window"
    );
}

/// (j) A roomy window must not be dragged down by the overhead correction: the
/// tail should still be budget-proportional when there is real room.
#[test]
fn roomy_window_keeps_proportional_tail_despite_overhead() {
    let budget = 1_000_000usize;
    let messages = make_transcript(800_000, 2_000);
    let mut manager = manager_with_budget(budget);
    manager.update_observed_input_tokens(820_000);

    let cutoff = manager.standard_cutoff_with(&messages, &messages);
    let kept_tokens: usize = messages[cutoff..]
        .iter()
        .map(message_char_count)
        .sum::<usize>()
        / CHARS_PER_TOKEN;
    let r = kept_tokens as f32 / budget as f32;
    assert!(
        r > 0.25,
        "a 1M window with modest overhead should still keep a large tail, got R={r:.3}"
    );
}

/// (k) Regression for a live 70k-window session where emergency compaction
/// dropped 16 of 18 messages, leaving 2.2% of context.
///
/// The cause was the fallback halving the kept turns (10 -> 5 -> 2) rather than
/// searching for the largest tail that fits. Halving can only ever land on a
/// power-of-two division of the floor, so whenever the true best tail sits
/// between two of those steps it overshoots all the way down.
///
/// Here each message is ~7k tokens against a ~31.5k target: four fit, five do
/// not. Halving tries 10 (too big), 5 (too big), then gives up at 2. A greedy
/// char-budget fit keeps 4, twice as much conversation.
#[test]
fn hard_compact_finds_largest_fitting_tail_not_halved_floor() {
    let budget = 70_000usize;
    let messages: Vec<Message> = (0..18)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            make_sized_message(role, 7_000, i)
        })
        .collect();

    let mut manager = manager_with_budget(budget);
    let cutoff = manager.hard_compact_with(&messages).expect("hard compact");
    let kept = messages.len() - cutoff;
    let kept_tokens: usize = messages[cutoff..]
        .iter()
        .map(message_char_count)
        .sum::<usize>()
        / CHARS_PER_TOKEN;

    assert!(
        kept > MIN_TURNS_TO_KEEP,
        "kept only {kept} of {} messages ({kept_tokens} tokens in a {budget} window); \
         the fallback collapsed to the floor instead of keeping the largest fitting tail",
        messages.len()
    );
    assert!(
        kept_tokens < budget,
        "kept tail {kept_tokens} must still fit the {budget} window"
    );
}

/// (l) End-to-end proof at Ben's actual window size.
///
/// Every live verification used a pinned small context window, because a real
/// 1M-token session takes hours to reach the trigger. This test closes that gap
/// by driving the full hard-compact path at 1M with realistic overhead and
/// asserting on the same numbers the `[compaction/outcome]` log line reports.
///
/// It guards exactly what Ben saw in his logs:
/// `pre_tokens=1048885 post_tokens=20217 messages_dropped=3115`. With the old
/// fixed-tail cutoff restored this test reproduces that symptom (kept=10,
/// R=0.003) and fails.
#[test]
fn one_million_window_end_to_end_keeps_large_tail() {
    let budget = 1_000_000usize;
    let messages = make_transcript(950_000, 300);
    let mut manager = manager_with_budget(budget);
    // Provider reports the real size, including system prompt + tools.
    manager.update_observed_input_tokens(1_010_000);

    let pre_active = messages.len();
    let dropped = manager.hard_compact_with(&messages).expect("hard compact");
    let kept = pre_active - dropped;
    let kept_tokens: usize = messages[dropped..]
        .iter()
        .map(message_char_count)
        .sum::<usize>()
        / CHARS_PER_TOKEN;
    let r = kept_tokens as f32 / budget as f32;

    println!(
        "1M end-to-end: pre_msgs={pre_active} dropped={dropped} kept={kept} \
         kept_tokens={kept_tokens} R={r:.3}"
    );

    assert!(
        kept > 100,
        "kept only {kept} of {pre_active} messages at a 1M budget; \
         this is the RECENT_TURNS_TO_KEEP collapse regressing"
    );
    assert!(
        r > 0.25,
        "retention {r:.3} at a 1M budget is back in amnesia territory"
    );
    assert!(
        r < COMPACTION_THRESHOLD,
        "retention {r:.3} would re-trigger compaction immediately"
    );
}

/// (m) Compaction must settle: after it runs, the manager must no longer want
/// to compact. This is the end-to-end form of the anti-thrash invariant,
/// exercised through the real trigger rather than by inspecting a cutoff.
///
/// Verified to fail if both the `keep_fraction` clamp and `ANTI_THRASH_CEILING`
/// are removed, which is the configuration that would thrash.
#[test]
fn compaction_settles_and_does_not_immediately_refire() {
    for budget in [200_000usize, 1_000_000] {
        let messages = make_transcript((budget as f32 * 0.9) as usize, 2_000);
        let mut manager = manager_with_budget(budget);
        manager.update_observed_input_tokens((budget as f64 * 0.96) as u64);

        assert!(
            manager.should_compact_with(&messages),
            "budget {budget}: a 96%-full context should want to compact"
        );

        manager.hard_compact_with(&messages).expect("hard compact");

        assert!(
            !manager.should_compact_with(&messages),
            "budget {budget}: compaction did not settle; it wants to compact again \
             immediately, which is the thrash loop"
        );
    }
}
