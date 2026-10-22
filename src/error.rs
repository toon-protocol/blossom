use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::LengthLimitError;

/// A refusal, rendered with BUD-01's `X-Reason` header and the same text as the body.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("missing or malformed payment statement (X-TOON-Payer / X-TOON-Amount / X-TOON-Chain)")]
    PaymentStatement,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("uploads go through TOON: send a paid request to the upload route on the connector")]
    UploadsGoThroughToon,
    #[error("blob not found")]
    NotFound,
    #[error("X-Content-Length is required")]
    LengthRequired,
    #[error("blob is larger than the {0} byte limit")]
    TooLarge(u64),
    #[error("content type {0} is not allowed on this server")]
    UnsupportedType(String),
    #[error(
        "the storage cap is not set or would be exceeded: this server is not accepting uploads"
    )]
    StorageFull,
    #[error("this blob is not accepted on this server")]
    TakenDown,
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
            Self::Forbidden(_) | Self::UploadsGoThroughToon | Self::TakenDown => {
                StatusCode::FORBIDDEN
            }
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::LengthRequired => StatusCode::LENGTH_REQUIRED,
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

/// Reads a request body of at most `limit` bytes.
///
/// # Errors
/// [`ApiError::TooLarge`] for a body over `limit`, [`ApiError::BadRequest`] for one that cannot be
/// read.
pub async fn read_body(body: Body, limit: usize) -> Result<Bytes, ApiError> {
    to_bytes(body, limit).await.map_err(|e| {
        let inner = e.into_inner();
        if inner.is::<LengthLimitError>() {
            ApiError::TooLarge(u64::try_from(limit).unwrap_or(u64::MAX))
        } else {
            ApiError::BadRequest(format!("could not read the body: {inner}"))
        }
    })
}
