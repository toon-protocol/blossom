//! The takedown: at startup, ends every holding on each listed blob and removes its bytes.

use std::sync::PoisonError;

use crate::hash::BlobHash;
use crate::state::{SharedState, now};
use crate::sweep::collect_unheld;

/// Takes down every blob on the takedown list and logs the list in force. Blobs never held are
/// no-ops, so the list may name hashes this server has not seen.
///
/// # Errors
/// Fails on a database or file error, which should stop startup: a listed blob must not stay up.
pub fn apply(state: &SharedState) -> Result<(), Box<dyn std::error::Error>> {
    let mut listed: Vec<&str> = state.taken_down.iter().map(BlobHash::as_str).collect();
    listed.sort_unstable();
    eprintln!(
        "takedown list in force ({}): {}",
        listed.len(),
        listed.join(",")
    );
    let _writing = state
        .write_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for hash in &state.taken_down {
        let ended = state.holdings.end_all_holdings(hash)?;
        // A file with no row (a part-finished upload) goes too.
        state.files.remove(hash)?;
        if ended > 0 {
            eprintln!("takedown ended {ended} holding(s) on {hash}");
        }
    }
    collect_unheld(state, now())?;
    Ok(())
}
