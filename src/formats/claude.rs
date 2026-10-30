//! Anthropic Messages 스트림 (message_start → content_block_* → message_delta → message_stop)

use serde_json::json;

use super::{event, random_id};

pub fn frames(chunks: &[String], model: &str) -> Vec<String> {
    let mut frames = Vec::with_capacity(chunks.len() + 6);

    frames.push(event(
        "message_start",
        &json!({
            "type": "message_start",
            "message": {
                "id": format!("msg_{}", random_id(24)),
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": { "input_tokens": 0, "output_tokens": 1 },
            },
        }),
    ));
    frames.push(event(
        "content_block_start",
        &json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "text", "text": "" },
        }),
    ));
    frames.push(event("ping", &json!({ "type": "ping" })));

    for text in chunks {
        frames.push(event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": text },
            }),
        ));
    }

    frames.push(event(
        "content_block_stop",
        &json!({ "type": "content_block_stop", "index": 0 }),
    ));
    frames.push(event(
        "message_delta",
        &json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": chunks.len() },
        }),
    ));
    frames.push(event("message_stop", &json!({ "type": "message_stop" })));
    frames
}
