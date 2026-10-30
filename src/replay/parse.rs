//! 붙여넣은 스트림 캡처를 재생 가능한 SSE 프레임 목록으로 변환
//!
//! - sse: 서버가 보낸 원문 (`event: ...` / `data: ...`, 빈 줄로 구분). 프레임을 그대로 보존
//! - devtools: 크롬 DevTools EventStream 탭 복사본 (`[Id\t]Type\tData\tTime` 행)
//! - json: 이벤트 배열 (문자열, 페이로드 객체, `{event, data, time}` 객체)

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    #[default]
    Auto,
    Sse,
    Devtools,
    Json,
}

impl InputType {
    pub fn as_str(self) -> &'static str {
        match self {
            InputType::Auto => "auto",
            InputType::Sse => "sse",
            InputType::Devtools => "devtools",
            InputType::Json => "json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    /// 그대로 전송할 SSE 프레임 ("\n\n"으로 끝남)
    pub frame: String,
    /// 캡처 시각(ms). 이벤트 간 간격 계산에만 쓰므로 기준점은 상관없음
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<f64>,
    /// 이 이벤트 전에 기다릴 시간이 명시된 경우
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
}

impl Event {
    fn new(frame: String) -> Self {
        Event { frame, time_ms: None, delay_ms: None }
    }
}

/// 입력을 파싱하고, 실제로 사용한 입력 형식을 함께 돌려준다 (auto일 때 감지 결과)
pub fn parse(raw: &str, input_type: InputType) -> Result<(InputType, Vec<Event>), String> {
    let raw = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let raw = raw.trim();

    let (detected, events) = match input_type {
        InputType::Auto => detect(raw),
        InputType::Sse => (InputType::Sse, sse(raw)),
        InputType::Devtools => (InputType::Devtools, devtools(raw)),
        InputType::Json => (InputType::Json, json(raw)?),
    };

    if events.is_empty() {
        return Err(format!("입력에서 이벤트를 찾지 못했습니다 (형식: {})", detected.as_str()));
    }
    Ok((detected, events))
}

fn detect(raw: &str) -> (InputType, Vec<Event>) {
    if raw.starts_with(['[', '{']) {
        if let Ok(events) = json(raw) {
            return (InputType::Json, events);
        }
    }
    let has_sse_fields = raw.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("data:") || l.starts_with("event:")
    });
    if has_sse_fields {
        (InputType::Sse, sse(raw))
    } else {
        // NDJSON(줄마다 JSON 하나)도 여기서 줄 단위 data 이벤트로 처리된다
        (InputType::Devtools, devtools(raw))
    }
}

