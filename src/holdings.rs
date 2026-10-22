//! Blob metadata and holdings, in SQLite.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OptionalExtension, params};

use crate::hash::BlobHash;
use crate::payment::Payment;

/// What the blob app knows about a blob that has at least one live holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobInfo {
    pub size: u64,
    pub content_type: String,
    pub uploaded: u64,
    /// The latest expiry across the blob's live holdings: when its bytes may go.
    pub sunset: u64,
}

/// What a paid upload recorded: the blob as first recorded (a later upload of the same bytes keeps
/// the first `uploaded` and type) and the expiry of this owner's holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// Whether this call stored the blob's bytes for the first time (no `blobs` row existed for them).
    pub created: bool,
    pub content_type: String,
    pub uploaded: u64,
    pub expires_at: u64,
}

pub struct Holdings {
    db: Mutex<Connection>,
}

impl Holdings {
    /// # Errors
    /// Fails if the database cannot be opened or migrated.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    fn init(db: Connection) -> rusqlite::Result<Self> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS blobs (
                 sha256 TEXT PRIMARY KEY,
                 size INTEGER NOT NULL,
                 content_type TEXT NOT NULL,
                 uploaded INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS holdings (
                 sha256 TEXT NOT NULL REFERENCES blobs(sha256),
                 owner TEXT NOT NULL,
                 expires_at INTEGER NOT NULL,
                 payer TEXT NOT NULL,
                 amount TEXT NOT NULL,
                 chain TEXT NOT NULL,
                 PRIMARY KEY (sha256, owner)
             );",
        )?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a paid upload as the owner's holding on the blob. The holding's expiry becomes
    /// `max(now, current expiry) + retention`. The payer, amount and chain are kept in their own
    /// columns, apart from the owner.
    ///
    /// # Errors
    /// Fails on a database error; nothing is recorded then.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        hash: &BlobHash,
        size: u64,
        content_type: &str,
        owner: &str,
        payment: &Payment,
        now: u64,
        retention: u64,
    ) -> rusqlite::Result<Recorded> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let created = tx.execute(
            "INSERT OR IGNORE INTO blobs (sha256, size, content_type, uploaded) VALUES (?1, ?2, ?3, ?4)",
            params![hash.as_str(), size, content_type, now],
        )? == 1;
        let (content_type, uploaded): (String, u64) = tx.query_row(
            "SELECT content_type, uploaded FROM blobs WHERE sha256 = ?1",
            params![hash.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let current: Option<u64> = tx
            .query_row(
                "SELECT expires_at FROM holdings WHERE sha256 = ?1 AND owner = ?2",
                params![hash.as_str(), owner],
                |row| row.get(0),
            )
            .optional()?;
        let expires_at = current.map_or(now, |c| c.max(now)) + retention;
        tx.execute(
            "INSERT INTO holdings (sha256, owner, expires_at, payer, amount, chain)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (sha256, owner) DO UPDATE SET
                 expires_at = excluded.expires_at, payer = excluded.payer,
                 amount = excluded.amount, chain = excluded.chain",
            params![
                hash.as_str(),
                owner,
                expires_at,
                payment.payer,
                payment.amount.to_string(),
                payment.chain
            ],
        )?;
        tx.commit()?;
        Ok(Recorded {
            created,
            content_type,
            uploaded,
            expires_at,
        })
    }

    /// The blob, if any holding on it is still live at `now`.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn live_blob(&self, hash: &BlobHash, now: u64) -> rusqlite::Result<Option<BlobInfo>> {
        self.db()
            .query_row(
                "SELECT b.size, b.content_type, b.uploaded, MAX(h.expires_at)
             FROM blobs b JOIN holdings h ON h.sha256 = b.sha256
             WHERE b.sha256 = ?1 AND h.expires_at > ?2
             GROUP BY b.sha256",
                params![hash.as_str(), now],
                |row| {
                    Ok(BlobInfo {
                        size: row.get(0)?,
                        content_type: row.get(1)?,
                        uploaded: row.get(2)?,
                        sunset: row.get(3)?,
                    })
                },
            )
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment() -> Payment {
        Payment {
            payer: "p".into(),
            amount: 1010,
            chain: "c".into(),
        }
    }

    #[test]
    fn reupload_extends_from_the_later_of_now_and_expiry() {
        let h = Holdings::init(Connection::open_in_memory().unwrap()).unwrap();
        let hash = BlobHash::of(b"x");
        assert_eq!(
            h.record(&hash, 1, "text/plain", "o", &payment(), 100, 50)
                .unwrap()
                .expires_at,
            150
        );
        assert_eq!(
            h.record(&hash, 1, "text/plain", "o", &payment(), 120, 50)
                .unwrap()
                .expires_at,
            200
        );
        assert_eq!(
            h.record(&hash, 1, "text/plain", "o", &payment(), 500, 50)
                .unwrap()
                .expires_at,
            550
        );
    }

    #[test]
    fn holdings_of_two_owners_are_separate_and_sunset_is_the_latest() {
        let h = Holdings::init(Connection::open_in_memory().unwrap()).unwrap();
        let hash = BlobHash::of(b"x");
        h.record(&hash, 1, "text/plain", "a", &payment(), 100, 50)
            .unwrap();
        h.record(&hash, 1, "text/plain", "b", &payment(), 200, 50)
            .unwrap();
        assert_eq!(h.live_blob(&hash, 160).unwrap().unwrap().sunset, 250);
        assert!(h.live_blob(&hash, 250).unwrap().is_none());
    }
}
