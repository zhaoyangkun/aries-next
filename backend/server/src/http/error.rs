use aries_core::ai::AiError;
use aries_core::auth::AuthError;
use aries_core::comments::CommentError;
use aries_core::content::ContentError;
use aries_core::galleries::GalleryError;
use aries_core::jobs::JobError;
use aries_core::journals::JournalError;
use aries_core::links::LinkError;
use aries_core::logs::LogError;
use aries_core::media::MediaError;
use aries_core::navigation::NavigationError;
use aries_core::pages::PageError;
use aries_core::settings::SettingError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    /// 结构化补充信息（如引用计数），大多数错误为空，序列化时省略。
    details: Option<serde_json::Value>,
    /// 429 的 Retry-After 秒数；无窗口信息（如上游 Provider 限流）时缺省。
    retry_after: Option<u64>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

impl ApiError {
    pub const fn bad_request(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    pub const fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "UNAUTHORIZED",
            message: "Authentication required",
            details: None,
            retry_after: None,
        }
    }

    /// 401 但需要区分业务错误码（如文章访问密码错误）时使用。
    pub const fn unauthorized_with(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    pub const fn invalid_credentials() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "INVALID_CREDENTIALS",
            message: "Invalid username or password",
            details: None,
            retry_after: None,
        }
    }

    pub const fn forbidden(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    pub const fn conflict(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    /// 400 且需要携带结构化 Detail（如逐文件校验失败原因）时使用。
    pub fn bad_request_with_details(
        code: &'static str,
        message: &'static str,
        details: serde_json::Value,
    ) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message,
            details: Some(details),
            retry_after: None,
        }
    }

    /// 409 且需要携带结构化 Detail（如 Taxonomy 引用计数）时使用。
    pub fn conflict_with_details(
        code: &'static str,
        message: &'static str,
        details: serde_json::Value,
    ) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message,
            details: Some(details),
            retry_after: None,
        }
    }

    pub const fn not_found(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    /// 429 但无剩余窗口信息时使用（如上游 Provider 限流）：不携带 Retry-After。
    pub const fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "RATE_LIMITED",
            message: "Too many requests",
            details: None,
            retry_after: None,
        }
    }

    /// 本地限流拒绝：携带距窗口恢复的剩余时长，向上游返回 Retry-After。
    /// 秒数向上取整（HTTP Retry-After 为整数秒），至少 1 秒。
    pub fn rate_limited_retry_after(retry_after: std::time::Duration) -> Self {
        let secs = retry_after
            .as_secs()
            .saturating_add(u64::from(retry_after.subsec_nanos() > 0))
            .max(1);
        Self {
            retry_after: Some(secs),
            ..Self::rate_limited()
        }
    }

    /// 上游（AI Provider 等）错误。
    pub const fn bad_gateway(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    pub const fn gateway_timeout(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            code,
            message,
            details: None,
            retry_after: None,
        }
    }

    pub const fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "Internal server error",
            details: None,
            retry_after: None,
        }
    }

    /// 记录底层错误后返回 500：服务端保留 ERROR 日志供生产排障，
    /// 响应体仍只含通用文案，不泄露错误细节。
    pub fn internal_logged(error: impl std::fmt::Debug) -> Self {
        tracing::error!(error = ?error, "internal server error");
        Self::internal()
    }
}

