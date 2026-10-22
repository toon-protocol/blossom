//! The app's black-box suite: asserts only on what comes back from the two ports.

mod common;

use common::{App, Signer, now, random_blob, sha256_hex, status};
use reqwest::blocking::Response;

fn header(response: &Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default()
}

fn sunset_secs(response: &Response) -> u64 {
    let text = header(response, "sunset");
    httpdate::parse_http_date(&text)
        .unwrap_or_else(|_| panic!("Sunset is not an HTTP date: {text:?}"))
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Uploads `body` as `signer`, expecting success; returns the descriptor and the `Sunset` seconds.
fn put_blob(
    app: &App,
    signer: &Signer,
    body: &[u8],
    content_type: &str,
) -> (serde_json::Value, u64) {
    let response = app
        .upload(body)
        .header("authorization", signer.upload_authorization(body))
        .header("content-type", content_type)
        .send()
        .unwrap();
    assert!(matches!(status(&response), 200 | 201));
    let sunset = sunset_secs(&response);
    (response.json().unwrap(), sunset)
}

#[test]
fn a_paid_upload_returns_a_descriptor_with_nip94_tags_and_sunset() {
    let app = App::start();
    let body = random_blob(2048);
    let hash = sha256_hex(&body);
    let (descriptor, sunset) = put_blob(&app, &Signer::random(), &body, "image/png");

    assert_eq!(descriptor["sha256"], hash);
    assert_eq!(descriptor["size"], 2048);
    assert_eq!(descriptor["type"], "image/png");
    assert!(descriptor["uploaded"].as_u64().unwrap() <= now());
    let url = descriptor["url"].as_str().unwrap();
    assert!(url.ends_with(&format!("/{hash}.png")), "{url}");

    let nip94 = descriptor["nip94"].as_array().unwrap();
    let tag = |name: &str| {
        nip94
            .iter()
            .find(|t| t[0] == name)
            .unwrap_or_else(|| panic!("no {name} tag"))[1]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(tag("url"), url);
    assert_eq!(tag("m"), "image/png");
    assert_eq!(tag("x"), hash);
    assert_eq!(tag("size"), "2048");
    assert!(sunset > now());
}

#[test]
fn uploaded_bytes_are_served_unchanged_by_hash_and_by_hash_with_extension() {
    let app = App::start();
    let body = random_blob(5000);
    let hash = sha256_hex(&body);
    let (_, sunset) = put_blob(&app, &Signer::random(), &body, "image/png");

    // The extension never changes the type served: `.html` must not turn a blob into a page.
    for path in [hash.clone(), format!("{hash}.png"), format!("{hash}.html")] {
        let response = app.read(&path).send().unwrap();
        assert_eq!(status(&response), 200, "{path}");
        assert_eq!(header(&response, "content-type"), "image/png");
        assert_eq!(header(&response, "x-content-type-options"), "nosniff");
        assert_eq!(header(&response, "content-length"), "5000");
        assert_eq!(sunset_secs(&response), sunset);
        assert_eq!(response.bytes().unwrap().as_ref(), body.as_slice());
    }

    let head = app
        .client
        .head(format!("{}/{hash}", app.public_url))
        .send()
        .unwrap();
    assert_eq!(status(&head), 200);
    assert_eq!(header(&head, "content-length"), "5000");
    assert_eq!(header(&head, "content-type"), "image/png");
    assert_eq!(sunset_secs(&head), sunset);
    assert!(head.bytes().unwrap().is_empty());
}

#[test]
fn range_requests_return_the_requested_bytes() {
    let app = App::start();
    let body = random_blob(1000);
    let hash = sha256_hex(&body);
    put_blob(&app, &Signer::random(), &body, "application/octet-stream");

    let get = |range: &str| app.read(&hash).header("range", range).send().unwrap();

    let response = get("bytes=10-19");
    assert_eq!(status(&response), 206);
    assert_eq!(header(&response, "content-range"), "bytes 10-19/1000");
    assert_eq!(response.bytes().unwrap().as_ref(), &body[10..20]);

    let response = get("bytes=990-");
    assert_eq!(status(&response), 206);
    assert_eq!(response.bytes().unwrap().as_ref(), &body[990..]);

    let response = get("bytes=-5");
    assert_eq!(status(&response), 206);
    assert_eq!(response.bytes().unwrap().as_ref(), &body[995..]);

    // An invalid range is ignored: the whole blob comes back.
    let response = get("bytes=20-10");
    assert_eq!(status(&response), 200);
    assert_eq!(response.bytes().unwrap().as_ref(), body.as_slice());

    let response = get("bytes=5000-6000");
    assert_eq!(status(&response), 416);
    assert_eq!(header(&response, "content-range"), "bytes */1000");
}

#[test]
fn reads_carry_open_cors_and_the_preflight_is_answered() {
    let app = App::start();
    let body = random_blob(64);
    let hash = sha256_hex(&body);
    put_blob(&app, &Signer::random(), &body, "text/plain");

    let response = app
        .read(&hash)
        .header("origin", "https://app.example")
        .send()
        .unwrap();
    assert_eq!(header(&response, "access-control-allow-origin"), "*");
    assert!(
        header(&response, "access-control-expose-headers")
            .to_lowercase()
            .contains("sunset")
    );

    let preflight = app
        .client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/{hash}", app.public_url),
        )
        .header("origin", "https://app.example")
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "authorization")
        .send()
        .unwrap();
    assert!(preflight.status().is_success(), "{}", preflight.status());
    assert_eq!(header(&preflight, "access-control-allow-origin"), "*");
    assert!(
        header(&preflight, "access-control-allow-headers")
            .to_lowercase()
            .contains("authorization")
    );
    assert!(!header(&preflight, "access-control-allow-methods").is_empty());
}