/// 이벤트 하나를 SSE 프레임 텍스트로 만든다 (기본 이벤트명 "message"는 생략)
fn frame(event: Option<&str>, id: Option<&str>, data: &str) -> String {
    let mut out = String::new();
    if let Some(id) = id.filter(|s| !s.is_empty()) {
        out.push_str(&format!("id: {id}\n"));
    }
    if let Some(event) = event.filter(|s| !s.is_empty() && *s != "message") {
        out.push_str(&format!("event: {event}\n"));
    }
    if data.is_empty() {
        out.push_str("data: \n");
    }
    for line in data.lines() {
        out.push_str(&format!("data: {line}\n"));
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------- sse

fn sse(raw: &str) -> Vec<Event> {
    // 공백만 있는 줄도 프레임 구분자로 본다
    let normalized: Vec<&str> = raw
        .lines()
        .map(|l| if l.trim().is_empty() { "" } else { l })
        .collect();
    let normalized = normalized.join("\n");

    normalized
        .split("\n\n")
        .map(|block| block.trim_matches('\n'))
        .filter(|block| !block.is_empty())
        .flat_map(split_collapsed)
        .map(|f| Event::new(format!("{f}\n\n")))
        .collect()
}

/// 복사 과정에서 빈 줄이 사라져 여러 이벤트가 한 블록으로 붙은 경우를 다시 나눈다.
/// - `event:` 줄이 둘 이상이면 각 `event:` 앞에서 나눔
/// - `event:` 없이 `data:` 줄이 여럿이고 각각이 완결된 JSON(또는 [DONE])이면 줄마다 나눔
fn split_collapsed(block: &str) -> Vec<String> {
    let lines: Vec<&str> = block.lines().collect();
    let is_event = |l: &str| l.starts_with("event:");
    let is_data = |l: &str| l.starts_with("data:");

    let event_lines = lines.iter().filter(|l| is_event(l)).count();
    let data_lines: Vec<&str> = lines.iter().copied().filter(|l| is_data(l)).collect();

    let boundary: &dyn Fn(&str) -> bool = if event_lines > 1 {
        &is_event
    } else if event_lines == 0 && data_lines.len() > 1 && data_lines.iter().all(|l| complete_data(l)) {
        &is_data
    } else {
        return vec![block.to_string()];
    };

    let mut frames = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in lines {
        if boundary(line) && current.iter().any(|l| boundary(l)) {
            frames.push(current.join("\n"));
            current.clear();
        }
        current.push(line);
    }
    if !current.is_empty() {
        frames.push(current.join("\n"));
    }
    frames
}

fn complete_data(line: &str) -> bool {
    let value = line.trim_start_matches("data:").trim();
    value == "[DONE]" || serde_json::from_str::<Value>(value).is_ok()
}

// ---------------------------------------------------------------- devtools

fn devtools(raw: &str) -> Vec<Event> {
    raw.lines()
        .filter_map(devtools_line)
        .map(|row| Event {
            frame: frame(row.event.as_deref(), row.id.as_deref(), &row.data),
            time_ms: row.time_ms,
            delay_ms: None,
        })
        .collect()
}

struct Row {
    id: Option<String>,
    event: Option<String>,
    data: String,
    time_ms: Option<f64>,
}

fn devtools_line(line: &str) -> Option<Row> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let row = if line.contains('\t') {
        // [Id] Type Data Time — 뒤에서부터 채운다
        let mut fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        // 표 머리글 행 (Id / Type / Data / Time)
        let header_names = ["id", "type", "data", "time"];
        if fields.iter().all(|f| header_names.iter().any(|h| f.eq_ignore_ascii_case(h))) {
            return None;
        }
        let time_ms = fields.last().and_then(|f| parse_clock(f));
        if time_ms.is_some() {
            fields.pop();
        }
        let data = fields.pop()?.to_string();
        let event = fields.pop().filter(|s| !s.is_empty()).map(String::from);
        let id = fields.pop().filter(|s| !s.is_empty()).map(String::from);
        Row { id, event, data, time_ms }
    } else {
        // 탭이 없으면 "message {json} 12:34:56.789" 또는 줄 전체를 data로 본다
        let (rest, time_ms) = match line.rsplit_once(' ') {
            Some((rest, last)) => match parse_clock(last) {
                Some(ms) => (rest.trim_end(), Some(ms)),
                None => (line, None),
            },
            None => (line, None),
        };
        let (event, data) = match rest.split_once(' ') {
            Some((first, data)) if !looks_like_json(first) && looks_like_json(data.trim_start()) => {
                (Some(first.to_string()), data.trim_start())
            }
            _ => (None, rest),
        };
        Row { id: None, event, data: data.to_string(), time_ms }
    };
    Some(row)
}

fn looks_like_json(s: &str) -> bool {
    s.starts_with(['{', '[', '"'])
}

/// "HH:MM:SS(.fff)" → 자정 기준 ms
fn parse_clock(s: &str) -> Option<f64> {
    let mut parts = s.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    let sec: f64 = sec.parse().ok()?;
    Some(f64::from(h * 3600 + m * 60) * 1000.0 + sec * 1000.0)
}

// ---------------------------------------------------------------- json

const ENVELOPE_KEYS: &[&str] = &["data", "event", "type", "id", "time", "timestamp", "delay_ms", "retry"];

fn json(raw: &str) -> Result<Vec<Event>, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| format!("JSON 파싱 실패: {e}"))?;
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    Ok(items.iter().map(json_event).collect())
}

