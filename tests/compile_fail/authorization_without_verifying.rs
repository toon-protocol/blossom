//! An `Authorization` is proof its event was signed by its owner, so it cannot be built from a
//! pubkey nobody checked.

use blossom::auth::Authorization;

fn main() {
    let _ = Authorization {
        owner: "0".repeat(64),
        hashes: Vec::new(),
    };
}
