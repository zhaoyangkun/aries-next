use std::time::Duration;

use axum::{
    Json, Router,
    body::Body,
    error_handling::HandleErrorLayer,
    extract::State,
    http::{
        HeaderName, HeaderValue, Method, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE, ORIGIN, USER_AGENT},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::TryStreamExt;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi as _;
use utoipa_scalar::Servable as _;

pub mod config;
pub mod http;
pub mod log_store;
pub mod logging;
pub mod openapi;
pub mod rate_limit;
pub mod security;
pub mod state;
pub mod worker;

use aries_core::health::HealthResponse;
use log_store::QUIET_INTERNAL_SPAN;
use state::AppState;

pub fn build_app(state: AppState) -> anyhow::Result<Router> {
    let admin_origins = state
        .config
        .admin_origins
        .iter()
        .map(|origin| {
            HeaderValue::try_from(origin.as_str())
                .map_err(|_| anyhow::anyhow!("invalid ADMIN_ORIGINS header value"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(admin_origins))
        .allow_credentials(true)
        .allow_headers([CONTENT_TYPE, ORIGIN, USER_AGENT])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::HEAD,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ]);

    // Layer 声明顺序与请求流向相反：越靠后越靠外。
    // TraceLayer 放在 SetRequestId 内侧，make_span 时 x-request-id 已写入请求头；
    // 参数日志中间件在 TraceLayer 内侧，事件落在请求 Span 之内以关联 request_id。
    // 安全响应头放在最外层，保证超时 / CORS 等产生的错误响应也携带安全头。
    let slow_request_ms = state.config.slow_request_ms;
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|request: &axum::extract::Request| {
            let request_id = request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-")
                .to_owned();
            let query = request.uri().query().unwrap_or("");
            let path = request.uri().path();
            // 运行日志查询接口自身的请求 Span 命名为 quiet_internal：
            // 其产生的 SQL 日志与 on_response "request completed" 事件
            // 都不落 server_logs，避免"查日志本身写日志"的噪音与膨胀。
            if path.starts_with("/api/admin/logs") {
                tracing::info_span!(
                    QUIET_INTERNAL_SPAN,
                    method = %request.method(),
                    path,
                    query,
                    request_id,
                )
            } else if path.starts_with("/api/health") {
                // 健康检查用 DEBUG 级 Span：on_response 据此降级，避免噪音。
                tracing::debug_span!(
                    "http_request",
                    method = %request.method(),
                    path,
                    query,
                    request_id,
                )
            } else {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path,
                    query,
                    request_id,
                )
            }
        })
        .on_response(
            move |response: &Response, latency: std::time::Duration, span: &tracing::Span| {
                let status = response.status().as_u16();
                let latency_ms = latency.as_millis();
                let is_health = span
                    .metadata()
                    .is_some_and(|meta| *meta.level() == tracing::Level::DEBUG);
                if is_health {
                    tracing::debug!(status, latency_ms, "request completed");
                } else if response.status().is_server_error() {
                    tracing::error!(status, latency_ms, "request completed");
                } else {
                    tracing::info!(status, latency_ms, "request completed");
                    // 慢请求：未达 5xx 但耗时超阈值，WARN 以便与慢查询日志互相定位。
                    if slow_request_ms > 0 && latency_ms >= u128::from(slow_request_ms) {
                        tracing::warn!(status, latency_ms, "slow request");
                    }
                }
            },
        );

    let hsts = hsts_enabled();
    let mut router = Router::new()
        .route("/api/health/live", get(live))
        .route("/api/health/ready", get(ready))
        .route(
            "/api/media/files/{*path}",
            get(http::media::serve_media_file),
        )
        .route("/admin", get(http::admin_spa::redirect_to_admin))
        .route("/admin/", get(http::admin_spa::serve_root))
        .route("/admin/{*path}", get(http::admin_spa::serve))
        .nest("/api/admin", http::admin_router(state.clone()))
        .nest("/api/public", http::public::router());

    // OpenAPI 文档（Scalar UI，spec 内嵌于页面）仅在显式开启时挂载：
    // 文档会暴露全部端点结构，生产默认关闭（OPENAPI_DOCS=true 开启）。
    // 与其他路由同一中间件链（安全响应头/超时/request-id）。
    if state.config.openapi_docs_enabled {
        // spec 以 serde_json::Value 传入：避免 utoipa 主版本与 utoipa-scalar 内部
        // 依赖版本不一致造成的类型冲突，Scalar 对 Value 与 OpenApi 同等支持。
        // spec 由宏静态生成，序列化失败属于编程错误，启动期即失败优于静默空文档。
        let spec = serde_json::to_value(openapi::ApiDoc::openapi())
            .expect("OpenAPI spec serialization failed");
        // utoipa-scalar 默认模板走 jsdelivr，国内访问不稳定；改用 npmmirror（阿里 CDN）
        // 的固定版本 standalone.js。版本升级时同步更新此 URL（并确认文件 200）。
        const SCALAR_HTML: &str = r#"<!doctype html>
<html>
<head>
    <title>$title</title>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1"/>
</head>
<body>
<script id="api-reference" type="application/json">$spec</script>
<script src="https://registry.npmmirror.com/@scalar/api-reference/1.72.4/files/dist/browser/standalone.js"></script>
</body>
</html>"#;
        router = router.merge(Router::<AppState>::from(
            utoipa_scalar::Scalar::with_url("/api/docs", spec)
                .custom_html(SCALAR_HTML)
                .title("Aries Next API"),
        ));
    }

    Ok(router
        .with_state(state)
        .layer(middleware::from_fn(log_request_params))
        .layer(trace_layer)
        .layer(PropagateRequestIdLayer::new(
            axum::http::header::HeaderName::from_static("x-request-id"),
        ))
        .layer(SetRequestIdLayer::new(
            axum::http::header::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(cors)
        // Router::layer 要求 layer 的错误类型为 Infallible：
        // Timeout 的 BoxError 由 HandleError 就地转成 408，二者组合为一个 layer。
        .layer(
            tower::ServiceBuilder::new()
                .layer(HandleErrorLayer::new(request_timeout))
                .timeout(REQUEST_TIMEOUT),
        )
        .layer(middleware::from_fn(move |request, next| {
            set_security_headers(hsts, request, next)
        })))
}

/// 全局请求超时：单请求处理超过该时长返回 408，防止慢请求长期占用连接。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn request_timeout(error: axum::BoxError) -> StatusCode {
    if error.is::<tower::timeout::error::Elapsed>() {
        StatusCode::REQUEST_TIMEOUT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// SECURITY_HSTS 仅识别 true/1，未设置默认关闭。
/// 只有 HTTPS 部署才能开启 HSTS（浏览器对 HTTP 明文站点忽略该头，
/// 但开启后子域名 HTTP 会被浏览器强制升级，本地开发绝不能打开）。
fn hsts_enabled() -> bool {
    matches!(
        std::env::var("SECURITY_HSTS").as_deref(),
        Ok("true") | Ok("1")
    )
}

/// 对 /api 响应统一追加安全响应头。
async fn set_security_headers(hsts: bool, request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    if hsts {
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

/// 敏感键判定清单见 `log_store`：请求参数在源头脱敏，落库层再兜底一道。
pub(crate) use crate::log_store::is_sensitive_key;

/// 递归脱敏：对象逐键判断，命中敏感键名的子树整体替换为 "***"（不展开），
/// 数组逐元素递归处理，标量原样保留。
fn redact_params(map: serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let redacted: serde_json::Map<String, serde_json::Value> = map
        .into_iter()
        .map(|(key, value)| {
            if is_sensitive_key(&key) {
                (key, serde_json::Value::String("***".to_owned()))
            } else {
                (key, redact_value(value))
            }
        })
        .collect();
    serde_json::Value::Object(redacted)
}

fn redact_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => redact_params(map),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redact_value).collect())
        }
        other => other,
    }
}

