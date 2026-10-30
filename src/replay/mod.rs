//! POST /api/replay — 캡처한 LLM 스트림을 같은 형식 그대로 다시 스트리밍
//!
//! 본문은 두 가지 방식 중 하나:
//! - `application/json` + `{"raw": "...", ...옵션}`
//! - 캡처 원문 그대로 (text/plain 등) + 옵션은 쿼리스트링 (`?timing=fixed&delay_ms=50`)

mod parse;

use std::time::Duration;

use axum::{
    Json,
    extract::Query,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::stream::sse_response;
use parse::{Event, InputType};

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Timing {
    /// 캡처에 시각/지연 정보가 있으면 original, 없으면 fixed
    #[default]
    Auto,
    /// 캡처된 이벤트 간격 그대로 (speed 배속 적용)
    Original,
    /// delay_ms (+ jitter_ms) 고정 간격
    Fixed,
}

#[derive(Debug, Deserialize)]
pub struct ReplayOptions {
    #[serde(default)]
    pub input_type: InputType,
    #[serde(default)]
    pub timing: Timing,
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    #[serde(default)]
    pub jitter_ms: u64,
    /// original 타이밍 배속 (2.0이면 두 배 빠르게)
    #[serde(default = "default_speed")]
    pub speed: f64,
    /// original 타이밍에서 이벤트 사이 최대 대기 시간
    pub max_gap_ms: Option<u64>,
}

fn default_delay_ms() -> u64 {
    30
}

fn default_speed() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct ReplayRequest {
    #[serde(alias = "input")]
    raw: String,
    #[serde(flatten)]
    options: ReplayOptions,
}

pub async fn replay(query: Query<ReplayOptions>, headers: HeaderMap, body: String) -> Response {
    let (detected, events, waits) = match prepare(query, &headers, body) {
        Ok(prepared) => prepared,
        Err(message) => return bad_request(message),
    };

    let count = events.len();
    let frames = waits
        .into_iter()
        .zip(events.into_iter().map(|e| e.frame))
        .collect();
    let mut response = sse_response(frames);
    let response_headers = response.headers_mut();
    response_headers.insert(
        "x-parrot-input-type",
        HeaderValue::from_static(detected.as_str()),
    );
    response_headers.insert("x-parrot-events", HeaderValue::from(count));
    response
}

/// POST /api/replay/parse — 스트리밍 없이 파싱 결과와 대기 시간만 확인 (디버깅/미리보기용)
pub async fn parse_preview(
    query: Query<ReplayOptions>,
    headers: HeaderMap,
    body: String,
) -> Response {
    match prepare(query, &headers, body) {
        Ok((detected, events, waits)) => {
            let events: Vec<Value> = events
                .iter()
                .zip(waits)
                .map(|(e, wait)| json!({ "wait_ms": wait.as_millis() as u64, "frame": e.frame }))
                .collect();
            Json(json!({ "input_type": detected, "count": events.len(), "events": events }))
                .into_response()
        }
        Err(message) => bad_request(message),
    }
}

type Prepared = (InputType, Vec<Event>, Vec<Duration>);

fn prepare(
    Query(query): Query<ReplayOptions>,
    headers: &HeaderMap,
    body: String,
) -> Result<Prepared, String> {
    let (raw, options) = read_request(query, headers, body)?;
    if !(options.speed > 0.0 && options.speed.is_finite()) {
        return Err("speed는 0보다 큰 수여야 합니다".into());
    }
    let (detected, events) = parse::parse(&raw, options.input_type)?;
    let waits = schedule(&events, &options);
    Ok((detected, events, waits))
}

/// JSON 본문에 raw(또는 input) 필드가 있으면 요청 객체로, 아니면 본문 전체를 캡처 원문으로 본다
fn read_request(
    query: ReplayOptions,
    headers: &HeaderMap,
    body: String,
) -> Result<(String, ReplayOptions), String> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("json"));

    if is_json
        && let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&body)
        && (obj.contains_key("raw") || obj.contains_key("input"))
    {
        let request: ReplayRequest = serde_json::from_value(Value::Object(obj))
            .map_err(|e| format!("요청 형식 오류: {e}"))?;
        return Ok((request.raw, request.options));
    }
    Ok((body, query))
}

fn schedule(events: &[Event], options: &ReplayOptions) -> Vec<Duration> {
    let use_original = match options.timing {
        Timing::Original => true,
        Timing::Fixed => false,
        Timing::Auto => events
            .iter()
            .any(|e| e.time_ms.is_some() || e.delay_ms.is_some()),
    };
    let fixed = || {
        let jitter = if options.jitter_ms > 0 {
            fastrand::u64(0..=options.jitter_ms)
        } else {
            0
        };
        (options.delay_ms + jitter) as f64
    };

    let mut prev_time: Option<f64> = None;
    events
        .iter()
        .enumerate()
        .map(|(i, event)| {
            let wait_ms = if !use_original {
                if i == 0 { 0.0 } else { fixed() }
            } else if let Some(delay) = event.delay_ms {
                delay as f64 / options.speed
            } else if let (Some(time), Some(prev)) = (event.time_ms, prev_time) {
                // 자정을 넘긴 캡처는 하루를 더해 보정
                let gap = if time >= prev {
                    time - prev
                } else {
                    time - prev + DAY_MS
                };
                gap / options.speed
            } else if i == 0 {
                0.0
            } else {
                // 시각 정보가 빠진 이벤트는 고정 간격으로
                fixed()
            };
            prev_time = event.time_ms.or(prev_time);

            let wait_ms = match options.max_gap_ms {
                Some(max) if use_original => wait_ms.min(max as f64),
                _ => wait_ms,
            };
            Duration::from_secs_f64(wait_ms.max(0.0) / 1000.0)
        })
        .collect()
}

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(timing: Timing, speed: f64, max_gap_ms: Option<u64>) -> ReplayOptions {
        ReplayOptions {
            input_type: InputType::Auto,
            timing,
            delay_ms: 30,
            jitter_ms: 0,
            speed,
            max_gap_ms,
        }
    }

    fn timed(times: &[Option<f64>]) -> Vec<Event> {
        times
            .iter()
            .map(|&time_ms| Event {
                frame: String::new(),
                time_ms,
                delay_ms: None,
            })
            .collect()
    }

    fn millis(waits: Vec<Duration>) -> Vec<u128> {
        waits.into_iter().map(|d| d.as_millis()).collect()
    }

    #[test]
    fn original_timing_uses_captured_gaps() {
        let events = timed(&[Some(1000.0), Some(1250.0), None, Some(2250.0)]);
        let waits = schedule(&events, &options(Timing::Auto, 1.0, None));
        assert_eq!(millis(waits), [0, 250, 30, 1000]);
    }

    #[test]
    fn speed_and_max_gap() {
        let events = timed(&[Some(0.0), Some(400.0), Some(10_400.0)]);
        let waits = schedule(&events, &options(Timing::Original, 2.0, Some(3000)));
        assert_eq!(millis(waits), [0, 200, 3000]);
    }

    #[test]
    fn midnight_wraparound() {
        let events = timed(&[Some(DAY_MS - 100.0), Some(50.0)]);
        let waits = schedule(&events, &options(Timing::Original, 1.0, None));
        assert_eq!(millis(waits), [0, 150]);
    }

    #[test]
    fn fixed_timing_ignores_captured_times() {
        let events = timed(&[Some(0.0), Some(5000.0), Some(9000.0)]);
        let waits = schedule(&events, &options(Timing::Fixed, 1.0, None));
        assert_eq!(millis(waits), [0, 30, 30]);
    }
}
