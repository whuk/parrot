use std::convert::Infallible;
use std::time::Duration;

use axum::{
    body::Body,
    http::{HeaderName, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;

/// SSE 프레임 한 개와, 그 프레임을 보내기 전에 기다릴 시간
pub type TimedFrame = (Duration, String);

/// 프레임을 순서대로, 각자의 대기 시간을 지켜가며 text/event-stream으로 전송
pub fn sse_response(frames: Vec<TimedFrame>) -> Response {
    let body = futures::stream::iter(frames).then(|(wait, frame)| async move {
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        Ok::<_, Infallible>(frame)
    });

    (
        [
            (header::CONTENT_TYPE, "text/event-stream; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
            // 리버스 프록시(nginx) 뒤에서도 버퍼링 없이 흘려보내도록
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(body),
    )
        .into_response()
}