/// 请求 Body 读取上限：超过则只记 body_skipped，不解析参数。
const PARAM_LOG_BODY_LIMIT: usize = 8192;

/// 请求参数日志：非 GET/HEAD/OPTIONS 且 JSON/Form 的请求缓冲 Body、
/// 递归脱敏后记录；其余请求记 body_skipped 原因。
/// 超限与读失败时放弃解析，但已缓冲的完整 Body 仍原样透传给下游
/// （下游 JSON/Form Extractor 本来就要全量缓冲，此处透传不引入额外内存开销）。
async fn log_request_params(request: axum::extract::Request, next: Next) -> Response {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }

    let content_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let is_json = content_type.starts_with("application/json");
    let is_form = content_type.starts_with("application/x-www-form-urlencoded");
    if !is_json && !is_form {
        // multipart（媒体上传 / Markdown 导入）等不解析内容。
        tracing::info!(
            body_skipped = "unsupported_content_type",
            "http request params"
        );
        return next.run(request).await;
    }

    let declared_length = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared_length.is_some_and(|length| length > PARAM_LOG_BODY_LIMIT) {
        tracing::info!(body_skipped = "too_large", "http request params");
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let bytes = match body
        .into_data_stream()
        .map_ok(|chunk| chunk.to_vec())
        .try_concat()
        .await
    {
        Ok(bytes) => bytes,
        Err(_) => {
            tracing::info!(body_skipped = "read_failed", "http request params");
            // Body 传输不完整，透传已无意义，直接终止请求。
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let params = if bytes.len() > PARAM_LOG_BODY_LIMIT {
        tracing::info!(body_skipped = "too_large", "http request params");
        None
    } else if is_json {
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .map(redact_params)
    } else {
        // Form：仅按键名脱敏；值保留原文（不做 percent-decode，仅用于日志展示）。
        let text = String::from_utf8_lossy(&bytes);
        let map: serde_json::Map<String, serde_json::Value> = text
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                (key.to_owned(), serde_json::Value::String(value.to_owned()))
            })
            .collect();
        Some(redact_params(map))
    };
    if let Some(params) = params {
        tracing::info!(params = %params, "http request params");
    }

    // 重建请求体，下游 handler 不受影响。
    next.run(axum::extract::Request::from_parts(parts, Body::from(bytes)))
        .await
}

