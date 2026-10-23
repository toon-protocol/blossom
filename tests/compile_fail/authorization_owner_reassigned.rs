//! An authorization's owner is the key that signed it, so it cannot be changed afterwards.

use blossom::auth::Authorization;

fn steal(mut authorization: Authorization) {
    authorization.owner = "0".repeat(64);
}

fn main() {}
