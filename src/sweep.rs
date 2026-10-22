//! The sweep: ends expired holdings and deletes the bytes of blobs no holding is left on.

use std::sync::PoisonError;
use std::time::Duration;

use crate::state::{SharedState, now};

/// One pass. Returns how many blobs were deleted.
///
/// # Errors
/// Fails on a database or file error; the next pass retries what was missed.
pub fn sweep_once(state: &SharedState) -> Result<usize, Box<dyn std::error::Error>> {
    let _writing = state
        .write_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let unheld = state.holdings.sweep(now())?;
    for hash in &unheld {
        state.files.remove(hash)?;
        state.holdings.forget(hash)?;
    }
    Ok(unheld.len())
}

/// Sweeps every `interval` for as long as the process runs.
pub async fn run(state: SharedState, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let pass = state.clone();
        match tokio::task::spawn_blocking(move || sweep_once(&pass).map_err(|e| e.to_string()))
            .await
        {
            Ok(Ok(0)) => {}
            Ok(Ok(n)) => eprintln!("sweep deleted {n} unheld blob(s)"),
            Ok(Err(e)) => eprintln!("sweep failed: {e}"),
            Err(e) => eprintln!("sweep failed: {e}"),
        }
    }
}
