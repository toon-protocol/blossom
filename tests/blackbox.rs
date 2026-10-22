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
    let before = now();
    let (first_descriptor, first) = put_blob(&app, &signer, &body, "text/plain");
    let after = now();
    let (second_descriptor, second) = put_blob(&app, &signer, &body, "application/octet-stream");
    // Paying early loses no time: the second payment adds a whole retention period to the first.
    let retention = (first - after)..=(first - before);
    assert!(
        retention.contains(&(second - first)),
        "{second} should be {first} plus a retention period in {retention:?}"
    );
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
    let first_owner = Signer::random();
    put_blob(&app, &first_owner, &body, "text/plain");
    let (_, later) = put_blob(&app, &first_owner, &body, "text/plain");
    // The second owner's holding expires first, so the blob's `Sunset` stays the first owner's.
    let (_, sunset) = put_blob(&app, &Signer::random(), &body, "text/plain");
    assert_eq!(sunset, later);
    let read = app.read(&sha256_hex(&body)).send().unwrap();
    assert_eq!(status(&read), 200);
    assert_eq!(sunset_secs(&read), later);
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

fn put_report(app: &App, body: String) -> Response {
    app.client
        .put(format!("{}/report", app.public_url))
        .body(body)
        .send()
        .unwrap()
}

#[test]
fn a_valid_report_is_accepted_free_and_changes_nothing() {
    let app = App::start();
    let body = random_blob(3000);
    let hash = sha256_hex(&body);
    let (_, sunset) = put_blob(&app, &Signer::random(), &body, "image/png");

    let report = Signer::random().report(1984, &[vec!["x", &hash, "illegal"]], false);
    assert_eq!(status(&put_report(&app, report)), 200);

    let response = app.read(&hash).send().unwrap();
    assert_eq!(status(&response), 200);
    assert_eq!(sunset_secs(&response), sunset);
    assert_eq!(response.bytes().unwrap().as_ref(), body.as_slice());
}

