use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// A refusal, rendered with BUD-01's `X-Reason` header and the same text as the body.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("missing or malformed payment statement (X-TOON-Payer / X-TOON-Amount / X-TOON-Chain)")]
    PaymentStatement,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("uploads go through TOON: send a paid request to the upload route on the connector")]
    UploadsGoThroughToon,
    #[error("blob not found")]
    NotFound,
    #[error("blob is larger than the {0} byte limit")]
    TooLarge(u64),
    #[error("content type {0} is not allowed on this server")]
    UnsupportedType(String),
    #[error(
        "the storage cap is not set or would be exceeded: this server is not accepting uploads"
    )]
    StorageFull,
    #[error("range not satisfiable")]
    RangeNotSatisfiable(u64),
    #[error("internal error")]
    Internal(String),
}

impl ApiError {
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::Internal(err.to_string())
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::PaymentStatement | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::UploadsGoThroughToon => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::StorageFull => StatusCode::INSUFFICIENT_STORAGE,
            Self::RangeNotSatisfiable(_) => StatusCode::RANGE_NOT_SATISFIABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            eprintln!("internal error: {detail}");
        }
        let message = self.to_string();
        let mut response = (self.status(), message.clone()).into_response();
        if let Ok(value) = HeaderValue::from_str(&message) {
            response.headers_mut().insert("x-reason", value);
        }
        if let Self::RangeNotSatisfiable(size) = self
            && let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}"))
        {
            response.headers_mut().insert("content-range", value);
        }
        response
    }
}
