//! The public port: free blob reads (BUD-01), open CORS, and `403` for paid paths.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, head, put};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer, ExposeHeaders};

use crate::acceptance::{self, Candidate};
use crate::error::ApiError;
use crate::hash::BlobHash;
use crate::state::{SharedState, now};
use crate::sunset::{SUNSET, sunset_value};

/// The paid paths: a `PUT` to any of them on the public port is refused.
const PAID_PATHS: [&str; 3] = ["/upload", "/mirror", "/media"];

pub fn router(state: SharedState) -> Router {
    let mut router = Router::new();
    for path in PAID_PATHS {
        router = router.route(path, put(refuse_put));
    }
    router
        .route("/upload", head(preflight))
        .route("/{name}", get(read_blob))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::any())
                .allow_methods(AllowMethods::list([
                    Method::GET,
                    Method::HEAD,
                    Method::PUT,
                    Method::DELETE,
                ]))
                .allow_headers(AllowHeaders::list([
                    header::AUTHORIZATION,
                    header::HeaderName::from_static("*"),
                ]))
                .expose_headers(ExposeHeaders::list([
                    header::CONTENT_LENGTH,
                    header::CONTENT_RANGE,
                    header::ACCEPT_RANGES,
                    SUNSET,
                    header::HeaderName::from_static("x-reason"),
                ]))
                .max_age(std::time::Duration::from_hours(24)),
        )
        .with_state(state)
}

/// BUD-06: runs the paid upload's acceptance check on the headers a client would send, for free.
async fn preflight(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let size = text("x-content-length")
        .ok_or_else(|| ApiError::BadRequest("X-Content-Length is required".into()))?
        .parse()
        .map_err(|_| ApiError::BadRequest("X-Content-Length is not a number".into()))?;
    let content_type = acceptance::declared_type(&headers, "x-content-type");
    let hash = text("x-sha-256")
        .map(|s| {
            BlobHash::parse(s)
                .ok_or_else(|| ApiError::BadRequest("X-SHA-256 is not a sha256 hash".into()))
        })
        .transpose()?;
    let candidate = Candidate {
        size: Some(size),
        content_type: Some(content_type),
        hash,
    };
    tokio::task::spawn_blocking(move || acceptance::check(&state, &candidate, now()))
        .await
        .map_err(ApiError::internal)??;
    Ok(StatusCode::OK)
}

async fn refuse_put() -> ApiError {
    ApiError::UploadsGoThroughToon
}

/// Parses `<sha256>[.<ext>]`. The extension is only a hint for clients: the blob is always served
/// with the type it was uploaded with, never one picked by whoever wrote the URL.
fn parse_name(name: &str) -> Option<BlobHash> {
    BlobHash::parse(name.split_once('.').map_or(name, |(stem, _)| stem))
}

async fn read_blob(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let hash = parse_name(&name).ok_or(ApiError::NotFound)?;
    let lookup_state = state.clone();
    let lookup_hash = hash.clone();
    let info =
        tokio::task::spawn_blocking(move || lookup_state.holdings.live_blob(&lookup_hash, now()))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?
            .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(state.files.path(&hash))
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ApiError::NotFound,
            _ => ApiError::internal(e),
        })?;

    let total = bytes.len() as u64;
    let (status, body, content_range) = match requested_range(&headers, total) {
        RangeRequest::Whole => (StatusCode::OK, bytes, None),
        RangeRequest::Partial(start, end) => {
            let slice = bytes
                [usize::try_from(start).unwrap_or(0)..=usize::try_from(end).unwrap_or(0)]
                .to_vec();
            (
                StatusCode::PARTIAL_CONTENT,
                slice,
                Some(format!("bytes {start}-{end}/{total}")),
            )
        }
        RangeRequest::Unsatisfiable => return Err(ApiError::RangeNotSatisfiable(total)),
    };

    let mut response = (status, body).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&info.content_type).map_err(ApiError::internal)?,
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(SUNSET, sunset_value(info.sunset));
    if let Some(range) = content_range {
        h.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&range).map_err(ApiError::internal)?,
        );
    }
    Ok(response)
}

#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    Whole,
    /// Inclusive byte offsets.
    Partial(u64, u64),
    Unsatisfiable,
}

/// A single `bytes=` range. Anything else (no header, another unit, several ranges, an invalid
/// range such as `5-2`) serves the whole blob, as RFC 9110 permits.
fn requested_range(headers: &HeaderMap, total: u64) -> RangeRequest {
    let Some(spec) = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
    else {
        return RangeRequest::Whole;
    };
    if spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangeRequest::Whole;
    };
    let (start, end) = match (first.parse::<u64>(), last.parse::<u64>()) {
        (Ok(start), Ok(end)) if start > end => return RangeRequest::Whole,
        (Ok(start), Ok(end)) => (start, end.min(total.saturating_sub(1))),
        (Ok(start), Err(_)) if last.is_empty() => (start, total.saturating_sub(1)),
        (Err(_), Ok(n)) if first.is_empty() && n > 0 => {
            (total.saturating_sub(n), total.saturating_sub(1))
        }
        (Err(_), Ok(_)) if first.is_empty() => return RangeRequest::Unsatisfiable,
        _ => return RangeRequest::Whole,
    };
    if total == 0 || start >= total {
        RangeRequest::Unsatisfiable
    } else {
        RangeRequest::Partial(start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(value: &str, total: u64) -> RangeRequest {
        let mut h = HeaderMap::new();
        h.insert(header::RANGE, HeaderValue::from_str(value).unwrap());
        requested_range(&h, total)
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(range("bytes=0-3", 10), RangeRequest::Partial(0, 3));
        assert_eq!(range("bytes=4-", 10), RangeRequest::Partial(4, 9));
        assert_eq!(range("bytes=-3", 10), RangeRequest::Partial(7, 9));
        assert_eq!(range("bytes=8-99", 10), RangeRequest::Partial(8, 9));
        assert_eq!(range("bytes=10-12", 10), RangeRequest::Unsatisfiable);
        assert_eq!(range("bytes=5-2", 10), RangeRequest::Whole);
        assert_eq!(range("bytes=0-1,4-5", 10), RangeRequest::Whole);
        assert_eq!(range("items=0-1", 10), RangeRequest::Whole);
    }

    #[test]
    fn splits_names() {
        let hash = BlobHash::of(b"x");
        assert_eq!(parse_name(hash.as_str()), Some(hash.clone()));
        assert_eq!(parse_name(&format!("{hash}.png")), Some(hash));
        assert_eq!(parse_name("nope"), None);
    }
}
