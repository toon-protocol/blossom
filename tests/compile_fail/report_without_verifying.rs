//! A `Report` is proof its event was signed by its reporter, so it cannot be built unverified.

use blossom::report::Report;

fn main() {
    let _ = Report {
        event_id: String::new(),
        reporter: String::new(),
        blobs: Vec::new(),
        content: String::new(),
        raw: String::new(),
    };
}
