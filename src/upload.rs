//! The write port: `PUT /upload` and `PUT /media`, reached only through the connector.

use std::sync::PoisonError;

use crate::acceptance::{self, Candidate};
use crate::auth::Authorization;
use crate::descriptor::Descriptor;
use crate::error::{ApiError, read_body};
use crate::hash::BlobHash;
use crate::holdings::Recorded;
use crate::payment::Payment;
use crate::state::{SharedState, now};
use crate::sunset::{SUNSET, sunset_value};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/upload", put(upload))
        .route("/media", put(crate::media::media))
        .with_state(state)
}

async fn upload(State(state): State<SharedState>, request: Request) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let at = now();

    let payment = Payment::from_headers(&parts.headers)?;
    let authorization = Authorization::verify(
        parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
        at,
        &state.config.domain(),
    )?;
    let claimed = acceptance::claimed_hash(&parts.headers)?;
    if let Some(claimed) = &claimed
        && !authorization.covers(claimed)
    {
        return Err(ApiError::BadRequest(
            "X-SHA-256 does not match the authorization's x tag".into(),
        ));
    }

    let read_limit = usize::try_from(state.config.max_blob_bytes).unwrap_or(usize::MAX);
    let bytes = read_body(body, read_limit).await?;
    let hash = BlobHash::of(&bytes);
    if !authorization.covers(&hash) || claimed.is_some_and(|c| c != hash) {
        return Err(ApiError::BadRequest(
            "the body's sha256 does not match the authorization's x tag or X-SHA-256".into(),
        ));
    }

    let content_type = acceptance::declared_type(&parts.headers, header::CONTENT_TYPE.as_str());
    let size = bytes.len() as u64;
    let candidate = Candidate {
        size,
        content_type: content_type.clone(),
        hash: Some(hash.clone()),
    };
    let retention = state.config.retention_secs;

    let job_state = state.clone();
    let job_hash = hash.clone();
    let job_type = content_type.clone();
    let recorded = tokio::task::spawn_blocking(move || -> Result<Recorded, ApiError> {
        let _writing = job_state
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        acceptance::check(&job_state, &candidate, at)?;
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
                now(),
                retention,
            )
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)??;

    let Recorded {
        created,
        content_type,
        uploaded,
        sunset,
        ..
    } = recorded;
    let descriptor = Descriptor::new(
        state.config.public_url(),
        &hash,
        size,
        content_type,
        uploaded,
    );
    let mut response = Json(descriptor).into_response();
    if created {
        *response.status_mut() = StatusCode::CREATED;
    }
    response.headers_mut().insert(SUNSET, sunset_value(sunset));
    Ok(response)
}
