//! OpenAI Chat Completions 스트림 (`chat.completion.chunk` ... `[DONE]`)

use serde_json::{Value, json};

use super::{data, random_id, unix_now};

pub fn frames(chunks: &[String], model: &str) -> Vec<String> {
    let id = format!("chatcmpl-{}", random_id(29));
    let created = unix_now();
    let chunk = |delta: Value, finish_reason: Value| {
        data(&json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "system_fingerprint": null,
            "choices": [{
                "index": 0,
                "delta": delta,
                "logprobs": null,
                "finish_reason": finish_reason,
            }],
        }))
    };

    let mut frames = Vec::with_capacity(chunks.len() + 3);
    frames.push(chunk(
        json!({ "role": "assistant", "content": "" }),
        Value::Null,
    ));
    for text in chunks {
        frames.push(chunk(json!({ "content": text }), Value::Null));
    }
    frames.push(chunk(json!({}), json!("stop")));
    frames.push("data: [DONE]\n\n".to_string());
    frames
}
