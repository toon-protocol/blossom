//! The `Sunset` header (RFC 8594): when a blob's retention ends. Both ports send it.

use axum::http::{HeaderName, HeaderValue};

pub const SUNSET: HeaderName = HeaderName::from_static("sunset");

/// An HTTP-date `Sunset` header value for a unix-seconds expiry.
pub(crate) fn sunset_value(expires_at: u64) -> HeaderValue {
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(expires_at);
    HeaderValue::from_str(&httpdate::fmt_http_date(when))
        .unwrap_or_else(|_| HeaderValue::from_static(""))
}
