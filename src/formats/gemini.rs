//! Gemini `streamGenerateContent?alt=sse` 스트림 ([DONE] 없이 마지막 청크에 finishReason)

use serde_json::{Value, json};

use super::{data, random_id};

pub fn frames(chunks: &[String], model: &str) -> Vec<String> {
    let response_id = random_id(22);
    let empty = [String::new()];
    // 빈 입력이어도 finishReason을 담은 청크 하나는 보낸다
    let chunks = if chunks.is_empty() {
        &empty[..]
    } else {
        chunks
    };
    let last = chunks.len() - 1;

    chunks
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut candidate = json!({
                "content": { "parts": [{ "text": text }], "role": "model" },
                "index": 0,
            });
            let mut body = json!({
                "candidates": [],
                "usageMetadata": {
                    "promptTokenCount": 0,
                    "candidatesTokenCount": i + 1,
                    "totalTokenCount": i + 1,
                },
                "modelVersion": model,
                "responseId": response_id,
            });
            if i == last {
                candidate["finishReason"] = Value::from("STOP");
            }
            body["candidates"] = json!([candidate]);
            data(&body)
        })
        .collect()
}
