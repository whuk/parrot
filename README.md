# 🦜 Parrot

LLM 스트리밍 응답을 테스트하기 위한 로컬 서버입니다. 실행 파일 하나로 동작하며, 실행하면 브라우저에 UI가 열립니다.

- **Echo**: 붙여넣은 텍스트를 LLM이 생성하듯 조금씩 스트리밍 (OpenAI / Claude / Gemini 형식)
- **Replay**: 캡처한 LLM 스트림을 같은 형식 그대로, 원래 타이밍대로 다시 스트리밍

## 설치 및 실행

[Releases](../../releases)에서 OS에 맞는 파일을 받아 압축을 풉니다.

| OS | 파일 |
|---|---|
| Windows | `parrot-<버전>-windows-x64.zip` |
| macOS (Apple Silicon) | `parrot-<버전>-macos-arm64.tar.gz` |
| macOS (Intel) | `parrot-<버전>-macos-x64.tar.gz` |
| Linux | `parrot-<버전>-linux-x64.tar.gz` |

실행하면 `http://127.0.0.1:8787`에서 서버가 뜨고 기본 브라우저가 열립니다. 종료는 터미널(콘솔) 창에서 `Ctrl+C`입니다.

**macOS**: 서명되지 않은 실행 파일이라 처음 실행 시 차단됩니다. 한 번만 격리 속성을 지워 주세요.

```bash
xattr -d com.apple.quarantine ./parrot
```

**Windows**: SmartScreen 경고가 뜨면 `추가 정보` → `실행`을 누르세요.

### 실행 옵션

```
parrot [--port 8787] [--host 127.0.0.1] [--no-browser]
```

| 옵션 | 기본값 | 설명 |
|---|---|---|
| `-p, --port` | `8787` | 시작 포트. 사용 중이면 다음 포트를 시도 |
| `--host` | `127.0.0.1` | 다른 기기에서 접속하려면 `0.0.0.0` |
| `--no-browser` | | 브라우저를 열지 않고 API 서버로만 실행 |

## API

모든 스트리밍 응답은 `text/event-stream`이며, CORS가 열려 있어 다른 포트의 프론트엔드에서 바로 호출할 수 있습니다.

### `POST /api/echo`

```bash
curl -N localhost:8787/api/echo -H 'content-type: application/json' \
  -d '{"text":"안녕하세요, 저는 parrot 입니다.","format":"claude"}'
```

| 필드 | 기본값 | 설명 |
|---|---|---|
| `text` | (필수) | 스트리밍할 텍스트 |
| `format` | `openai` | `openai`(`gpt`), `claude`(`anthropic`), `gemini` |
| `chunk` | `word` | 조각 단위: `word`, `char`, `line` |
| `delay_ms` | `30` | 조각 사이 지연 |
| `jitter_ms` | `20` | 지연에 더해지는 0~N ms 무작위 값 |
| `model` | 포맷별 기본값 | 응답에 표시할 모델명 |

### `POST /api/replay`

캡처한 스트림을 그대로 다시 보냅니다. 본문은 둘 중 하나로 보냅니다.

```bash
# 캡처 원문을 그대로, 옵션은 쿼리스트링으로
curl -N "localhost:8787/api/replay?speed=2" --data-binary @capture.txt

# JSON으로 감싸서
curl -N localhost:8787/api/replay -H 'content-type: application/json' \
  -d '{"raw":"data: {...}\n\ndata: [DONE]\n\n","timing":"fixed","delay_ms":50}'
```

| 옵션 | 기본값 | 설명 |
|---|---|---|
| `input_type` | `auto` | `sse`, `devtools`, `json` (자동 감지) |
| `timing` | `auto` | 시각 정보가 있으면 `original`, 없으면 `fixed` |
| `delay_ms` / `jitter_ms` | `30` / `0` | `fixed` 타이밍의 간격 |
| `speed` | `1.0` | `original` 타이밍 배속 |
| `max_gap_ms` | 없음 | `original` 타이밍에서 이벤트 사이 최대 대기 |

지원하는 입력:

- **SSE 원문**: DevTools Network → Response 탭이나 `curl -N` 출력. 프레임을 바이트 단위로 그대로 재생합니다.
- **DevTools EventStream 복사본**: `[Id] Type Data Time` 탭 구분 행. Time 열로 원래 간격을 재현합니다.
- **JSON 배열**: 문자열(`"[DONE]"`), 페이로드 객체(`type` 필드가 있으면 `event:` 줄도 생성), `{event, data, time | delay_ms}` 객체를 섞어 쓸 수 있습니다.

응답 헤더 `x-parrot-input-type`, `x-parrot-events`로 감지 결과를 확인할 수 있습니다.

### `POST /api/replay/parse`

`/api/replay`와 같은 입력을 받아, 스트리밍 대신 이벤트 분할 결과와 대기 시간을 JSON으로 돌려줍니다.

## 개발

```bash
cargo run              # static/ 을 디스크에서 읽으므로 HTML 수정은 새로고침만으로 반영
cargo test
cargo build --release  # static/ 이 실행 파일에 내장됨
```

### 릴리스

`v*` 태그를 푸시하면 GitHub Actions가 Windows / macOS(arm64, x64) / Linux 실행 파일을 빌드해 Release에 올립니다.

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Actions 탭에서 `Release` 워크플로를 수동 실행하면 릴리스 없이 빌드 결과물만 받을 수 있습니다.
