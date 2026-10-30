//! POST /api/echo — 붙여넣은 텍스트를 LLM이 생성하듯 조각내서 스트리밍

use std::time::Duration;

use axum::{Json, response::Response};
use serde::Deserialize;

use crate::formats::Format;
use crate::stream::sse_response;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkMode {
    /// 단어 단위, 앞의 공백은 다음 단어에 붙임 ("Hello", " world")
    #[default]
    Word,
    Char,
    Line,
}

#[derive(Debug, Deserialize)]
pub struct EchoRequest {
    pub text: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub chunk: ChunkMode,
    /// 프레임 사이 기본 지연
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    /// 기본 지연에 더해질 0..=jitter_ms 범위의 무작위 지연
    #[serde(default = "default_jitter_ms")]
    pub jitter_ms: u64,
    /// 응답에 표시할 모델명 (없으면 포맷별 기본값)
    pub model: Option<String>,
}

fn default_delay_ms() -> u64 {
    30
}

fn default_jitter_ms() -> u64 {
    20
}

pub async fn echo(Json(req): Json<EchoRequest>) -> Response {
    let chunks = split(&req.text, req.chunk);
    let model = req
        .model
        .as_deref()
        .unwrap_or_else(|| req.format.default_model());

    let frames = req
        .format
        .frames(&chunks, model)
        .into_iter()
        .enumerate()
        .map(|(i, frame)| {
            let wait = if i == 0 {
                0
            } else {
                req.delay_ms + fastrand::u64(0..=req.jitter_ms)
            };
            (Duration::from_millis(wait), frame)
        })
        .collect();

    sse_response(frames)
}

pub fn split(text: &str, mode: ChunkMode) -> Vec<String> {
    match mode {
        ChunkMode::Char => text.chars().map(String::from).collect(),
        ChunkMode::Line => text.split_inclusive('\n').map(String::from).collect(),
        ChunkMode::Word => {
            let mut chunks = Vec::new();
            let mut current = String::new();
            let mut prev_whitespace = false;
            for c in text.chars() {
                // 단어가 끝나고 공백이 시작되는 지점에서 끊어, 공백이 다음 단어 앞에 붙게 한다
                if c.is_whitespace() && !prev_whitespace && !current.trim().is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
                current.push(c);
                prev_whitespace = c.is_whitespace();
            }
            if !current.is_empty() {
                chunks.push(current);
            }
            chunks
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_data(frame: &str) -> serde_json::Value {
        let line = frame.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn word_split_keeps_leading_whitespace_on_next_word() {
        assert_eq!(
            split("  Hello world\n\n안녕 하세요", ChunkMode::Word),
            ["  Hello", " world", "\n\n안녕", " 하세요"]
        );
    }

    #[test]
    fn splits_are_lossless() {
        let text = "a b\n c  d \n";
        for mode in [ChunkMode::Word, ChunkMode::Char, ChunkMode::Line] {
            assert_eq!(split(text, mode).concat(), text, "{mode:?}");
        }
    }

    #[test]
    fn openai_frames() {
        let chunks = split("Hi there", ChunkMode::Word);
        let frames = Format::Openai.frames(&chunks, "m");
        assert_eq!(frames.len(), 5);
        assert_eq!(parse_data(&frames[0])["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(parse_data(&frames[2])["choices"][0]["delta"]["content"], " there");
        assert_eq!(parse_data(&frames[3])["choices"][0]["finish_reason"], "stop");
        assert_eq!(frames[4], "data: [DONE]\n\n");
    }

    #[test]
    fn claude_frames() {
        let chunks = split("Hi there", ChunkMode::Word);
        let frames = Format::Claude.frames(&chunks, "m");
        let events: Vec<_> = frames
            .iter()
            .map(|f| f.lines().next().unwrap().trim_start_matches("event: "))
            .collect();
        assert_eq!(
            events,
            [
                "message_start",
                "content_block_start",
                "ping",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        assert_eq!(parse_data(&frames[4])["delta"]["text"], " there");
    }

    #[test]
    fn gemini_frames() {
        let chunks = split("Hi there", ChunkMode::Word);
        let frames = Format::Gemini.frames(&chunks, "m");
        assert_eq!(frames.len(), 2);
        let first = parse_data(&frames[0]);
        assert_eq!(first["candidates"][0]["content"]["parts"][0]["text"], "Hi");
        assert!(first["candidates"][0].get("finishReason").is_none());
        assert_eq!(parse_data(&frames[1])["candidates"][0]["finishReason"], "STOP");
    }

    #[test]
    fn gemini_empty_text_still_finishes() {
        let frames = Format::Gemini.frames(&[], "m");
        assert_eq!(frames.len(), 1);
        assert_eq!(parse_data(&frames[0])["candidates"][0]["finishReason"], "STOP");
    }
}
