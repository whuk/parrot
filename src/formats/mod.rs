//! 텍스트 조각 목록을 각 LLM 공급자의 스트리밍 응답 프레임으로 변환

pub mod claude;
pub mod gemini;
pub mod openai;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    #[serde(alias = "gpt")]
    Openai,
    #[serde(alias = "anthropic")]
    Claude,
    Gemini,
}

impl Format {
    pub fn default_model(self) -> &'static str {
        match self {
            Format::Openai => "gpt-4o",
            Format::Claude => "claude-sonnet-4-5",
            Format::Gemini => "gemini-2.5-flash",
        }
    }

    /// 조각 목록을 이 포맷의 SSE 프레임 목록으로 변환 (각 프레임은 "\n\n"으로 끝남)
    pub fn frames(self, chunks: &[String], model: &str) -> Vec<String> {
        match self {
            Format::Openai => openai::frames(chunks, model),
            Format::Claude => claude::frames(chunks, model),
            Format::Gemini => gemini::frames(chunks, model),
        }
    }
}

/// `data: {...}` 프레임
fn data(value: &Value) -> String {
    format!("data: {value}\n\n")
}

/// `event: name` + `data: {...}` 프레임
fn event(name: &str, value: &Value) -> String {
    format!("event: {name}\ndata: {value}\n\n")
}

fn random_id(len: usize) -> String {
    std::iter::repeat_with(fastrand::alphanumeric)
        .take(len)
        .collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