impl From<ContentError> for ApiError {
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::InvalidStatus => {
                Self::bad_request("INVALID_ARTICLE_STATUS", "Article status is invalid")
            }
            ContentError::InvalidTransition => Self::bad_request(
                "INVALID_ARTICLE_TRANSITION",
                "Article status transition is invalid",
            ),
            ContentError::InvalidTitle => {
                Self::bad_request("INVALID_ARTICLE_TITLE", "Article title is invalid")
            }
            ContentError::InvalidSlug => {
                Self::bad_request("INVALID_ARTICLE_SLUG", "Article slug is invalid")
            }
            ContentError::InvalidSummary => {
                Self::bad_request("INVALID_ARTICLE_SUMMARY", "Article summary is invalid")
            }
            ContentError::InvalidTaxonomyName => {
                Self::bad_request("INVALID_TAXONOMY_NAME", "Taxonomy name is invalid")
            }
            ContentError::InvalidCategoryKind => {
                Self::bad_request("INVALID_CATEGORY_KIND", "Category kind is invalid")
            }
            ContentError::PublishValidation => {
                Self::bad_request("ARTICLE_NOT_PUBLISHABLE", "Article content is incomplete")
            }
            ContentError::Conflict => {
                Self::conflict("ARTICLE_CONFLICT", "Article slug or version conflicts")
            }
            ContentError::NotFound => Self::not_found("ARTICLE_NOT_FOUND", "Article was not found"),
            ContentError::RevisionNotFound => {
                Self::not_found("REVISION_NOT_FOUND", "Article revision was not found")
            }
            ContentError::NotRecycled => Self::conflict(
                "ARTICLE_NOT_RECYCLED",
                "Article must be recycled before deletion",
            ),
            ContentError::Referenced { count } => Self::conflict_with_details(
                "TAXONOMY_IN_USE",
                "Taxonomy is still referenced by articles",
                serde_json::json!({ "reference_count": count }),
            ),
            ContentError::StoreUnavailable
            | ContentError::RenderFailed
            | ContentError::InvalidRevisionSnapshot => Self::internal_logged(&error),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::InvalidUsername => {
                Self::bad_request("INVALID_USERNAME", "Username format is invalid")
            }
            AuthError::InvalidPasswordLength | AuthError::WeakPassword => {
                Self::bad_request("INVALID_PASSWORD", "Password does not meet the policy")
            }
            AuthError::Conflict => Self::conflict("RESOURCE_CONFLICT", "Resource already exists"),
            AuthError::NotFound => {
                Self::bad_request("RESOURCE_NOT_FOUND", "Resource was not found")
            }
            AuthError::InvalidRole | AuthError::InvalidUserStatus | AuthError::StoreUnavailable => {
                Self::internal_logged(&error)
            }
        }
    }
}

