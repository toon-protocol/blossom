//! The BUD-02 blob descriptor, with BUD-08 `nip94` tags.

use serde::Serialize;

use crate::hash::BlobHash;

/// A BUD-02 blob descriptor with BUD-08 `nip94` tags.
#[derive(Debug, Serialize)]
pub struct Descriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub content_type: String,
    pub uploaded: u64,
    pub nip94: Vec<Vec<String>>,
}

impl Descriptor {
    #[must_use]
    pub fn new(
        public_url: &str,
        hash: &BlobHash,
        size: u64,
        content_type: String,
        uploaded: u64,
    ) -> Self {
        let url = blob_url(public_url, hash, &content_type);
        Self {
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
        }
    }
}

fn blob_url(public_url: &str, hash: &BlobHash, content_type: &str) -> String {
    let essence = content_type.split(';').next().unwrap_or("").trim();
    match mime_guess::get_mime_extensions_str(essence).and_then(|e| e.first()) {
        Some(ext) => format!("{public_url}/{hash}.{ext}"),
        None => format!("{public_url}/{hash}"),
    }
}