#[test]
fn a_bad_report_is_refused() {
    let app = App::start();
    let hash = sha256_hex(b"anything");
    let signer = Signer::random();
    let tags = [vec!["x", hash.as_str(), "spam"]];
    for body in [
        signer.report(1984, &tags, true),
        signer.report(1, &tags, false),
        signer.report(1984, &[], false),
        signer.report(1984, &[vec!["x", "not-a-hash"]], false),
        "not an event".to_owned(),
    ] {
        assert_eq!(status(&put_report(&app, body)), 400);
    }
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

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

fn read_status(app: &App, hash: &str) -> (u16, u16) {
    let get = status(&app.read(hash).send().unwrap());
    let head = status(
        &app.client
            .head(format!("{}/{hash}", app.public_url))
            .send()
            .unwrap(),
    );
    (get, head)
}

#[test]
fn a_blob_is_404_after_its_last_holding_expires_and_stays_readable_while_another_is_live() {
    let Some(app) = App::start_short(6, 1) else {
        return;
    };
    let (early, late) = (Signer::random(), Signer::random());
    let body = random_blob(200);
    let hash = sha256_hex(&body);
    let (_, early_sunset) = put_blob(&app, &early, &body, "text/plain");
    std::thread::sleep(std::time::Duration::from_secs(3));
    let (_, late_sunset) = put_blob(&app, &late, &body, "text/plain");
    assert!(late_sunset > early_sunset);
    assert_eq!(sunset_secs(&app.read(&hash).send().unwrap()), late_sunset);

    wait_until("the first holding to expire", || now() >= early_sunset);
    assert_eq!(read_status(&app, &hash), (200, 200));
    let read = app.read(&hash).send().unwrap();
    assert_eq!(sunset_secs(&read), late_sunset);
    assert_eq!(read.bytes().unwrap().as_ref(), body.as_slice());

    wait_until("the last holding to expire", || {
        read_status(&app, &hash) == (404, 404)
    });
}

#[test]
fn an_expired_and_swept_blob_can_be_uploaded_again_afresh() {
    let Some(app) = App::start_short(2, 1) else {
        return;
    };
    let signer = Signer::random();
    let body = random_blob(100);
    let hash = sha256_hex(&body);
    put_blob(&app, &signer, &body, "text/plain");
    wait_until("expiry", || read_status(&app, &hash).0 == 404);
    // Outlast a sweep interval, so the blob's bytes have been deleted.
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(read_status(&app, &hash), (404, 404));

    let before = now();
    let (descriptor, sunset) = put_blob(&app, &signer, &body, "image/png");
    assert!((before + 2..=now() + 2).contains(&sunset), "{sunset}");
    assert_eq!(descriptor["type"], "image/png");
    assert!(descriptor["uploaded"].as_u64().unwrap() >= before);
    let read = app.read(&hash).send().unwrap();
    assert_eq!(status(&read), 200);
    assert_eq!(read.bytes().unwrap().as_ref(), body.as_slice());
}

/// What a paid upload of `body` answers, with the status and the `X-Reason`.
fn paid_upload(app: &App, body: &[u8], content_type: &str) -> (u16, Option<String>) {
    let response = app
        .upload(body)
        .header("authorization", Signer::random().upload_authorization(body))
        .header("content-type", content_type)
        .send()
        .unwrap();
    let reason = response
        .headers()
        .get("x-reason")
        .map(|v| v.to_str().unwrap().to_owned());
    (status(&response), reason)
}

/// What the preflight answers for the same blob.
fn preflight(app: &App, body: &[u8], content_type: &str) -> (u16, Option<String>) {
    let response = app
        .preflight(body.len(), content_type, Some(&sha256_hex(body)))
        .send()
        .unwrap();
    let reason = response
        .headers()
        .get("x-reason")
        .map(|v| v.to_str().unwrap().to_owned());
    (status(&response), reason)
}

/// The refusal in an answer, or `None` when it accepts: the preflight accepts with `200` and a
/// new upload with `201`, so any `2xx` is acceptance.
fn refusal(answer: &(u16, Option<String>)) -> Option<&(u16, Option<String>)> {
    (!(200..300).contains(&answer.0)).then_some(answer)
}

/// Asserts the preflight and the paid upload both accept or refuse alike, and returns the
/// preflight's status.
fn both_agree(app: &App, body: &[u8], content_type: &str) -> u16 {
    let checked = preflight(app, body, content_type);
    let paid = paid_upload(app, body, content_type);
    assert_eq!(
        refusal(&checked),
        refusal(&paid),
        "the preflight and the paid upload disagree: {checked:?} and {paid:?}"
    );
    checked.0
}

#[test]
fn with_no_storage_cap_set_the_preflight_and_the_upload_both_refuse() {
    let Some(app) = App::start_with(&[]) else {
        return;
    };
    let body = random_blob(100);
    assert_eq!(both_agree(&app, &body, "text/plain"), 507);
}

#[test]
fn a_blob_over_the_size_limit_gets_413_from_the_preflight_and_the_upload() {
    let Some(app) =
        App::start_with(&["--storage-cap-bytes", "1000000", "--max-blob-bytes", "1000"])
    else {
        return;
    };
    assert_eq!(both_agree(&app, &random_blob(1001), "text/plain"), 413);
    assert_eq!(both_agree(&app, &random_blob(1000), "text/plain"), 200);
}

#[test]
fn the_default_size_limit_is_just_under_the_connectors_2_mib_cap() {
    let app = App::start();
    let limit = 2 * 1024 * 1024;
    let just_under_2_mib = app.preflight(limit - 1, "text/plain", None).send().unwrap();
    assert_eq!(status(&just_under_2_mib), 413);
    let fits = app.preflight(1_900_000, "text/plain", None).send().unwrap();
    assert_eq!(status(&fits), 200);
}

#[test]
fn a_type_outside_the_allowlist_is_refused_by_both_and_a_listed_type_is_accepted_by_both() {
    let Some(app) = App::start_with(&[
        "--storage-cap-bytes",
        "1000000",
        "--allowed-content-types",
        "image/png,text/plain",
    ]) else {
        return;
    };
    assert_eq!(both_agree(&app, &random_blob(50), "application/zip"), 415);
    assert_eq!(both_agree(&app, &random_blob(50), "image/png"), 200);
    assert_eq!(
        both_agree(&app, &random_blob(50), "Text/Plain; charset=utf-8"),
        200
    );
}

#[test]
fn once_the_storage_cap_would_be_exceeded_both_refuse() {
    let Some(app) = App::start_with(&["--storage-cap-bytes", "250"]) else {
        return;
    };
    let first = random_blob(200);
    assert_eq!(both_agree(&app, &first, "text/plain"), 200);
    assert_eq!(both_agree(&app, &random_blob(100), "text/plain"), 507);
    // Bytes already kept add nothing, so extending the holding still fits.
    assert_eq!(both_agree(&app, &first, "text/plain"), 200);
    assert_eq!(both_agree(&app, &random_blob(50), "text/plain"), 200);
}

#[test]
fn the_preflight_is_free_and_stores_nothing() {
    let app = App::start();
    let body = random_blob(100);
    assert_eq!(preflight(&app, &body, "text/plain").0, 200);
    assert_eq!(status(&app.read(&sha256_hex(&body)).send().unwrap()), 404);
}

#[test]
fn a_preflight_without_a_length_is_411_and_a_bad_hash_is_400() {
    let app = App::start();
    let no_length = app
        .client
        .head(format!("{}/upload", app.public_url))
        .header("x-content-type", "text/plain")
        .send()
        .unwrap();
    assert_eq!(status(&no_length), 411);
    let bad_hash = app
        .preflight(100, "text/plain", Some("not-a-hash"))
        .send()
        .unwrap();
    assert_eq!(status(&bad_hash), 400);
}

fn delete(app: &App, hash: &str, authorization: Option<String>) -> Response {
    let mut request = app.client.delete(format!("{}/{hash}", app.public_url));
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    request.send().unwrap()
}

fn delete_authorization(signer: &Signer, hash: &str) -> String {
    signer.authorization("delete", hash, now() - 10, now() + 600)
}

fn listed(app: &App, pubkey: &str) -> Vec<serde_json::Value> {
    let response = app.read(&format!("list/{pubkey}")).send().unwrap();
    assert_eq!(status(&response), 200);
    response.json().unwrap()
}

#[test]
fn an_owners_delete_ends_their_holding_and_the_blob_is_404() {
    let app = App::start();
    let owner = Signer::random();
    let body = random_blob(300);
    let hash = sha256_hex(&body);
    put_blob(&app, &owner, &body, "text/plain");

    let response = delete(&app, &hash, Some(delete_authorization(&owner, &hash)));
    assert!(status(&response) == 200 || status(&response) == 204);
    assert_eq!(read_status(&app, &hash), (404, 404));
    assert!(listed(&app, &owner.pubkey()).is_empty());
    // The holding is gone and with it the blob, so a second delete finds nothing.
    let again = delete(&app, &hash, Some(delete_authorization(&owner, &hash)));
    assert_eq!(status(&again), 404);
}

#[test]
fn with_two_owners_one_delete_leaves_the_blob_readable_with_the_remaining_sunset() {
    let Some(app) = App::start_short(6, 1) else {
        return;
    };
    let (first, second) = (Signer::random(), Signer::random());
    let body = random_blob(300);
    let hash = sha256_hex(&body);
    let (_, first_sunset) = put_blob(&app, &first, &body, "text/plain");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let (_, second_sunset) = put_blob(&app, &second, &body, "text/plain");
    assert!(first_sunset < second_sunset);

    // The later holding goes; the sunset falls back to the earlier one.
    let response = delete(&app, &hash, Some(delete_authorization(&second, &hash)));
    assert!(status(&response) == 200 || status(&response) == 204);
    let read = app.read(&hash).send().unwrap();
    assert_eq!(status(&read), 200);
    assert_eq!(sunset_secs(&read), first_sunset);
    assert_eq!(read.bytes().unwrap().as_ref(), body.as_slice());
    assert_eq!(read_status(&app, &hash), (200, 200));
    assert_eq!(listed(&app, &first.pubkey()).len(), 1);
    assert!(listed(&app, &second.pubkey()).is_empty());
}

#[test]
fn a_delete_by_a_non_owner_or_without_a_valid_authorization_changes_nothing() {
    let app = App::start();
    let (owner, stranger) = (Signer::random(), Signer::random());
    let body = random_blob(300);
    let other = random_blob(300);
    let hash = sha256_hex(&body);
    put_blob(&app, &owner, &body, "text/plain");

    let refused = delete(&app, &hash, Some(delete_authorization(&stranger, &hash)));
    assert_eq!(status(&refused), 403);
    assert_eq!(status(&delete(&app, &hash, None)), 401);
    let upload_verb = owner.authorization("upload", &hash, now() - 10, now() + 600);
    assert_eq!(status(&delete(&app, &hash, Some(upload_verb))), 401);
    let wrong_hash = delete_authorization(&owner, &sha256_hex(&other));
    assert_eq!(status(&delete(&app, &hash, Some(wrong_hash))), 401);
    let expired = owner.authorization("delete", &hash, now() - 100, now() - 50);
    assert_eq!(status(&delete(&app, &hash, Some(expired))), 401);

    assert_eq!(read_status(&app, &hash), (200, 200));
    assert_eq!(listed(&app, &owner.pubkey()).len(), 1);
}

#[test]
fn list_returns_exactly_the_pubkeys_live_blobs_and_drops_deleted_and_expired_ones() {
    let Some(app) = App::start_short(4, 1) else {
        return;
    };
    let (owner, other) = (Signer::random(), Signer::random());
    let (kept, deleted, expiring, others) = (
        random_blob(100),
        random_blob(110),
        random_blob(120),
        random_blob(130),
    );
    put_blob(&app, &owner, &expiring, "text/plain");
    std::thread::sleep(std::time::Duration::from_secs(3));
    put_blob(&app, &owner, &kept, "image/png");
    put_blob(&app, &owner, &deleted, "text/plain");
    put_blob(&app, &other, &others, "text/plain");

    let hash = sha256_hex(&deleted);
    delete(&app, &hash, Some(delete_authorization(&owner, &hash)));
    wait_until("the first blob to expire", || {
        read_status(&app, &sha256_hex(&expiring)).0 == 404
    });

    let list = listed(&app, &owner.pubkey());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["sha256"], sha256_hex(&kept));
    assert_eq!(list[0]["size"], 100);
    assert_eq!(list[0]["type"], "image/png");
    assert!(
        list[0]["url"]
            .as_str()
            .unwrap()
            .contains(&sha256_hex(&kept))
    );
    assert!(list[0]["nip94"].is_array());
    assert_eq!(listed(&app, &other.pubkey()).len(), 1);
    assert!(listed(&app, &Signer::random().pubkey()).is_empty());
}

#[test]
fn list_pages_with_cursor_and_limit_and_refuses_malformed_ones() {
    let app = App::start();
    let owner = Signer::random();
    for _ in 0..3 {
        put_blob(&app, &owner, &random_blob(100), "text/plain");
    }
    let pubkey = owner.pubkey();
    let all = listed(&app, &pubkey);
    assert_eq!(all.len(), 3);

    let first = listed(&app, &format!("{pubkey}?limit=1"));
    assert_eq!(first, all[..1]);
    let cursor = all[0]["sha256"].as_str().unwrap();
    let rest = listed(&app, &format!("{pubkey}?cursor={cursor}"));
    assert_eq!(rest, all[1..]);

    for query in ["limit=abc", "limit=-1", "cursor=nothex"] {
        let response = app.read(&format!("list/{pubkey}?{query}")).send().unwrap();
        assert_eq!(status(&response), 400, "{query}");
    }
}

#[test]
fn a_delete_of_a_blob_no_one_holds_is_404() {
    let app = App::start();
    let hash = sha256_hex(&random_blob(100));
    let response = delete(
        &app,
        &hash,
        Some(delete_authorization(&Signer::random(), &hash)),
    );
    assert_eq!(status(&response), 404);
}