#[test]
fn an_unknown_hash_is_404() {
    let app = App::start();
    let hash = sha256_hex(&random_blob(32));
    assert_eq!(status(&app.read(&hash).send().unwrap()), 404);
    assert_eq!(
        status(&app.read(&format!("{hash}.png")).send().unwrap()),
        404
    );
    assert_eq!(status(&app.read("not-a-hash").send().unwrap()), 404);
    assert_eq!(
        status(
            &app.client
                .head(format!("{}/{hash}", app.public_url))
                .send()
                .unwrap()
        ),
        404
    );
}

#[test]
fn an_upload_without_a_payment_statement_is_refused() {
    let app = App::start();
    let body = random_blob(100);
    let response = app
        .client
        .put(format!("{}/upload", app.write_url))
        .header(
            "authorization",
            Signer::random().upload_authorization(&body),
        )
        .body(body.clone())
        .send()
        .unwrap();
    assert_eq!(status(&response), 400);
    assert_eq!(status(&app.read(&sha256_hex(&body)).send().unwrap()), 404);
}

#[test]
fn an_upload_with_a_partial_payment_statement_is_refused() {
    let app = App::start();
    let body = random_blob(100);
    let response = app
        .client
        .put(format!("{}/upload", app.write_url))
        .header("x-toon-payer", "0xchannelkey")
        .header("x-toon-amount", "1010")
        .header(
            "authorization",
            Signer::random().upload_authorization(&body),
        )
        .body(body)
        .send()
        .unwrap();
    assert_eq!(status(&response), 400);
}

#[test]
fn an_upload_with_a_missing_invalid_expired_or_wrong_verb_authorization_is_refused() {
    let app = App::start();
    let signer = Signer::random();
    let body = random_blob(100);
    let hash = sha256_hex(&body);

    let attempt = |authorization: Option<String>| {
        let mut request = app.upload(&body);
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        status(&request.send().unwrap())
    };

    assert_eq!(attempt(None), 401, "missing");
    assert_eq!(attempt(Some("Nostr !!!".into())), 401, "garbage");
    let mut forged = signer.upload_authorization(&body);
    forged.insert(forged.len() - 8, 'A');
    assert_eq!(attempt(Some(forged)), 401, "tampered");
    assert_eq!(
        attempt(Some(signer.authorization(
            "upload",
            &hash,
            now() - 100,
            now() - 1
        ))),
        401,
        "expired"
    );
    assert_eq!(
        attempt(Some(signer.authorization(
            "delete",
            &hash,
            now() - 10,
            now() + 600
        ))),
        401,
        "verb"
    );
    assert_eq!(
        attempt(Some(signer.authorization(
            "upload",
            &hash,
            now() + 600,
            now() + 900
        ))),
        401,
        "future"
    );

    assert_eq!(
        status(&app.read(&hash).send().unwrap()),
        404,
        "nothing was recorded"
    );
}

