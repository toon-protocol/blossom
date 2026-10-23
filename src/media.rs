//! The media route's image work (BUD-05): decode, drop every metadata chunk, re-encode.
//!
//! Re-encoding from decoded pixels is what strips metadata: EXIF, GPS, XMP, ICC and text chunks
//! are never carried into the output. The EXIF orientation is applied to the pixels first, so the
//! picture still shows the right way up.

use std::io::Cursor;
use std::sync::PoisonError;
use std::time::Duration;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::acceptance::{self, Candidate};
use crate::auth::Authorization;
use crate::descriptor::Descriptor;
use crate::error::{ApiError, read_body};
use crate::hash::BlobHash;
use crate::holdings::Recorded;
use crate::payment::Payment;
use crate::state::{SharedState, now};
use crate::sunset::{SUNSET, sunset_value};

/// JPEG output quality.
const JPEG_QUALITY: u8 = 85;
/// The most memory decoding may use, so a small file cannot expand into a huge bitmap.
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

/// An image format the media route accepts, found from the bytes and never from a header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format(ImageFormat);

impl Format {
    /// # Errors
    /// [`ApiError::NotAnImage`] when the bytes are not a JPEG, PNG or WebP image.
    pub fn sniff(bytes: &[u8]) -> Result<Self, ApiError> {
        match image::guess_format(bytes) {
            Ok(f @ (ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)) => Ok(Self(f)),
            _ => Err(ApiError::NotAnImage),
        }
    }

    #[must_use]
    pub fn mime(self) -> &'static str {
        self.0.to_mime_type()
    }
}

/// The optimized output: metadata-free bytes in the input's format.
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

/// # Errors
/// [`ApiError::NotAnImage`] when the bytes do not decode as the sniffed image.
pub fn optimize(bytes: &[u8], format: Format) -> Result<Optimized, ApiError> {
    let not_an_image = |_| ApiError::NotAnImage;
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format.0)
        .into_decoder()
        .map_err(not_an_image)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    decoder.set_limits(limits).map_err(not_an_image)?;
    let orientation = decoder.orientation().map_err(not_an_image)?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(not_an_image)?;
    picture.apply_orientation(orientation);

    let mut out = Vec::new();
    match format.0 {
        ImageFormat::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
            if picture.color().has_color() {
                picture.to_rgb8().write_with_encoder(encoder)
            } else {
                picture.to_luma8().write_with_encoder(encoder)
            }
        }
        ImageFormat::Png => picture.write_with_encoder(PngEncoder::new(&mut out)),
        _ => {
            let encoder = WebPEncoder::new_lossless(&mut out);
            if picture.color().has_alpha() {
                picture.to_rgba8().write_with_encoder(encoder)
            } else {
                picture.to_rgb8().write_with_encoder(encoder)
            }
        }
    }
    .map_err(ApiError::internal)?;
    Ok(Optimized {
        bytes: out,
        content_type: format.mime(),
    })
}

/// `PUT /media`: the payment statement and a `t=media` authorization whose `x` tag is the input's
/// hash. Only the output is kept, held by its owner under the output's hash.
///
/// # Errors
/// As the upload route, plus [`ApiError::NotAnImage`] for a body that is not an image and
/// [`ApiError::TooSlow`] when optimizing outlasts the deadline.
#[allow(clippy::too_many_lines)]
pub async fn media(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let at = now();

    let payment = Payment::from_headers(&parts.headers)?;
    let authorization = Authorization::verify_for(
        parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
        at,
        &state.config.domain(),
        "media",
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
    let input = read_body(body, read_limit).await?;
    let input_hash = BlobHash::of(&input);
    if !authorization.covers(&input_hash) || claimed.is_some_and(|c| c != input_hash) {
        return Err(ApiError::BadRequest(
            "the body's sha256 does not match the authorization's x tag or X-SHA-256".into(),
        ));
    }
    let format = Format::sniff(&input)?;

    // Refuse what the acceptance check would refuse before spending any work on the image.
    let input_candidate = Candidate {
        size: input.len() as u64,
        content_type: format.mime().to_owned(),
        hash: Some(input_hash),
    };
    {
        let state = state.clone();
        tokio::task::spawn_blocking(move || acceptance::check(&state, &input_candidate, at))
            .await
            .map_err(ApiError::internal)??;
    }

    let deadline = Duration::from_millis(state.config.media_deadline_ms);
    let optimizing = tokio::task::spawn_blocking(move || optimize(&input, format));
    let optimized = tokio::time::timeout(deadline, optimizing)
        .await
        .map_err(|_| ApiError::TooSlow)?
        .map_err(ApiError::internal)??;

    let hash = BlobHash::of(&optimized.bytes);
    let size = optimized.bytes.len() as u64;
    let content_type = optimized.content_type;
    let candidate = Candidate {
        size,
        content_type: content_type.to_owned(),
        hash: Some(hash.clone()),
    };
    let retention = state.config.retention_secs;
    let job_state = state.clone();
    let job_hash = hash.clone();
    let recorded = tokio::task::spawn_blocking(move || -> Result<Recorded, ApiError> {
        let _writing = job_state
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        acceptance::check(&job_state, &candidate, at)?;
        job_state
            .files
            .put(&job_hash, &optimized.bytes)
            .map_err(ApiError::internal)?;
        job_state
            .holdings
            .record(
                &job_hash,
                size,
                content_type,
                authorization.owner(),
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