impl From<MediaError> for ApiError {
    fn from(error: MediaError) -> Self {
        match error {
            MediaError::NotFound => Self::not_found("MEDIA_NOT_FOUND", "Media asset was not found"),
            MediaError::Referenced { count } => Self::conflict_with_details(
                "MEDIA_IN_USE",
                "Media asset is still referenced",
                serde_json::json!({ "reference_count": count }),
            ),
            MediaError::Conflict => Self::conflict("MEDIA_CONFLICT", "Media asset conflicts"),
            MediaError::InvalidProvider
            | MediaError::InvalidUsageTarget
            | MediaError::InvalidCommentPolicy
            | MediaError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<JobError> for ApiError {
    fn from(error: JobError) -> Self {
        match error {
            JobError::NotFound => Self::not_found("JOB_NOT_FOUND", "Background job was not found"),
            JobError::InvalidKind | JobError::InvalidStatus | JobError::StoreUnavailable => {
                Self::internal_logged(&error)
            }
        }
    }
}

impl From<PageError> for ApiError {
    fn from(error: PageError) -> Self {
        match error {
            PageError::InvalidStatus => {
                Self::bad_request("INVALID_PAGE_STATUS", "Page status is invalid")
            }
            PageError::InvalidSlug => {
                Self::bad_request("INVALID_PAGE_SLUG", "Page slug is invalid")
            }
            PageError::InvalidTitle => {
                Self::bad_request("INVALID_PAGE_TITLE", "Page title is invalid")
            }
            PageError::NotFound => Self::not_found("PAGE_NOT_FOUND", "Page was not found"),
            PageError::Conflict => Self::conflict("SLUG_CONFLICT", "Page slug already exists"),
            PageError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<JournalError> for ApiError {
    fn from(error: JournalError) -> Self {
        match error {
            JournalError::InvalidVisibility => Self::bad_request(
                "INVALID_JOURNAL_VISIBILITY",
                "Journal visibility is invalid",
            ),
            JournalError::ContentLength => Self::bad_request(
                "INVALID_JOURNAL_CONTENT",
                "Journal content length is invalid",
            ),
            JournalError::NotFound => Self::not_found("JOURNAL_NOT_FOUND", "Journal was not found"),
            JournalError::Conflict => Self::conflict("JOURNAL_CONFLICT", "Journal conflicts"),
            JournalError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<GalleryError> for ApiError {
    fn from(error: GalleryError) -> Self {
        match error {
            GalleryError::InvalidStatus => {
                Self::bad_request("INVALID_GALLERY_STATUS", "Gallery status is invalid")
            }
            GalleryError::InvalidSlug => {
                Self::bad_request("INVALID_GALLERY_SLUG", "Gallery slug is invalid")
            }
            GalleryError::InvalidTitle => {
                Self::bad_request("INVALID_GALLERY_TITLE", "Gallery title is invalid")
            }
            GalleryError::NotFound => Self::not_found("GALLERY_NOT_FOUND", "Gallery was not found"),
            GalleryError::ItemNotFound => {
                Self::not_found("GALLERY_ITEM_NOT_FOUND", "Gallery item was not found")
            }
            GalleryError::Conflict => Self::conflict(
                "GALLERY_CONFLICT",
                "Gallery slug or media asset already exists",
            ),
            GalleryError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<LinkError> for ApiError {
    fn from(error: LinkError) -> Self {
        match error {
            LinkError::InvalidStatus => {
                Self::bad_request("INVALID_LINK_STATUS", "Link status is invalid")
            }
            LinkError::InvalidUrl => {
                Self::bad_request("INVALID_LINK_URL", "Link url must use http or https scheme")
            }
            LinkError::InvalidTitle => {
                Self::bad_request("INVALID_LINK_TITLE", "Link title is invalid")
            }
            LinkError::NotFound => Self::not_found("LINK_NOT_FOUND", "Link was not found"),
            LinkError::Conflict => Self::conflict("LINK_CONFLICT", "Link conflicts"),
            LinkError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<NavigationError> for ApiError {
    fn from(error: NavigationError) -> Self {
        match error {
            NavigationError::InvalidTargetType => Self::bad_request(
                "INVALID_NAVIGATION_TARGET_TYPE",
                "Navigation target type is invalid",
            ),
            NavigationError::InvalidTarget => Self::bad_request(
                "INVALID_NAVIGATION_TARGET",
                "Navigation target fields do not match target type",
            ),
            NavigationError::InvalidUrl => Self::bad_request(
                "INVALID_NAVIGATION_URL",
                "Navigation url must use http or https scheme",
            ),
            NavigationError::InvalidLabel => {
                Self::bad_request("INVALID_NAVIGATION_LABEL", "Navigation label is invalid")
            }
            NavigationError::InvalidHierarchy => Self::bad_request(
                "INVALID_NAVIGATION_HIERARCHY",
                "Navigation allows at most two levels",
            ),
            NavigationError::NotFound => {
                Self::not_found("NAVIGATION_NOT_FOUND", "Navigation item was not found")
            }
            NavigationError::Conflict => {
                Self::conflict("NAVIGATION_CONFLICT", "Navigation item still has children")
            }
            NavigationError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<AiError> for ApiError {
    fn from(error: AiError) -> Self {
        match error {
            AiError::Disabled => Self::forbidden("AI_DISABLED", "AI is disabled"),
            AiError::FeatureDisabled => {
                Self::forbidden("AI_FEATURE_DISABLED", "AI feature is disabled")
            }
            AiError::Unconfigured => {
                Self::bad_request("AI_NOT_CONFIGURED", "AI provider is not configured")
            }
            AiError::ProviderFailed => {
                Self::bad_gateway("AI_PROVIDER_FAILED", "AI provider request failed")
            }
            AiError::Timeout => {
                Self::gateway_timeout("AI_PROVIDER_TIMEOUT", "AI provider request timed out")
            }
            AiError::RateLimited => Self::rate_limited(),
            AiError::InvalidOutput => {
                Self::bad_gateway("AI_INVALID_OUTPUT", "AI provider returned invalid output")
            }
            AiError::Validation => {
                Self::bad_request("INVALID_AI_PARAMETER", "AI parameter is invalid")
            }
            AiError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<SettingError> for ApiError {
    fn from(error: SettingError) -> Self {
        match error {
            SettingError::InvalidGroup | SettingError::NotFound => {
                Self::not_found("SETTING_GROUP_NOT_FOUND", "Setting group was not found")
            }
            SettingError::Conflict => Self::conflict(
                "SETTING_VERSION_CONFLICT",
                "Setting was modified by someone else",
            ),
            SettingError::Validation => {
                Self::bad_request("INVALID_SETTING_PAYLOAD", "Setting payload is invalid")
            }
            SettingError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<CommentError> for ApiError {
    fn from(error: CommentError) -> Self {
        match error {
            CommentError::InvalidTargetType => {
                Self::bad_request("INVALID_COMMENT_TARGET", "Comment target type is invalid")
            }
            CommentError::Validation => {
                Self::bad_request("INVALID_COMMENT", "Comment parameter is invalid")
            }
            CommentError::InvalidStatus => {
                Self::bad_request("INVALID_COMMENT_STATUS", "Comment status is invalid")
            }
            CommentError::InvalidTransition { .. } => Self::conflict(
                "INVALID_COMMENT_TRANSITION",
                "Comment status transition is invalid",
            ),
            CommentError::NotFound => Self::not_found("COMMENT_NOT_FOUND", "Comment was not found"),
            CommentError::AlreadyModerated => {
                Self::conflict("COMMENT_ALREADY_MODERATED", "Comment is already moderated")
            }
            CommentError::TargetNotFound => Self::not_found(
                "COMMENT_TARGET_NOT_FOUND",
                "Comment target is not commentable",
            ),
            CommentError::ContentLength => Self::bad_request(
                "INVALID_COMMENT_CONTENT",
                "Comment content length is invalid",
            ),
            CommentError::InvalidAuthorName => Self::bad_request(
                "INVALID_COMMENT_AUTHOR_NAME",
                "Comment author name is invalid",
            ),
            CommentError::InvalidAuthorEmail => {
                Self::bad_request("INVALID_COMMENT_EMAIL", "Comment author email is invalid")
            }
            CommentError::InvalidAuthorUrl => Self::bad_request(
                "INVALID_COMMENT_URL",
                "Comment author url must use http or https scheme",
            ),
            CommentError::Closed => Self::forbidden("COMMENTS_CLOSED", "Comments are closed"),
            CommentError::Duplicate => {
                Self::conflict("COMMENT_DUPLICATE", "Duplicate comment submission")
            }
            CommentError::RateLimited => Self::rate_limited(),
            CommentError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl From<LogError> for ApiError {
    fn from(error: LogError) -> Self {
        match error {
            LogError::NotFound => Self::not_found("LOG_ENTRY_NOT_FOUND", "Log entry was not found"),
            LogError::StoreUnavailable => Self::internal_logged(&error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry_after = self.retry_after;
        // API 错误响应一律禁止缓存，避免错误被 CDN/浏览器缓存放大。
        let mut response = (
            self.status,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(ErrorResponse {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                    details: self.details,
                },
            }),
        )
            .into_response();
        if let Some(secs) = retry_after {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_str(&secs.to_string())
                    .expect("integer seconds is a valid header value"),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aries_core::content::ContentError;

    /// 捕获 tracing 输出到内存缓冲，供断言日志内容。
    #[derive(Clone, Default)]
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn internal_from_conversion_logs_the_underlying_error() {
        let writer = CaptureWriter::default();
        let buffer = writer.0.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .finish();

        let error = tracing::subscriber::with_default(subscriber, || {
            ApiError::from(ContentError::StoreUnavailable)
        });

        let logged = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("internal server error"),
            "expected ERROR log, got: {logged}"
        );
        assert!(
            logged.contains("StoreUnavailable"),
            "expected underlying error detail in log, got: {logged}"
        );
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn internal_error_response_does_not_leak_details() {
        let response = ApiError::from(ContentError::StoreUnavailable).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "INTERNAL_ERROR");
        // 底层错误细节只能进服务端日志，不能出现在响应体。
        let serialized = body.to_string();
        assert!(!serialized.contains("StoreUnavailable"));
    }

    #[test]
    fn rate_limited_without_hint_has_no_retry_after_header() {
        let response = ApiError::rate_limited().into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            !response
                .headers()
                .contains_key(axum::http::header::RETRY_AFTER)
        );
    }

    #[tokio::test]
    async fn rate_limited_retry_after_sets_header_and_keeps_body() {
        let response =
            ApiError::rate_limited_retry_after(std::time::Duration::from_secs(42)).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[axum::http::header::RETRY_AFTER], "42");

        // 亚秒剩余窗口向上取整，且至少 1 秒。
        let response = ApiError::rate_limited_retry_after(std::time::Duration::from_millis(1500))
            .into_response();
        assert_eq!(response.headers()[axum::http::header::RETRY_AFTER], "2");
        let response = ApiError::rate_limited_retry_after(std::time::Duration::from_millis(10))
            .into_response();
        assert_eq!(response.headers()[axum::http::header::RETRY_AFTER], "1");

        // 响应体保持 RATE_LIMITED 结构不变。
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "RATE_LIMITED");
    }
}
