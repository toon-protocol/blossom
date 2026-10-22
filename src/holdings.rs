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
/// the first `uploaded` and type while any holding on it is live), the expiry of this owner's
/// holding, and the blob's sunset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub content_type: String,
    pub uploaded: u64,
    pub expires_at: u64,
    /// The latest expiry across the blob's live holdings, this one included.
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

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a paid upload as the owner's holding on the blob. The holding's expiry becomes
    /// `max(now, current expiry) + retention`. A blob with no live holding left is recorded afresh,
    /// as if the sweep had already run. The payer, amount and chain are kept in their own columns,
    /// apart from the owner.
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
        tx.execute(
            "INSERT INTO blobs (sha256, size, content_type, uploaded) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (sha256) DO UPDATE SET
                 size = excluded.size, content_type = excluded.content_type,
                 uploaded = excluded.uploaded
             WHERE NOT EXISTS (
                 SELECT 1 FROM holdings WHERE sha256 = excluded.sha256 AND expires_at > ?4
             )",
            params![hash.as_str(), size, content_type, now],
        )?;
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
        let sunset: u64 = tx.query_row(
            "SELECT MAX(expires_at) FROM holdings WHERE sha256 = ?1",
            params![hash.as_str()],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(Recorded {
            content_type,
            uploaded,
            expires_at,
            sunset,
        })
    }

    /// Ends the holdings that expired at or before `now` and returns every blob left with none, so
    /// their bytes can be deleted and then the blobs forgotten. A blob stays listed until it is
    /// forgotten, so a pass that fails part-way is finished by the next.
    ///
    /// # Errors
    /// Fails on a database error; nothing is ended then.
    pub fn sweep(&self, now: u64) -> rusqlite::Result<Vec<BlobHash>> {
        let db = self.db();
        db.execute("DELETE FROM holdings WHERE expires_at <= ?1", params![now])?;
        let mut stmt = db.prepare(
            "SELECT sha256 FROM blobs WHERE sha256 NOT IN (SELECT sha256 FROM holdings)",
        )?;
        let unheld: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(unheld.iter().filter_map(|h| BlobHash::parse(h)).collect())
    }

    /// Forgets the blob if no holding is left on it; call once its bytes are deleted.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn forget(&self, hash: &BlobHash) -> rusqlite::Result<()> {
        self.db().execute(
            "DELETE FROM blobs WHERE sha256 = ?1
             AND sha256 NOT IN (SELECT sha256 FROM holdings)",
            params![hash.as_str()],
        )?;
        Ok(())
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

    #[test]
    fn sweep_ends_expired_holdings_and_returns_only_unheld_blobs() {
        let h = Holdings::init(Connection::open_in_memory().unwrap()).unwrap();
        let (x, y) = (BlobHash::of(b"x"), BlobHash::of(b"y"));
        h.record(&x, 1, "text/plain", "a", &payment(), 100, 50)
            .unwrap();
        h.record(&x, 1, "text/plain", "b", &payment(), 200, 50)
            .unwrap();
        h.record(&y, 1, "text/plain", "a", &payment(), 100, 50)
            .unwrap();
        assert_eq!(h.sweep(160).unwrap(), vec![y.clone()]);
        // Until forgotten, an unheld blob is listed again, so a failed delete is retried.
        assert_eq!(h.sweep(160).unwrap(), vec![y.clone()]);
        h.forget(&y).unwrap();
        assert!(h.sweep(160).unwrap().is_empty());
        assert!(h.live_blob(&x, 160).unwrap().is_some());
        assert_eq!(h.sweep(250).unwrap(), vec![x]);
    }

    #[test]
    fn sunset_is_the_latest_live_expiry_and_an_expired_blob_is_recorded_afresh() {
        let h = Holdings::init(Connection::open_in_memory().unwrap()).unwrap();
        let hash = BlobHash::of(b"x");
        h.record(&hash, 1, "text/plain", "a", &payment(), 100, 50)
            .unwrap();
        h.record(&hash, 1, "text/plain", "a", &payment(), 100, 50)
            .unwrap();
        let b = h
            .record(&hash, 1, "image/png", "b", &payment(), 120, 50)
            .unwrap();
        assert_eq!((b.expires_at, b.sunset), (170, 200));
        assert_eq!((b.content_type.as_str(), b.uploaded), ("text/plain", 100));

        let again = h
            .record(&hash, 1, "image/png", "a", &payment(), 300, 50)
            .unwrap();
        assert_eq!((again.expires_at, again.sunset), (350, 350));
        assert_eq!(
            (again.content_type.as_str(), again.uploaded),
            ("image/png", 300)
        );
    }
}
