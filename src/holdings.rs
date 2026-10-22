//! Blob metadata and holdings, in SQLite.

use std::path::Path;
use std::sync::Mutex;

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

    /// Records a paid upload as the owner's holding on the blob and returns that holding's expiry:
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
    ) -> rusqlite::Result<u64> {
        let mut db = self
            .db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tx = db.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO blobs (sha256, size, content_type, uploaded) VALUES (?1, ?2, ?3, ?4)",
            params![hash.as_str(), size, content_type, now],
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
        Ok(expires_at)
    }

    /// The blob, if any holding on it is still live at `now`.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn live_blob(&self, hash: &BlobHash, now: u64) -> rusqlite::Result<Option<BlobInfo>> {
        let db = self
            .db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        db.query_row(
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
                .unwrap(),
            150
        );
        assert_eq!(
            h.record(&hash, 1, "text/plain", "o", &payment(), 120, 50)
                .unwrap(),
            200
        );
        assert_eq!(
            h.record(&hash, 1, "text/plain", "o", &payment(), 500, 50)
                .unwrap(),
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
