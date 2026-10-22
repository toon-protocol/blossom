use sha2::{Digest, Sha256};
use std::fmt;

/// A blob's address: a sha256 digest, held as 64 lowercase hex characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlobHash(String);

impl BlobHash {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(hex::encode(Sha256::digest(bytes)))
    }

    /// Parses 64 hex characters; uppercase is accepted and normalised.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(text.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_64_hex_characters() {
        let hash = BlobHash::of(b"hello");
        assert_eq!(BlobHash::parse(hash.as_str()), Some(hash.clone()));
        assert_eq!(BlobHash::parse(&hash.as_str().to_uppercase()), Some(hash));
        assert_eq!(BlobHash::parse("abc"), None);
        assert_eq!(BlobHash::parse(&"g".repeat(64)), None);
    }
}