fn json_event(item: &Value) -> Event {
    match item {
        // "data:" 뒤에 들어갈 값 그대로 ("[DONE]" 포함)
        Value::String(data) => Event::new(frame(None, None, data)),
        Value::Object(obj) if is_envelope(obj) => {
            let data = match &obj["data"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let event = obj.get("event").or_else(|| obj.get("type")).and_then(Value::as_str);
            let id = obj.get("id").and_then(Value::as_str);
            Event {
                frame: frame(event, id, &data),
                time_ms: obj.get("time").or_else(|| obj.get("timestamp")).and_then(json_time),
                delay_ms: obj.get("delay_ms").and_then(Value::as_u64),
            }
        }
        // 페이로드 객체 자체. "type" 필드가 있으면 (Anthropic, OpenAI Responses처럼) event 이름으로도 쓴다
        payload => {
            let event = payload.get("type").and_then(Value::as_str);
            Event::new(frame(event, None, &payload.to_string()))
        }
    }
}

fn is_envelope(obj: &Map<String, Value>) -> bool {
    obj.contains_key("data") && obj.keys().all(|k| ENVELOPE_KEYS.contains(&k.as_str()))
}

fn json_time(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_clock(s).or_else(|| s.parse().ok()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(events: &[Event]) -> Vec<&str> {
        events.iter().map(|e| e.frame.as_str()).collect()
    }

    #[test]
    fn sse_keeps_frames_verbatim() {
        let raw = "event: message_start\r\ndata: {\"a\":1}\r\n\r\n: keep-alive\r\n\r\nevent: ping\r\ndata: {}\r\n\r\n";
        let (ty, events) = parse(raw, InputType::Auto).unwrap();
        assert_eq!(ty, InputType::Sse);
        assert_eq!(
            frames(&events),
            ["event: message_start\ndata: {\"a\":1}\n\n", ": keep-alive\n\n", "event: ping\ndata: {}\n\n"]
        );
    }

    #[test]
    fn sse_splits_collapsed_blocks() {
        let raw = "data: {\"a\":1}\ndata: {\"a\":2}\ndata: [DONE]";
        let (_, events) = parse(raw, InputType::Auto).unwrap();
        assert_eq!(frames(&events), ["data: {\"a\":1}\n\n", "data: {\"a\":2}\n\n", "data: [DONE]\n\n"]);

        let raw = "event: a\ndata: {}\nevent: b\ndata: {}";
        let (_, events) = parse(raw, InputType::Sse).unwrap();
        assert_eq!(frames(&events), ["event: a\ndata: {}\n\n", "event: b\ndata: {}\n\n"]);
    }

    #[test]
    fn sse_keeps_multiline_data() {
        let raw = "data: hello\ndata: world\n\n";
        let (_, events) = parse(raw, InputType::Sse).unwrap();
        assert_eq!(frames(&events), ["data: hello\ndata: world\n\n"]);
    }

    #[test]
    fn devtools_tab_rows() {
        let raw = "Type\tData\tTime\n\
                   message\t{\"x\":1}\t12:00:00.100\n\
                   content_block_delta\t{\"x\":2}\t12:00:00.350\n\
                   message\t[DONE]\t12:00:01.000\n";
        let (ty, events) = parse(raw, InputType::Auto).unwrap();
        assert_eq!(ty, InputType::Devtools);
        assert_eq!(
            frames(&events),
            ["data: {\"x\":1}\n\n", "event: content_block_delta\ndata: {\"x\":2}\n\n", "data: [DONE]\n\n"]
        );
        let times: Vec<_> = events.iter().map(|e| e.time_ms.unwrap()).collect();
        assert_eq!(times, [43_200_100.0, 43_200_350.0, 43_201_000.0]);
    }

    #[test]
    fn devtools_with_id_column() {
        let (_, events) = parse("7\tping\t{}\t09:15:00.000", InputType::Devtools).unwrap();
        assert_eq!(frames(&events), ["id: 7\nevent: ping\ndata: {}\n\n"]);
    }

    #[test]
    fn devtools_space_separated_and_ndjson() {
        let raw = "message {\"a\": 1} 12:00:00.000\n{\"b\": 2}";
        let (ty, events) = parse(raw, InputType::Auto).unwrap();
        assert_eq!(ty, InputType::Devtools);
        assert_eq!(frames(&events), ["data: {\"a\": 1}\n\n", "data: {\"b\": 2}\n\n"]);
        assert_eq!(events[0].time_ms, Some(43_200_000.0));
        assert_eq!(events[1].time_ms, None);
    }

    #[test]
    fn json_strings_and_payloads() {
        let raw = r#"[
            {"type":"message_start","message":{}},
            {"choices":[{"delta":{"content":"hi"}}]},
            "[DONE]"
        ]"#;
        let (ty, events) = parse(raw, InputType::Auto).unwrap();
        assert_eq!(ty, InputType::Json);
        assert_eq!(
            frames(&events),
            [
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
                "data: [DONE]\n\n",
            ]
        );
    }

    #[test]
    fn json_envelopes() {
        let raw = r#"[
            {"event":"ping","data":{"type":"ping"},"time":"10:00:00.000"},
            {"data":"[DONE]","delay_ms":500}
        ]"#;
        let (_, events) = parse(raw, InputType::Json).unwrap();
        assert_eq!(frames(&events), ["event: ping\ndata: {\"type\":\"ping\"}\n\n", "data: [DONE]\n\n"]);
        assert_eq!(events[0].time_ms, Some(36_000_000.0));
        assert_eq!(events[1].delay_ms, Some(500));
    }

    #[test]
    fn empty_input_is_error() {
        assert!(parse("  \n ", InputType::Auto).is_err());
        assert!(parse("not json", InputType::Json).is_err());
    }
}