#[utoipa::path(
    get,
    path = "/api/health/live",
    tag = "Health",
    operation_id = "live",
    summary = "存活探针",
    description = "进程存活即返回 200；不检查数据库等外部依赖。",
    responses((status = 200, description = "服务存活", body = crate::openapi::HealthView))
)]
pub(crate) async fn live() -> Json<HealthResponse> {
    Json(HealthResponse::live())
}

#[utoipa::path(
    get,
    path = "/api/health/ready",
    tag = "Health",
    operation_id = "ready",
    summary = "就绪探针",
    description = "额外执行 `SELECT 1` 检查数据库连通性，不可用时返回 503。",
    responses(
        (status = 200, description = "服务就绪", body = crate::openapi::HealthView),
        (status = 503, description = "数据库不可用")
    )
)]
pub(crate) async fn ready(
    State(state): State<AppState>,
) -> Result<Json<HealthResponse>, StatusCode> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.database)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(Json(HealthResponse::live()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn redaction_masks_sensitive_keys_case_insensitively() {
        let map = serde_json::json!({
            "password": "plain-text",
            "Password": "plain-text",
            "bootstrap_secret": "s3cret",
            "api_key": "sk-live",
            "access_token": "tok",
            "Authorization": "Bearer x",
            "set_cookie": "session=abc",
            "username": "owner",
            "title": "正常字段",
        });
        let redacted = redact_params(map.as_object().unwrap().clone());
        for key in [
            "password",
            "Password",
            "bootstrap_secret",
            "api_key",
            "access_token",
            "Authorization",
            "set_cookie",
        ] {
            assert_eq!(redacted[key], "***", "key {key} must be masked");
        }
        // 正常字段不受影响。
        assert_eq!(redacted["username"], "owner");
        assert_eq!(redacted["title"], "正常字段");
    }

    #[test]
    fn redaction_keeps_non_string_values() {
        let map = serde_json::json!({ "page_size": 20, "visible": true });
        let redacted = redact_params(map.as_object().unwrap().clone());
        assert_eq!(redacted["page_size"], 20);
        assert_eq!(redacted["visible"], true);
    }

    #[test]
    fn redaction_masks_sensitive_keys_in_nested_objects_and_arrays() {
        // 设置分组更新体为嵌套结构（{"settings": {...}}），敏感键必须递归脱敏。
        let map = serde_json::json!({
            "settings": {
                "api_key": "sk-live",
                "smtp_password": "smtp-secret",
                "endpoint": { "token": "tok", "host": "api.example.com" },
                "limits": [{ "access_token": "a" }, { "count": 3 }]
            },
            "title": "正常字段"
        });
        let redacted = redact_params(map.as_object().unwrap().clone());
        let settings = &redacted["settings"];
        assert_eq!(settings["api_key"], "***");
        assert_eq!(settings["smtp_password"], "***");
        assert_eq!(settings["endpoint"]["token"], "***");
        assert_eq!(settings["endpoint"]["host"], "api.example.com");
        assert_eq!(settings["limits"][0]["access_token"], "***");
        assert_eq!(settings["limits"][1]["count"], 3);
        assert_eq!(redacted["title"], "正常字段");
    }

    #[tokio::test]
    async fn security_headers_are_applied_to_responses() {
        for hsts in [false, true] {
            let app = Router::new()
                .route("/api/test", get(|| async { "ok" }))
                .layer(middleware::from_fn(move |request, next| {
                    set_security_headers(hsts, request, next)
                }));

            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/test")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let headers = response.headers();
            assert_eq!(headers["x-content-type-options"], "nosniff");
            assert_eq!(headers["x-frame-options"], "DENY");
            assert_eq!(
                headers["referrer-policy"],
                "strict-origin-when-cross-origin"
            );
            assert_eq!(headers["content-security-policy"], "frame-ancestors 'none'");
            if hsts {
                assert_eq!(
                    headers["strict-transport-security"],
                    "max-age=31536000; includeSubDomains"
                );
            } else {
                assert!(!headers.contains_key("strict-transport-security"));
            }
        }
    }
}
