//! Persists ConnectorStatus rows in a small app SQLite database.
use super::ConnectorStatus;
use crate::types::{AppError, AppResult};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct ConnectorStore {
    db: Connection,
}

impl ConnectorStore {
    pub fn open(directory: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(directory).map_err(|_| AppError::storage())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
        }
        let path = directory.join("connectors.sqlite3");
        Self::initialize(Connection::open(path).map_err(|_| AppError::storage())?)
    }

    pub fn initialize(db: Connection) -> AppResult<Self> {
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS connectors (
                 id TEXT PRIMARY KEY,
                 provider TEXT NOT NULL,
                 scopes TEXT NOT NULL,
                 display_name TEXT,
                 connected_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL,
                 disconnected INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        let has_disconnected = db
            .prepare("PRAGMA table_info(connectors)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|name| name == "disconnected");
        if !has_disconnected {
            db.execute_batch(
                "ALTER TABLE connectors ADD COLUMN disconnected INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Ok(Self { db })
    }

    pub fn list(&self) -> AppResult<Vec<ConnectorStatus>> {
        let mut stmt = self.db.prepare(
            "SELECT id, provider, scopes, display_name, connected_at, expires_at \
             FROM connectors WHERE disconnected=0 ORDER BY connected_at, id",
        )?;
        let rows = stmt.query_map([], row_to_status)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> AppResult<Option<ConnectorStatus>> {
        let mut stmt = self.db.prepare(
            "SELECT id, provider, scopes, display_name, connected_at, expires_at \
             FROM connectors WHERE id=?1 AND disconnected=0",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_status(row)?)),
            None => Ok(None),
        }
    }

    pub fn upsert(&self, status: &ConnectorStatus) -> AppResult<()> {
        self.save(status, false)
    }

    pub fn stage(&self, status: &ConnectorStatus) -> AppResult<()> {
        self.save(status, true)
    }

    fn save(&self, status: &ConnectorStatus, disconnected: bool) -> AppResult<()> {
        let scopes = serde_json::to_string(&status.scopes).map_err(|_| AppError::storage())?;
        self.db.execute(
            "INSERT INTO connectors(id, provider, scopes, display_name, connected_at, expires_at, disconnected) \
             VALUES(?1,?2,?3,?4,?5,?6,?7) \
             ON CONFLICT(id) DO UPDATE SET \
                provider=excluded.provider, \
                scopes=excluded.scopes, \
                display_name=excluded.display_name, \
                connected_at=excluded.connected_at, \
                expires_at=excluded.expires_at, \
                disconnected=excluded.disconnected",
            params![
                status.id,
                status.provider,
                scopes,
                status.display_name,
                status.connected_at,
                status.expires_at,
                disconnected
            ],
        )?;
        Ok(())
    }

    /// Durable local revocation precedes credential cleanup; hidden across restart.
    pub fn mark_disconnected(&self, id: &str) -> AppResult<bool> {
        Ok(self
            .db
            .execute("UPDATE connectors SET disconnected=1 WHERE id=?1", [id])?
            > 0)
    }

    pub fn cleanup_ids(&self) -> AppResult<Vec<String>> {
        Ok(self
            .db
            .prepare("SELECT id FROM connectors WHERE disconnected=1 ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete(&self, id: &str) -> AppResult<bool> {
        let n = self
            .db
            .execute("DELETE FROM connectors WHERE id=?1", [id])?;
        Ok(n > 0)
    }
}

fn row_to_status(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectorStatus> {
    let scopes: String = row.get(2)?;
    let scopes: Vec<String> = serde_json::from_str(&scopes).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "scopes",
            )),
        )
    })?;
    Ok(ConnectorStatus {
        id: row.get(0)?,
        provider: row.get(1)?,
        scopes,
        display_name: row.get(3)?,
        connected_at: row.get(4)?,
        expires_at: row.get(5)?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn memory() -> ConnectorStore {
        ConnectorStore::initialize(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn sample(id: &str) -> ConnectorStatus {
        ConnectorStatus {
            id: id.into(),
            provider: "google".into(),
            scopes: vec!["a".into(), "b".into()],
            display_name: Some("Alex".into()),
            connected_at: "2026-01-01T00:00:00.000Z".into(),
            expires_at: "2026-01-01T01:00:00.000Z".into(),
        }
    }

    #[test]
    fn round_trips_and_upsert_replaces() {
        let store = memory();
        assert!(store.list().unwrap().is_empty());
        store.upsert(&sample("a")).unwrap();
        store.upsert(&sample("b")).unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "a");
        let mut updated = sample("a");
        updated.display_name = Some("Ren".into());
        updated.expires_at = "2026-01-01T02:00:00.000Z".into();
        store.upsert(&updated).unwrap();
        let found = store.get("a").unwrap().unwrap();
        assert_eq!(found.display_name.as_deref(), Some("Ren"));
        assert_eq!(found.expires_at, "2026-01-01T02:00:00.000Z");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn staged_and_disconnected_rows_are_hidden_but_cleanup_is_recoverable() {
        let store = memory();
        store.stage(&sample("staged")).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(store.get("staged").unwrap().is_none());
        assert_eq!(store.cleanup_ids().unwrap(), vec!["staged"]);
        store.upsert(&sample("staged")).unwrap();
        assert!(store.cleanup_ids().unwrap().is_empty());
        assert_eq!(store.list().unwrap().len(), 1);
        store.mark_disconnected("staged").unwrap();
        assert!(store.get("staged").unwrap().is_none());
        assert_eq!(store.cleanup_ids().unwrap(), vec!["staged"]);
    }

    #[test]
    fn old_schema_migrates_without_resetting_existing_rows() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE connectors (id TEXT PRIMARY KEY, provider TEXT NOT NULL, scopes TEXT NOT NULL, display_name TEXT, connected_at TEXT NOT NULL, expires_at TEXT NOT NULL); INSERT INTO connectors VALUES ('old','google','[]',NULL,'start','end');").unwrap();
        let store = ConnectorStore::initialize(db).unwrap();
        assert_eq!(store.list().unwrap()[0].id, "old");
        assert!(store.cleanup_ids().unwrap().is_empty());
        store.mark_disconnected("old").unwrap();
        let reopened = ConnectorStore::initialize(store.db).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert_eq!(reopened.cleanup_ids().unwrap(), vec!["old"]);
    }

    #[test]
    fn database_activation_failure_keeps_staged_row_hidden() {
        let store = memory();
        store.stage(&sample("staged")).unwrap();
        store.db.execute_batch("CREATE TRIGGER fail_activation BEFORE UPDATE ON connectors WHEN NEW.disconnected=0 BEGIN SELECT RAISE(FAIL, 'fixture activation failure'); END;").unwrap();
        assert!(store.upsert(&sample("staged")).is_err());
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.cleanup_ids().unwrap(), vec!["staged"]);
    }

    #[test]
    fn delete_reports_missing_and_wipes() {
        let store = memory();
        assert!(!store.delete("missing").unwrap());
        store.upsert(&sample("a")).unwrap();
        assert!(store.delete("a").unwrap());
        assert!(store.get("a").unwrap().is_none());
        assert!(store.list().unwrap().is_empty());
    }
}
