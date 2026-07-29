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
