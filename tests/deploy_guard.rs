//! Guards on the bundle files: the compose files, `connector.toml` and the Caddyfile.
//! They read the files as text and parse them; nothing here needs Docker.

use std::fs;
use std::path::PathBuf;

use serde_yaml_ng::Value as Yaml;

fn deploy_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("deploy")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn yaml(name: &str) -> Yaml {
    serde_yaml_ng::from_str(&deploy_file(name)).expect("valid YAML")
}

fn service<'a>(compose: &'a Yaml, name: &str) -> &'a Yaml {
    compose
        .get("services")
        .and_then(|s| s.get(name))
        .unwrap_or_else(|| panic!("service {name} missing"))
}

fn strings(value: Option<&Yaml>) -> Vec<String> {
    value
        .and_then(Yaml::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn own_bundle_runs_connector_blob_app_and_caddy() {
    let compose = yaml("docker-compose.yml");
    let mut names: Vec<_> = compose["services"]
        .as_mapping()
        .unwrap()
        .keys()
        .filter_map(Yaml::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["blossom", "caddy", "connector"]);
}

/// One immutable connector build, moved only by a reviewed commit here and in the compose file.
const EXPECTED_CONNECTOR_IMAGE: &str = "ghcr.io/toon-protocol/connector:rust-2026.10.23.1";

/// `rust-sha-<commit>` or `rust-<YYYY.MM.DD.N>`: tags nothing ever repoints.
fn is_immutable_connector_tag(tag: &str) -> bool {
    let all = |s: &str, f: fn(&char) -> bool| !s.is_empty() && s.chars().all(|c| f(&c));
    if let Some(sha) = tag.strip_prefix("rust-sha-") {
        return (7..=40).contains(&sha.len()) && all(sha, char::is_ascii_hexdigit);
    }
    let Some(release) = tag.strip_prefix("rust-") else {
        return false;
    };
    let parts: Vec<_> = release.split('.').collect();
    parts.len() == 4
        && [4, 2, 2].iter().zip(&parts).all(|(n, p)| p.len() == *n)
        && parts.iter().all(|p| all(p, char::is_ascii_digit))
}

#[test]
fn connector_is_on_an_immutable_pin() {
    let compose = yaml("docker-compose.yml");
    let image = service(&compose, "connector")["image"].as_str().unwrap();
    assert_eq!(image, EXPECTED_CONNECTOR_IMAGE);
    let tag = image
        .strip_prefix("ghcr.io/toon-protocol/connector:")
        .expect("stock connector image");
    assert!(
        is_immutable_connector_tag(tag),
        "{tag} is not an immutable handle"
    );
    assert!(!deploy_file("docker-compose.yml").contains("rust-release"));
}

#[test]
fn moving_connector_tags_are_not_immutable() {
    for tag in [
        "rust-release",
        "rust-main",
        "rust-latest",
        "latest",
        "rust-sha-xyz1234",
    ] {
        assert!(!is_immutable_connector_tag(tag), "{tag}");
    }
    for tag in ["rust-sha-8708caf", "rust-2026.09.29.1"] {
        assert!(is_immutable_connector_tag(tag), "{tag}");
    }
}

#[test]
fn blobs_live_on_a_named_volume() {
    let compose = yaml("docker-compose.yml");
    let mounts = strings(service(&compose, "blossom").get("volumes"));
    assert!(mounts.contains(&"blob_data:/data".to_owned()), "{mounts:?}");
    assert!(
        compose["volumes"].get("blob_data").is_some(),
        "blob_data must be a declared named volume"
    );
}

#[test]
fn connector_toml_is_mounted_read_only() {
    let compose = yaml("docker-compose.yml");
    let mounts = strings(service(&compose, "connector").get("volumes"));
    assert!(
        mounts.contains(&"./connector.toml:/app/config/connector.toml:ro".to_owned()),
        "{mounts:?}"
    );
}

/// A service's `ports:` entries, in any compose syntax, as YAML text for matching.
fn published_ports(svc: &Yaml) -> Vec<String> {
    svc.get("ports")
        .and_then(Yaml::as_sequence)
        .map(|seq| {
            seq.iter()
                .map(|v| serde_yaml_ng::to_string(v).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn only_caddy_publishes_ports_and_never_the_write_port() {
    let compose = yaml("docker-compose.yml");
    for (name, svc) in compose["services"].as_mapping().unwrap() {
        let name = name.as_str().unwrap();
        if name != "caddy" {
            assert!(svc.get("ports").is_none(), "{name} publishes ports");
        }
        for port in published_ports(svc) {
            assert!(
                !port.contains("3001"),
                "{name} publishes the write port: {port}"
            );
        }
    }
}

#[test]
fn connector_toml_holds_the_three_routes_at_devnet_prices() {
    let config: toml::Table = deploy_file("connector.toml").parse().expect("valid TOML");
    let routes = config["routes"].as_array().expect("[[routes]]");
    assert_eq!(routes.len(), 3);
    let route = |prefix: &str| {
        routes
            .iter()
            .find(|r| r["prefix"].as_str() == Some(prefix))
            .unwrap_or_else(|| panic!("route {prefix} missing"))
    };
    for (prefix, handler) in [
        ("g.toon.blossom.upload", "http://blossom:3001/upload"),
        ("g.toon.blossom.mirror", "http://blossom:3001/mirror"),
        ("g.toon.blossom.media", "http://blossom:3001/media"),
    ] {
        assert_eq!(route(prefix)["handler_url"].as_str(), Some(handler));
    }
    for prefix in ["g.toon.blossom.upload", "g.toon.blossom.media"] {
        let price = route(prefix)["price"]
            .as_table()
            .expect("a base + per_kib price");
        assert_eq!(price["base"].as_integer(), Some(1000));
        assert_eq!(price["per_kib"].as_integer(), Some(10));
    }
    assert_eq!(
        route("g.toon.blossom.mirror")["price"].as_integer(),
        Some(25_000)
    );
}

#[test]
fn caddy_proxies_only_the_public_port() {
    let caddyfile = deploy_file("Caddyfile");
    assert!(
        !caddyfile.contains("3001"),
        "Caddy must never reach the write port"
    );
    for line in caddyfile.lines().filter(|l| l.contains("reverse_proxy")) {
        let target = line.trim().trim_start_matches("reverse_proxy").trim();
        assert!(
            ["blossom:3000", "connector:4000"].contains(&target),
            "unexpected upstream {target}"
        );
    }
    let words: Vec<_> = caddyfile.split_whitespace().collect();
    assert!(
        words
            .windows(4)
            .any(|w| w[0] == "handle" && w[1] == "{" && w[2] == "respond" && w[3].starts_with('"')),
        "unmatched requests must be refused"
    );
}

#[test]
fn shared_edge_turns_the_connector_and_caddy_off_and_joins_edge_blossom() {
    let overlay = yaml("docker-compose.shared-edge.yml");
    for name in ["connector", "caddy"] {
        assert_eq!(
            strings(service(&overlay, name).get("profiles")),
            ["disabled"],
            "{name}"
        );
    }
    let blossom = service(&overlay, "blossom");
    let networks = blossom["networks"].as_mapping().unwrap();
    assert!(networks.contains_key("edge-blossom") && networks.contains_key("default"));
    assert_eq!(
        overlay["networks"]["edge-blossom"]["external"].as_bool(),
        Some(true)
    );
}

#[test]
fn shared_edge_leaves_the_blob_app_unchanged() {
    let overlay = yaml("docker-compose.shared-edge.yml");
    let blossom = service(&overlay, "blossom");
    for key in [
        "image",
        "environment",
        "volumes",
        "ports",
        "expose",
        "command",
        "entrypoint",
        "env_file",
        "user",
        "profiles",
        "deploy",
    ] {
        assert!(
            blossom.get(key).is_none(),
            "the overlay must not set blossom.{key}"
        );
    }
}
