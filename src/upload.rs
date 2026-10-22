//! The write port: `PUT /upload`, reached only through the connector.

use std::sync::PoisonError;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use http_body_util::LengthLimitError;
use serde::Serialize;

use crate::auth::UploadAuthorization;
use crate::error::ApiError;
use crate::hash::BlobHash;
use crate::holdings::Recorded;
use crate::payment::Payment;
use crate::state::{SharedState, now};
use crate::sunset::{SUNSET, sunset_value};

/// The connector caps a paid request at 2 MiB; a body larger than this is refused outright.
pub const MAX_BLOB_BYTES: usize = 2 * 1024 * 1024;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/upload", put(upload))
        .with_state(state)
}

/// A BUD-02 blob descriptor with BUD-08 `nip94` tags.
#[derive(Debug, Serialize)]
struct Descriptor {
    url: String,
    sha256: String,
    size: u64,
    #[serde(rename = "type")]
    content_type: String,
    uploaded: u64,
    nip94: Vec<Vec<String>>,
}

async fn upload(State(state): State<SharedState>, request: Request) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let at = now();

    let payment = Payment::from_headers(&parts.headers)?;
    let authorization = UploadAuthorization::verify(
        parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
        at,
    )?;
    let claimed = claimed_hash(&parts.headers)?;
    if let Some(claimed) = &claimed
        && !authorization.covers(claimed)
    {
        return Err(ApiError::BadRequest(
            "X-SHA-256 does not match the authorization's x tag".into(),
        ));
    }

    let bytes = to_bytes(body, MAX_BLOB_BYTES).await.map_err(|e| {
        let inner = e.into_inner();
        if inner.is::<LengthLimitError>() {
            ApiError::TooLarge(MAX_BLOB_BYTES)
        } else {
            ApiError::BadRequest(format!("could not read the body: {inner}"))
        }
    })?;
    let hash = BlobHash::of(&bytes);
    if !authorization.covers(&hash) || claimed.is_some_and(|c| c != hash) {
        return Err(ApiError::BadRequest(
            "the body's sha256 does not match the authorization's x tag or X-SHA-256".into(),
        ));
    }

    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let size = bytes.len() as u64;
    let retention = state.config.retention_secs;

    let job_state = state.clone();
    let job_hash = hash.clone();
    let job_type = content_type.clone();
    let recorded = tokio::task::spawn_blocking(move || -> Result<Recorded, ApiError> {
        let _writing = job_state
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        job_state
            .files
            .put(&job_hash, &bytes)
            .map_err(ApiError::internal)?;
        job_state
            .holdings
            .record(
                &job_hash,
                size,
                &job_type,
                &authorization.owner,
                &payment,
                at,
                retention,
            )
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)??;

    let Recorded {
        content_type,
        uploaded,
        expires_at,
    } = recorded;
    let url = blob_url(state.config.public_url(), &hash, &content_type);
    let descriptor = Descriptor {
        nip94: vec![
            vec!["url".into(), url.clone()],
            vec!["m".into(), content_type.clone()],
            vec!["x".into(), hash.to_string()],
            vec!["size".into(), size.to_string()],
        ],
        url,
        sha256: hash.to_string(),
        size,
        content_type,
        uploaded,
    };
    let mut response = Json(descriptor).into_response();
    response
        .headers_mut()
        .insert(SUNSET, sunset_value(expires_at));
    Ok(response)
}

fn claimed_hash(headers: &HeaderMap) -> Result<Option<BlobHash>, ApiError> {
    headers
        .get("x-sha-256")
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|s| BlobHash::parse(s.trim()))
                .ok_or_else(|| ApiError::BadRequest("X-SHA-256 is not a sha256 hash".into()))
        })
        .transpose()
}

fn blob_url(public_url: &str, hash: &BlobHash, content_type: &str) -> String {
    let essence = content_type.split(';').next().unwrap_or("").trim();
    match mime_guess::get_mime_extensions_str(essence).and_then(|e| e.first()) {
        Some(ext) => format!("{public_url}/{hash}.{ext}"),
        None => format!("{public_url}/{hash}"),
    }
}
