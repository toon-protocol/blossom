//! The mirror route (BUD-04): `PUT /mirror` on the write port. The app fetches a blob from another
//! server itself, behind the mirror guard, and records a holding exactly as an upload does.
//!
//! The mirror guard keeps a paid mirror off the bundle's internal network: only `https` URLs, the
//! host resolved here, and any address that is not public refused, on the first request and on
//! every redirect hop. The connection is made to the address that was checked, so the name cannot
//! resolve to something else between the check and the fetch. Origins in
//! `TOON_MIRROR_ALLOWED_ORIGINS` are exempt.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header;
use axum::response::Response;
use reqwest::Url;
use serde::Deserialize;

use crate::acceptance;
use crate::auth::Authorization;
use crate::error::{ApiError, read_body};
use crate::hash::BlobHash;
use crate::payment::Payment;
use crate::state::{AppState, SharedState, now};

const MAX_REDIRECTS: usize = 5;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// A mirror request's own body is one URL.
const REQUEST_LIMIT: usize = 16 * 1024;

#[derive(Deserialize)]
struct MirrorRequest {
    url: String,
}

pub(crate) async fn mirror(
    State(state): State<SharedState>,
    request: Request,
) -> Result<Response, ApiError> {
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
    let body = read_body(body, REQUEST_LIMIT).await?;
    let MirrorRequest { url } = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("the body must be JSON with a `url`".into()))?;
    let url = Url::parse(&url).map_err(|_| ApiError::BadRequest("`url` is not a URL".into()))?;

    let (bytes, content_type) = fetch(&state, url).await?;
    let hash = BlobHash::of(&bytes);
    if !authorization.covers(&hash) {
        return Err(ApiError::Conflict(
            "the fetched bytes do not hash to the authorization's x tag".into(),
        ));
    }
    crate::upload::accept_and_hold(
        &state,
        bytes,
        hash,
        content_type,
        authorization,
        payment,
        at,
    )
    .await
}

/// Fetches `url`, following redirects by hand so each hop meets the guard. Returns the bytes and
/// the source's content type (`application/octet-stream` when it states none).
async fn fetch(state: &AppState, mut url: Url) -> Result<(Bytes, String), ApiError> {
    let limit = state.config.mirror_max_bytes;
    for _ in 0..=MAX_REDIRECTS {
        let (host, addrs) = guard(state, &url).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            // A proxy would resolve the host again itself, past the guard.
            .no_proxy()
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(ApiError::internal)?;
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| ApiError::BadGateway(format!("could not fetch the source: {e}")))?;

        if response.status().is_redirection() {
            let next = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .ok_or_else(|| {
                    ApiError::BadGateway("the source redirected without a usable Location".into())
                })?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(ApiError::BadGateway(format!(
                "the source answered {}",
                response.status()
            )));
        }
        if response.content_length().is_some_and(|n| n > limit) {
            return Err(ApiError::TooLarge(limit));
        }
        let content_type =
            acceptance::declared_type(response.headers(), header::CONTENT_TYPE.as_str());
        // Streamed, so a source that lies about (or omits) its length is cut off at the limit.
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ApiError::BadGateway(format!("could not read the source: {e}")))?
        {
            if (body.len() + chunk.len()) as u64 > limit {
                return Err(ApiError::TooLarge(limit));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok((Bytes::from(body), content_type));
    }
    Err(ApiError::BadGateway(
        "the source redirected too many times".into(),
    ))
}

/// The mirror guard for one hop: the host and the addresses the fetch may connect to.
async fn guard(state: &AppState, url: &Url) -> Result<(String, Vec<SocketAddr>), ApiError> {
    let exempt = state
        .mirror_origins
        .contains(&url.origin().ascii_serialization());
    // An exempt origin may also be plain `http`.
    if !(url.scheme() == "https" || exempt && url.scheme() == "http") {
        return Err(refused("only https URLs can be mirrored"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| refused("the URL has no host"))?
        .to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((bare, port))
        .await
        .map_err(|e| ApiError::BadGateway(format!("could not resolve {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(ApiError::BadGateway(format!("{host} has no address")));
    }
    if !exempt && let Some(bad) = addrs.iter().find(|a| !is_public(a.ip())) {
        return Err(refused(&format!(
            "{host} resolves to {}, which is not a public address",
            bad.ip()
        )));
    }
    Ok((host, addrs))
}

fn refused(why: &str) -> ApiError {
    ApiError::BadRequest(format!("mirror refused: {why}"))
}

/// Whether `ip` is a public address: not loopback, private, link-local, unique-local, shared,
/// multicast, documentation or otherwise reserved. An IPv6 address that embeds an IPv4 one is
/// judged by the IPv4 address.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

fn v4_from(high: u16, low: u16) -> Ipv4Addr {
    let [o1, o2] = high.to_be_bytes();
    let [o3, o4] = low.to_be_bytes();
    Ipv4Addr::new(o1, o2, o3, o4)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let seg = ip.segments();
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4 address.
    if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6].iter().all(|&x| x == 0) {
        return is_public_v4(v4_from(seg[6], seg[7]));
    }
    if seg[0] == 0x2002 {
        return is_public_v4(v4_from(seg[1], seg[2]));
    }
    // Only global unicast (2000::/3) is public; that already leaves out loopback, unspecified,
    // IPv4-compatible, link-local, site-local, unique-local and multicast addresses. Within it,
    // the IETF protocol assignments (2001::/23, Teredo among them) and documentation are not.
    (seg[0] & 0xe000) == 0x2000 && !(seg[0] == 0x2001 && (seg[1] < 0x0200 || seg[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(ip: &str) -> bool {
        is_public(ip.parse().unwrap())
    }

    #[test]
    fn refuses_every_non_public_address() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "2001:db8::1",
            "ff02::1",
            "fec0::1",
            "::127.0.0.1",
            "2001::1",
            "100::1",
        ] {
            assert!(!public(ip), "{ip} must be refused");
        }
    }

    #[test]
    fn allows_public_addresses() {
        for ip in ["93.184.216.34", "8.8.8.8", "172.32.0.1", "2606:4700::1111"] {
            assert!(public(ip), "{ip} must be allowed");
        }
    }
}
