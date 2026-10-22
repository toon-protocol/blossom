//! The acceptance check every paid path runs, and the free preflight (`HEAD /upload`) runs too:
//! the blob size limit, the content-type allowlist and the total storage cap.

use axum::http::HeaderMap;

use crate::error::ApiError;
use crate::hash::BlobHash;
use crate::state::AppState;

/// What a client says it will upload. A field the client did not state is not checked.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub size: Option<u64>,
    pub content_type: Option<String>,
    pub hash: Option<BlobHash>,
}

/// # Errors
/// [`ApiError::TooLarge`] over the size limit, [`ApiError::UnsupportedType`] outside the
/// allowlist, [`ApiError::StorageFull`] when the cap is unset or the blob's bytes would not fit.
pub fn check(state: &AppState, candidate: &Candidate, now: u64) -> Result<(), ApiError> {
    let config = &state.config;
    if let Some(size) = candidate.size
        && size > config.max_blob_bytes
    {
        return Err(ApiError::TooLarge(config.max_blob_bytes));
    }
    // An empty setting (`TOON_BLOB_ALLOWED_CONTENT_TYPES=`) lists nothing, so it is no allowlist.
    let allowed: Vec<String> = config
        .allowed_content_types
        .iter()
        .flatten()
        .map(|a| self::essence(a))
        .filter(|a| !a.is_empty())
        .collect();
    if let Some(content_type) = &candidate.content_type
        && !allowed.is_empty()
    {
        let essence = self::essence(content_type);
        if !allowed.contains(&essence) {
            return Err(ApiError::UnsupportedType(content_type.clone()));
        }
    }
    let cap = config.storage_cap_bytes.ok_or(ApiError::StorageFull)?;
    // Bytes already kept add nothing, so extending a holding is never refused for space.
    let already_kept = match &candidate.hash {
        Some(hash) => state
            .holdings
            .live_blob(hash, now)
            .map_err(ApiError::internal)?
            .is_some(),
        None => false,
    };
    if !already_kept {
        let kept = state.holdings.kept_bytes(now).map_err(ApiError::internal)?;
        if kept.saturating_add(candidate.size.unwrap_or(0)) > cap {
            return Err(ApiError::StorageFull);
        }
    }
    Ok(())
}

/// The media type without parameters, lowercased: `Text/Plain; charset=utf-8` is `text/plain`.
fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// The content type a client states in header `name`; `application/octet-stream` when it states
/// none. The preflight and the paid upload read it the same way, so they check the same type.
pub fn declared_type(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned()
}
