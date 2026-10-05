//! 自定义请求提取器：统一请求体解析失败时的错误响应。

use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
};

use super::error::ApiError;

/// JSON 请求体提取器，行为与 axum `Json` 一致，但把 Rejection 统一映射为
/// 400 `INVALID_REQUEST_BODY`：axum 0.8 的 `Json` 对 serde 数据错误默认返回 422，
/// 与 OpenAPI 契约约定的 400 不符。serde 的具体原因只记入服务端日志（DEBUG），
/// 不随响应返回，避免泄露内部结构。
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                tracing::debug!(reason = %rejection, "request body rejected");
                Err(ApiError::bad_request(
                    "INVALID_REQUEST_BODY",
                    "Request body is missing or malformed",
                ))
            }
        }
    }
}
