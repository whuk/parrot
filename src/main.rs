mod echo;
mod formats;
mod replay;
mod stream;

use std::io;
use std::net::IpAddr;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use rust_embed::Embed;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

/// LLM 스트림 응답을 흉내 내거나(echo) 캡처한 스트림을 그대로 재생(replay)하는 테스트 서버
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// 바인딩할 주소 (외부 접속 허용 시 0.0.0.0)
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// 시작 포트 (사용 중이면 다음 포트를 시도)
    #[arg(short, long, default_value_t = 8787)]
    port: u16,

    /// 브라우저를 자동으로 열지 않음 (API 서버로만 사용)
    #[arg(long)]
    no_browser: bool,
}

/// static/ 폴더를 바이너리에 내장 (debug 빌드에서는 디스크에서 직접 읽음)
#[derive(Embed)]
#[folder = "static/"]
struct Assets;

const PORT_RETRIES: u16 = 10;
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse();

    let listener = bind(args.host, args.port).await?;
    let addr = listener.local_addr()?;
    let browser_host = if addr.ip().is_unspecified() {
        "localhost".to_string()
    } else {
        addr.ip().to_string()
    };
    let url = format!("http://{browser_host}:{}", addr.port());

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/echo", post(echo::echo))
        .route("/api/replay", post(replay::replay))
        .route("/api/replay/parse", post(replay::parse_preview))
        .fallback(static_handler)
        // 긴 스트림 캡처도 붙여넣을 수 있도록 (기본 2MB)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        // 다른 포트의 로컬 프론트엔드에서도 바로 호출할 수 있도록
        .layer(CorsLayer::permissive());

    println!("parrot v{} 실행 중: {url}", env!("CARGO_PKG_VERSION"));
    println!("종료하려면 Ctrl+C");

    if !args.no_browser {
        if let Err(e) = open::that_detached(&url) {
            eprintln!("브라우저를 열지 못했습니다 ({e}). 직접 접속하세요: {url}");
        }
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

/// 지정 포트부터 순서대로 시도하고, 모두 사용 중이면 OS가 고른 빈 포트를 사용
async fn bind(host: IpAddr, port: u16) -> io::Result<TcpListener> {
    for p in port..port.saturating_add(PORT_RETRIES) {
        match TcpListener::bind((host, p)).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    TcpListener::bind((host, 0)).await
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn static_handler(uri: Uri) -> Response {
    let path = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        p => p,
    };
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.to_string())], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
