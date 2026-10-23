//! A `BlobHash` is 64 lowercase hex characters, so it cannot be built from any other string.

use blossom::hash::BlobHash;

fn main() {
    let _ = BlobHash("not a hash".to_owned());
}