#[test]
fn an_upload_whose_body_does_not_hash_to_x_or_x_sha_256_is_refused() {
    let app = App::start();
    let signer = Signer::random();
    let body = random_blob(100);
    let other = random_blob(100);

    let response = app
        .upload(&body)
        .header("authorization", signer.upload_authorization(&other))
        .send()
        .unwrap();
    assert_eq!(status(&response), 400, "body does not match x");

    let response = app
        .upload(&body)
        .header("authorization", signer.upload_authorization(&body))
        .header("x-sha-256", sha256_hex(&other))
        .send()
        .unwrap();
    assert_eq!(status(&response), 400, "body does not match X-SHA-256");

    let response = app
        .upload(&body)
        .header("authorization", signer.upload_authorization(&body))
        .header("x-sha-256", sha256_hex(&body))
        .send()
        .unwrap();
    assert_eq!(status(&response), 201, "a matching X-SHA-256 is accepted");

    assert_eq!(status(&app.read(&sha256_hex(&other)).send().unwrap()), 404);
}

#[test]
fn a_put_to_a_paid_path_on_the_public_port_is_403_and_says_uploads_go_through_toon() {
    let app = App::start();
    let body = random_blob(100);
    for path in ["upload", "mirror", "media"] {
        let response = app
            .client
            .put(format!("{}/{path}", app.public_url))
            .header(
                "authorization",
                Signer::random().upload_authorization(&body),
            )
            .body(body.clone())
            .send()
            .unwrap();
        assert_eq!(status(&response), 403, "{path}");
        assert!(response.text().unwrap().contains("TOON"), "{path}");
    }
    assert_eq!(status(&app.read(&sha256_hex(&body)).send().unwrap()), 404);
}

#[test]
fn uploading_again_extends_the_same_owners_holding() {
    let app = App::start();
    let signer = Signer::random();
    let body = random_blob(300);
    let (first_descriptor, first) = put_blob(&app, &signer, &body, "text/plain");
    let (second_descriptor, second) = put_blob(&app, &signer, &body, "application/octet-stream");
    assert!(second > first, "{second} should be after {first}");
    // The descriptor describes the blob as first recorded, not the second request.
    assert_eq!(second_descriptor["uploaded"], first_descriptor["uploaded"]);
    assert_eq!(second_descriptor["type"], "text/plain");
    let read = app.read(&sha256_hex(&body)).send().unwrap();
    assert_eq!(sunset_secs(&read), second);
}

#[test]
fn two_owners_uploading_the_same_bytes_keep_one_blob() {
    let app = App::start();
    let body = random_blob(300);
    put_blob(&app, &Signer::random(), &body, "text/plain");
    put_blob(&app, &Signer::random(), &body, "text/plain");
    let read = app.read(&sha256_hex(&body)).send().unwrap();
    assert_eq!(status(&read), 200);
    assert_eq!(read.bytes().unwrap().as_ref(), body.as_slice());
}

#[test]
fn an_upload_over_the_size_limit_is_413() {
    let app = App::start();
    let body = random_blob(2 * 1024 * 1024 + 1);
    let response = app
        .upload(&body)
        .header(
            "authorization",
            Signer::random().upload_authorization(&body),
        )
        .send()
        .unwrap();
    assert_eq!(status(&response), 413);
}

#[test]
fn a_new_blob_is_201_created_and_the_same_bytes_again_are_200() {
    let app = App::start();
    let signer = Signer::random();
    let body = random_blob(512);
    let send = |signer: &Signer| {
        app.upload(&body)
            .header("authorization", signer.upload_authorization(&body))
            .send()
            .unwrap()
    };
    assert_eq!(status(&send(&signer)), 201);
    assert_eq!(status(&send(&signer)), 200);
    assert_eq!(
        status(&send(&Signer::random())),
        200,
        "another owner, same bytes"
    );
}
