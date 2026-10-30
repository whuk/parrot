//! 포맷별 모델 목록 (UI의 모델 선택용)
//!
//! 내장 목록으로 시작하고, 서버 시작 시 백그라운드에서 models.dev(키 없이 쓸 수 있는 공개
//! 모델 카탈로그)에서 최신 목록을 받아 교체한다. 오프라인이거나 연결에 실패하면 내장 목록을 그대로 쓴다.

use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::formats::Format;

const SOURCE_URL: &str = "https://models.dev/api.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_PER_FORMAT: usize = 15;

/// 오프라인일 때 쓰는 내장 목록 (최신순)
const BUILTIN_OPENAI: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6",
    "gpt-5.5",
    "gpt-5.4-mini",
    "gpt-4.1",
    "gpt-4o-mini",
];
const BUILTIN_CLAUDE: &[&str] = &[
    "claude-haiku-5-5",
    "claude-sonnet-5-5",
    "claude-opus-5-5",
    "claude-fable-5-1",
    "claude-opus-4-8",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];
const BUILTIN_GEMINI: &[&str] = &[
    "gemini-4-argon",
    "gemini-3.8-flash",
    "gemini-3.5-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.1-pro-preview",
    "gemini-2.5-pro",
];

pub type SharedCatalog = Arc<RwLock<Catalog>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 시작 직후, 아직 받아오는 중
    Pending,
    /// 최신 목록으로 갱신됨
    Updated,
    /// 오프라인 등으로 갱신을 건너뜀 (내장 목록 사용)
    Skipped,
    /// --no-model-update 로 갱신을 끔
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub status: Status,
    /// 목록 출처 ("builtin" 또는 "models.dev")
    pub source: &'static str,
    /// 갱신 시각 (unix 초)
    pub updated_at: Option<u64>,
    pub formats: FormatModels,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatModels {
    pub openai: FormatEntry,
    pub claude: FormatEntry,
    pub gemini: FormatEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatEntry {
    /// 요청에 model이 없을 때 서버가 쓰는 모델명
    pub default: &'static str,
    /// 선택 가능한 모델 (최신순, default 제외)
    pub models: Vec<String>,
}

impl FormatEntry {
    fn new(format: Format, models: impl IntoIterator<Item = String>) -> Self {
        let default = format.default_model();
        FormatEntry {
            default,
            models: models.into_iter().filter(|m| m != default).collect(),
        }
    }
}

impl Catalog {
    pub fn builtin(status: Status) -> Self {
        let owned = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Catalog {
            status,
            source: "builtin",
            updated_at: None,
            formats: FormatModels {
                openai: FormatEntry::new(Format::Openai, owned(BUILTIN_OPENAI)),
                claude: FormatEntry::new(Format::Claude, owned(BUILTIN_CLAUDE)),
                gemini: FormatEntry::new(Format::Gemini, owned(BUILTIN_GEMINI)),
            },
        }
    }
}

/// GET /api/models
pub async fn list(Extension(catalog): Extension<SharedCatalog>) -> Json<Catalog> {
    Json(catalog.read().expect("catalog lock").clone())
}

/// 최신 목록을 받아 catalog를 교체한다. 실패하면 내장 목록을 유지하고 상태만 Skipped로 바꾼다.
pub async fn refresh(catalog: SharedCatalog) {
    let result = fetch().await;
    let mut current = catalog.write().expect("catalog lock");
    match result {
        Ok(fresh) => {
            println!(
                "모델 목록 갱신: OpenAI {} · Claude {} · Gemini {} (models.dev)",
                fresh.formats.openai.models.len(),
                fresh.formats.claude.models.len(),
                fresh.formats.gemini.models.len(),
            );
            *current = fresh;
        }
        Err(e) => {
            println!("모델 목록 갱신 건너뜀, 내장 목록 사용 ({e})");
            current.status = Status::Skipped;
        }
    }
}

async fn fetch() -> Result<Catalog, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("parrot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let source: Source = client
        .get(SOURCE_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(describe)?
        .json()
        .await
        .map_err(describe)?;

    from_source(source).ok_or_else(|| "받은 목록이 비어 있음".to_string())
}

fn describe(e: reqwest::Error) -> String {
    if e.is_connect() {
        "인터넷에 연결할 수 없음".into()
    } else if e.is_timeout() {
        "응답 시간 초과".into()
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------- models.dev 응답 (필요한 필드만)

#[derive(Debug, Deserialize)]
struct Source {
    openai: Option<Provider>,
    anthropic: Option<Provider>,
    google: Option<Provider>,
}

#[derive(Debug, Deserialize)]
struct Provider {
    models: std::collections::HashMap<String, Model>,
}

#[derive(Debug, Deserialize)]
struct Model {
    id: String,
    #[serde(default)]
    release_date: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    modalities: Modalities,
}

#[derive(Debug, Default, Deserialize)]
struct Modalities {
    #[serde(default)]
    output: Vec<String>,
}

/// 공급자별로 텍스트 출력 모델만 최신순으로 골라낸다. 어느 공급자든 비어 있으면 그 포맷은 내장 목록을 쓴다.
fn from_source(source: Source) -> Option<Catalog> {
    let builtin = Catalog::builtin(Status::Updated);
    let pick = |provider: Option<Provider>, prefix: &str| -> Option<Vec<String>> {
        let mut models: Vec<Model> = provider?
            .models
            .into_values()
            .filter(|m| m.modalities.output == ["text"])
            .filter(|m| m.status.as_deref() != Some("deprecated"))
            .filter(|m| m.id.starts_with(prefix) && !m.id.contains("embedding"))
            .collect();
        models.sort_by(|a, b| {
            b.release_date
                .cmp(&a.release_date)
                .then_with(|| b.id.cmp(&a.id))
        });
        let ids: Vec<String> = models
            .into_iter()
            .map(|m| m.id)
            .take(MAX_PER_FORMAT)
            .collect();
        (!ids.is_empty()).then_some(ids)
    };

    let openai = pick(source.openai, "");
    let claude = pick(source.anthropic, "claude");
    let gemini = pick(source.google, "gemini");
    if openai.is_none() && claude.is_none() && gemini.is_none() {
        return None;
    }

    let entry = |format, fresh: Option<Vec<String>>, fallback: FormatEntry| match fresh {
        Some(models) => FormatEntry::new(format, models),
        None => fallback,
    };
    Some(Catalog {
        status: Status::Updated,
        source: "models.dev",
        updated_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        formats: FormatModels {
            openai: entry(Format::Openai, openai, builtin.formats.openai),
            claude: entry(Format::Claude, claude, builtin.formats.claude),
            gemini: entry(Format::Gemini, gemini, builtin.formats.gemini),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_text_models_newest_first() {
        let json = r#"{
            "openai": { "models": {
                "gpt-old":   { "id": "gpt-old", "release_date": "2024-01-01", "modalities": { "output": ["text"] } },
                "gpt-new":   { "id": "gpt-new", "release_date": "2026-01-01", "modalities": { "output": ["text"] } },
                "gpt-4o":    { "id": "gpt-4o", "release_date": "2024-05-13", "modalities": { "output": ["text"] } },
                "gpt-voice": { "id": "gpt-voice", "release_date": "2026-02-01", "modalities": { "output": ["text", "audio"] } },
                "gpt-gone":  { "id": "gpt-gone", "release_date": "2026-03-01", "status": "deprecated", "modalities": { "output": ["text"] } },
                "text-embedding-9": { "id": "text-embedding-9", "release_date": "2026-04-01", "modalities": { "output": ["text"] } }
            }},
            "google": { "models": {
                "gemma-9": { "id": "gemma-9", "release_date": "2026-01-01", "modalities": { "output": ["text"] } },
                "gemini-9": { "id": "gemini-9", "release_date": "2026-01-01", "modalities": { "output": ["text"] }, "cost": {} }
            }},
            "somebody-else": { "weird": true }
        }"#;
        let catalog = from_source(serde_json::from_str(json).unwrap()).unwrap();

        assert_eq!(catalog.source, "models.dev");
        // default(gpt-4o)는 목록에서 빠지고, 음성·폐기·임베딩 모델도 제외
        assert_eq!(catalog.formats.openai.default, "gpt-4o");
        assert_eq!(catalog.formats.openai.models, ["gpt-new", "gpt-old"]);
        assert_eq!(catalog.formats.gemini.models, ["gemini-9"]);
        // anthropic이 없으면 Claude는 내장 목록 유지
        assert_eq!(catalog.formats.claude.models.len(), BUILTIN_CLAUDE.len());
    }

    #[test]
    fn empty_source_is_none() {
        let source: Source = serde_json::from_str(r#"{ "openai": { "models": {} } }"#).unwrap();
        assert!(from_source(source).is_none());
    }

    #[test]
    fn builtin_excludes_default() {
        let catalog = Catalog::builtin(Status::Pending);
        for entry in [
            &catalog.formats.openai,
            &catalog.formats.claude,
            &catalog.formats.gemini,
        ] {
            assert!(!entry.models.iter().any(|m| m == entry.default));
            assert!(!entry.models.is_empty());
        }
    }
}
