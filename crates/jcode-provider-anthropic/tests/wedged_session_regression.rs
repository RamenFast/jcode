//! Regression proof against the real wedged session.
//!
//! `session_rabbit_1785214770012_f492451ded8d3f49` recorded a `read` of a
//! zero-byte `/tmp/editor.png`, which stored `ContentBlock::Image{data:""}` in
//! permanent history. Anthropic answered every subsequent request with
//! `image.source.base64: image cannot be empty`, so retry, `/poke`, and model
//! fallback all failed on the same block.
//!
//! This test replays that block sequence verbatim (as captured from the session
//! journal) and asserts the wire payload is now clean.

use jcode_message_types::ContentBlock;
use jcode_provider_anthropic::format_content_blocks;

#[test]
fn wedged_session_block_sequence_serializes_without_an_empty_image() {
    // Verbatim from the journal: tool_result, empty image, attached-image label.
    let blocks = vec![
        ContentBlock::ToolResult {
            tool_use_id: "toolu_01NX7WFMv2VQBh7jgRiLa5cH".to_string(),
            content:
                "Image: /tmp/editor.png (0 bytes)\nDimensions: unknown\nImage sent to model for vision analysis."
                    .to_string(),
            is_error: None,
        },
        ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: String::new(),
        },
        ContentBlock::Text {
            text: "[Attached image associated with the preceding tool result: /tmp/editor.png]"
                .to_string(),
            cache_control: None,
        },
    ];

    let json = serde_json::to_string(&format_content_blocks(&blocks, false)).expect("serialize");

    assert!(
        !json.contains(r#""data":"""#),
        "the exact 400-triggering payload is still emitted: {json}"
    );
    assert!(
        json.contains("toolu_01NX7WFMv2VQBh7jgRiLa5cH"),
        "the tool result must survive so its tool_use keeps its pair: {json}"
    );
}

/// Best-effort sweep over the machine's own session store.
///
/// Session files are live user state: they rotate, compact, and disappear, so
/// this test never *requires* a poisoned session to exist. When one is present
/// it proves the real recorded bytes no longer serialize an empty image; when
/// none is present it simply has nothing to prove and passes. The hermetic test
/// above is the actual regression pin.
#[test]
fn recorded_sessions_never_serialize_an_empty_image() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = std::path::Path::new(&home).join(".jcode/sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };

    let mut poisoned_blocks_seen = 0usize;
    let mut messages_checked = 0usize;

    // Filter before bounding: the session directory holds thousands of files,
    // so truncating the listing first would usually skip the very sessions this
    // test exists to check.
    let poisoned: Vec<(std::path::PathBuf, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let text = std::fs::read_to_string(&path).ok()?;
            // Only sessions that actually recorded an empty image are
            // interesting, and parsing every session would be needlessly slow.
            text.contains(r#""data":"""#).then_some((path, text))
        })
        .collect();

    for (path, text) in poisoned {
        for value in text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        {
            // `.journal.jsonl` appends live under "append_messages"; the
            // rolled-up `.json` store keeps the whole history in "messages".
            let message_lists = ["append_messages", "messages"]
                .into_iter()
                .filter_map(|key| value.get(key))
                .filter_map(|v| v.as_array());

            for messages in message_lists {
                for message in messages {
                    let Some(content) = message.get("content") else {
                        continue;
                    };
                    let Ok(blocks) = serde_json::from_value::<Vec<ContentBlock>>(content.clone())
                    else {
                        continue;
                    };
                    poisoned_blocks_seen += blocks
                        .iter()
                        .filter(|b| {
                            matches!(b, ContentBlock::Image { data, .. } if data.trim().is_empty())
                        })
                        .count();

                    let json = serde_json::to_string(&format_content_blocks(&blocks, false))
                        .expect("serialize");
                    assert!(
                        !json.contains(r#""data":"""#),
                        "recorded session {} still yields an empty image on the wire",
                        path.display()
                    );
                    messages_checked += 1;
                }
            }
        }
    }

    // Not an assertion, just a signal in the test log about what was covered.
    println!(
        "checked {messages_checked} recorded messages, {poisoned_blocks_seen} carrying an empty image"
    );
}
