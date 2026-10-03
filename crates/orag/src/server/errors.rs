//! Uniform JSON error responses: `{"error": {"code": "...", "message": "..."}}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::error::OragError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// `Retry-After` seconds; private, so only `busy()` can set it (D-019).
    retry_after_secs: Option<u32>,
}

pub type ApiResult<T> = Result<T, ApiError>;

/// `Retry-After` seconds sent with `busy`.
const RETRY_AFTER_SECS: u32 = 5;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            retry_after_secs: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", message)
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal server error",
        )
    }

    /// `503 shutting_down`: the service is stopping and starts no new work.
    pub fn shutting_down() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "shutting_down",
            "the service is stopping",
        )
    }

    /// `429 busy`: a fixed capacity limit was hit (D-019); the response carries `Retry-After: 5`.
    pub fn busy(message: impl Into<String>) -> Self {
        ApiError {
            retry_after_secs: Some(RETRY_AFTER_SECS),
            ..Self::new(StatusCode::TOO_MANY_REQUESTS, "busy", message)
        }
    }

    /// A body the JSON extractor refused: over the body limit is `413 too_large`, anything else 400.
    pub fn json_rejection(status: StatusCode, message: String) -> Self {
        match status {
            StatusCode::PAYLOAD_TOO_LARGE => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "request body is too large",
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "send the body as JSON with Content-Type: application/json",
            ),
            _ => Self::invalid(message),
        }
    }

    pub fn body(&self) -> Value {
        json!({ "error": { "code": self.code, "message": self.message } })
    }
}

impl From<OragError> for ApiError {
    fn from(err: OragError) -> Self {
        match &err {
            OragError::NotFound { .. } => {
                Self::new(StatusCode::NOT_FOUND, "not_found", err.to_string())
            }
            OragError::Conflict(_) => Self::new(StatusCode::CONFLICT, "conflict", err.to_string()),
            OragError::ReindexRequired { .. } => {
                Self::new(StatusCode::CONFLICT, "reindex_required", err.to_string())
            }
            OragError::UnsupportedFormat(_) => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_format",
                err.to_string(),
            ),
            OragError::InvalidInput(_) => Self::invalid(err.to_string()),
            _ => {
                tracing::error!(error = %err, "internal error");
                Self::internal()
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body())).into_response();
        if let Some(secs) = self.retry_after_secs {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(secs),
            );
        }
        response
    }
}

/// `Path` whose rejection is the API's JSON `invalid_input` error.
pub struct ApiPath<T>(pub T);

impl<T, S> axum::extract::FromRequestParts<S> for ApiPath<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, ApiError> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| ApiPath(value))
            .map_err(|e| ApiError::invalid(e.body_text()))
    }
}

/// `Query` whose rejection is the API's JSON `invalid_input` error.
pub struct ApiQuery<T>(pub T);

impl<T, S> axum::extract::FromRequestParts<S> for ApiQuery<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, ApiError> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| ApiQuery(value))
            .map_err(|e| ApiError::invalid(e.body_text()))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::RETRY_AFTER;
    use axum::response::IntoResponse;

    use super::*;

    #[test]
    fn only_busy_carries_retry_after() {
        let busy = ApiError::busy("full").into_response();
        assert_eq!(
            (busy.status(), busy.headers()[RETRY_AFTER].to_str().unwrap()),
            (StatusCode::TOO_MANY_REQUESTS, "5")
        );
        // A future 429 with other semantics must choose its own retry hint.
        let other_429 =
            ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "x").into_response();
        assert!(other_429.headers().get(RETRY_AFTER).is_none());
        assert!(
            ApiError::invalid("x")
                .into_response()
                .headers()
                .get(RETRY_AFTER)
                .is_none()
        );
        assert_eq!(
            ApiError::shutting_down().into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn a_missing_json_content_type_is_415() {
        let err = ApiError::json_rejection(StatusCode::UNSUPPORTED_MEDIA_TYPE, "x".into());
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(err.code, "unsupported_media_type");
    }
}
