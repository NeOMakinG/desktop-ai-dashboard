use crate::{types::*, validation};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    pub db: Connection,
}
#[derive(Clone)]
pub struct StoredProvider {
    pub config: ProviderConfig,
    pub generation: u64,
    pub credential: Option<String>,
}
fn encode<T: serde::Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| AppError::storage())
}
fn decode<T: serde::de::DeserializeOwned>(value: String) -> AppResult<T> {
    serde_json::from_str(&value).map_err(|_| AppError::storage())
}

impl Store {
    pub fn open(directory: &Path) -> AppResult<Self> {
        use std::fs;
        if !directory.is_absolute() || directory.parent().is_none() {
            return Err(AppError::storage());
        }
        if directory.exists()
            && fs::symlink_metadata(directory)
                .map_err(|_| AppError::storage())?
                .file_type()
                .is_symlink()
        {
            return Err(AppError::storage());
        }
        fs::create_dir_all(directory).map_err(|_| AppError::storage())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| AppError::storage())?;
            let path = directory.join("forma.sqlite3");
            if !path.exists() {
                match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                {
                    Ok(_) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(_) => return Err(AppError::storage()),
                }
            }
        }
        let path = directory.join("forma.sqlite3");
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(AppError::storage());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|_| AppError::storage())?;
        }
        Self::initialize(Connection::open(path)?)
    }
    pub fn initialize(mut db: Connection) -> AppResult<Self> {
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 5 {
            return Err(AppError::new("storage_version", "This data was created by an unsupported Forma version. Update the app; your data has not been reset."));
        }
        let integrity: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(AppError::storage());
        }
        if version == 0 {
            let tables: u32 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
            if tables != 0 {
                return Err(AppError::storage());
            }
            let tx = db.transaction()?;
            tx.execute_batch("CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
                CREATE TABLE provider (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL, generation INTEGER NOT NULL, credential TEXT);
                CREATE TABLE workspaces (id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, draft TEXT NOT NULL DEFAULT '', draft_version INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE request_ids (id TEXT PRIMARY KEY);
                CREATE TABLE messages (seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL, workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE, role TEXT NOT NULL CHECK(role IN ('user','assistant')), content TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('complete','pending','error','cancelled')), created_at TEXT NOT NULL, request_id TEXT NOT NULL REFERENCES request_ids(id), generation INTEGER NOT NULL);
                CREATE INDEX messages_workspace ON messages(workspace_id,seq);
                CREATE UNIQUE INDEX one_pending_per_workspace ON messages(workspace_id) WHERE status='pending';
                PRAGMA user_version=1;")?;
            tx.execute(
                "INSERT INTO preferences VALUES(1,?1)",
                [encode(&SettingsInput::default())?],
            )?;
            tx.execute(
                "INSERT INTO provider VALUES(1,?1,0,NULL)",
                [encode(&ProviderConfig::default())?],
            )?;
            tx.commit()?;
        }
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA max_page_count=65536;",
        )?;
        crate::runtime::storage::migrate(&mut db)?;
        let store = Self { db };
        // Direct completions cannot reconcile. Hermes admissions have durable request IDs:
        // preserve their pending rows for explicit status reconciliation, never redispatch.
        store.db.execute("UPDATE messages SET status='error',content='Interrupted when Forma closed. Send a new message to try again.' WHERE status='pending' AND request_id NOT IN (SELECT request_id FROM runtime_requests)", [])?;
        store.settings()?;
        Ok(store)
    }
    pub fn provider(&self) -> AppResult<StoredProvider> {
        let (value, generation, credential) = self.db.query_row(
            "SELECT value,generation,credential FROM provider WHERE id=1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(StoredProvider {
            config: decode(value)?,
            generation,
            credential,
        })
    }
    pub fn set_provider(&self, provider: &StoredProvider) -> AppResult<()> {
        self.db.execute(
            "UPDATE provider SET value=?1,generation=?2,credential=?3 WHERE id=1",
            params![
                encode(&provider.config)?,
                provider.generation,
                provider.credential
            ],
        )?;
        Ok(())
    }
    pub fn settings(&self) -> AppResult<AppSettings> {
        Ok(AppSettings {
            schema_version: 1,
            preferences: decode(self.db.query_row(
                "SELECT value FROM preferences WHERE id=1",
                [],
                |r| r.get(0),
            )?)?,
            provider: self.provider()?.config,
        })
    }
    pub fn save_settings(&self, input: SettingsInput) -> AppResult<AppSettings> {
        validation::single_line(&input.display_name, 80, true)?;
        if input.onboarding_step > 2 {
            return Err(AppError::invalid());
        }
        self.db.execute(
            "UPDATE preferences SET value=?1 WHERE id=1",
            [encode(&input)?],
        )?;
        self.settings()
    }
    pub fn summaries(&self) -> AppResult<Vec<WorkspaceSummary>> {
        let mut statement = self.db.prepare("SELECT w.id,w.title,w.created_at,w.updated_at,count(m.id) FROM workspaces w LEFT JOIN messages m ON m.workspace_id=w.id GROUP BY w.id ORDER BY w.updated_at DESC,w.id")?;
        let rows = statement.query_map([], |r| {
            Ok(WorkspaceSummary {
                id: r.get(0)?,
                title: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
                message_count: r.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
    pub fn workspace(&self, id: &str) -> AppResult<ChatWorkspace> {
        validation::id(id)?;
        let (title, created_at, updated_at, draft, draft_version) = self.db.query_row("SELECT title,created_at,updated_at,draft,draft_version FROM workspaces WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or_else(AppError::missing)?;
        let mut statement = self.db.prepare("SELECT id,role,content,status,created_at,request_id FROM messages WHERE workspace_id=?1 ORDER BY seq")?;
        let messages = statement
            .query_map([id], |r| {
                Ok(ChatMessage {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    status: r.get(3)?,
                    created_at: r.get(4)?,
                    request_id: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ChatWorkspace {
            summary: WorkspaceSummary {
                id: id.to_owned(),
                title,
                created_at,
                updated_at,
                message_count: messages.len() as u32,
            },
            draft,
            draft_version,
            messages,
        })
    }
    pub fn create_workspace(&self) -> AppResult<ChatWorkspace> {
        let count: u32 = self
            .db
            .query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))?;
        if count >= 200 {
            return Err(AppError::new(
                "workspace_limit",
                "Workspace limit reached. Delete an unused workspace first.",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = now();
        self.db.execute("INSERT INTO workspaces(id,title,created_at,updated_at) VALUES(?1,'New workspace',?2,?2)", params![id,timestamp])?;
        self.workspace(&id)
    }
    pub fn update_draft(&self, id: &str, draft: &str, version: u64) -> AppResult<ChatWorkspace> {
        validation::id(id)?;
        validation::text(draft, validation::MAX_TEXT, true)?;
        if version > validation::MAX_VERSION {
            return Err(AppError::invalid());
        }
        self.db.execute("UPDATE workspaces SET draft=?1,draft_version=?2,updated_at=?3 WHERE id=?4 AND draft_version<?2", params![draft,version,now(),id])?;
        self.workspace(id)
    }
    pub fn rename(&self, id: &str, title: &str) -> AppResult<ChatWorkspace> {
        validation::id(id)?;
        validation::single_line(title, 100, false)?;
        self.db.execute(
            "UPDATE workspaces SET title=?1,updated_at=?2 WHERE id=?3",
            params![title.trim(), now(), id],
        )?;
        self.workspace(id)
    }
    pub fn delete(&mut self, id: &str) -> AppResult<()> {
        validation::id(id)?;
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM runtime_requests WHERE workspace_id=?1", [id])?;
        tx.execute(
            "DELETE FROM runtime_legacy_requests WHERE workspace_id=?1",
            [id],
        )?;
        tx.execute(
            "DELETE FROM runtime_legacy_owners WHERE workspace_id=?1",
            [id],
        )?;
        let changed = tx.execute("DELETE FROM workspaces WHERE id=?1", [id])?;
        if changed == 0 {
            return Err(AppError::missing());
        }
        tx.commit()?;
        Ok(())
    }
    pub fn start(
        &mut self,
        workspace: &str,
        content: &str,
        request: &str,
        generation: u64,
    ) -> AppResult<()> {
        self.start_captured(workspace, content, request, generation, None)
    }
    pub fn start_captured(
        &mut self,
        workspace: &str,
        content: &str,
        request: &str,
        generation: u64,
        runtime: Option<&crate::runtime::storage::StoredRequest>,
    ) -> AppResult<()> {
        validation::id(request)?;
        validation::text(content, validation::MAX_TEXT, false)?;
        let current = self.workspace(workspace)?;
        if current.messages.iter().any(|m| m.status == "pending") {
            return Err(AppError::new(
                "busy",
                "This workspace already has a pending reply.",
            ));
        }
        if current.messages.len() >= 2000 || current.draft_version >= validation::MAX_VERSION {
            return Err(AppError::new(
                "history_limit",
                "This workspace is full. Start a new workspace to continue.",
            ));
        }
        let bytes = current
            .messages
            .iter()
            .filter(|m| m.status == "complete")
            .map(|m| m.content.len())
            .sum::<usize>()
            + content.len();
        if bytes > validation::MAX_CONTEXT {
            return Err(AppError::new("context_limit", "This chat exceeds the provider context limit. History is preserved; start a new workspace to continue."));
        }
        let tx = self.db.transaction()?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM request_ids WHERE id=?1)",
            [request],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::new(
                "duplicate_request",
                "This request was already used. Send again with a new request.",
            ));
        }
        tx.execute("INSERT INTO request_ids VALUES(?1)", [request])?;
        if let Some(runtime) = runtime {
            tx.execute(
                "INSERT INTO runtime_requests VALUES(?1,?2,?3)",
                params![
                    request,
                    workspace,
                    crate::runtime::storage::encode(runtime)?
                ],
            )?;
        }
        let timestamp = now();
        for (role, text, status) in [("user", content, "complete"), ("assistant", "", "pending")] {
            tx.execute("INSERT INTO messages(id,workspace_id,role,content,status,created_at,request_id,generation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![uuid::Uuid::new_v4().to_string(),workspace,role,text,status,timestamp,request,generation])?;
        }
        tx.execute("UPDATE workspaces SET draft='',draft_version=draft_version+1,updated_at=?1 WHERE id=?2", params![timestamp,workspace])?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish(
        &mut self,
        workspace: &str,
        request: &str,
        generation: u64,
        status: &str,
        text: &str,
    ) -> AppResult<()> {
        let tx = self.db.transaction()?;
        let changed = tx.execute("UPDATE messages SET status=?1,content=?2 WHERE workspace_id=?3 AND request_id=?4 AND generation=?5 AND status='pending' AND generation=(SELECT generation FROM provider WHERE id=1)", params![status,text,workspace,request,generation])?;
        if changed != 1 {
            return Err(AppError::stale());
        }
        tx.execute(
            "UPDATE workspaces SET updated_at=?1 WHERE id=?2",
            params![now(), workspace],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn cancel_pending(&self, workspace: &str, request: &str) -> AppResult<()> {
        self.db.execute("UPDATE messages SET status='cancelled',content='Reply cancelled.' WHERE workspace_id=?1 AND request_id=?2 AND status='pending'", params![workspace,request])?;
        Ok(())
    }
    pub fn invalidate_provider(&mut self, provider: &StoredProvider) -> AppResult<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE provider SET value=?1,generation=?2,credential=?3 WHERE id=1",
            params![
                encode(&provider.config)?,
                provider.generation,
                provider.credential
            ],
        )?;
        tx.execute("UPDATE messages SET status='cancelled',content='Reply cancelled because the AI connection changed.' WHERE status='pending' AND request_id NOT IN (SELECT request_id FROM runtime_requests)", [])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::initialize(Connection::open_in_memory().unwrap()).unwrap()
    }
    #[test]
    fn drafts_are_monotonic_and_delete_never_resurrects() {
        let mut store = store();
        let id = store.create_workspace().unwrap().summary.id;
        store.update_draft(&id, "new", 2).unwrap();
        assert_eq!(store.update_draft(&id, "old", 1).unwrap().draft, "new");
        assert_eq!(store.update_draft(&id, "same", 2).unwrap().draft, "new");
        store.delete(&id).unwrap();
        assert!(store.update_draft(&id, "late", 3).is_err());
        assert!(store.rename(&id, "late").is_err());
        assert!(store.summaries().unwrap().is_empty());
    }
    #[test]
    fn durable_two_stage_request_cancel_and_no_reuse() {
        let mut store = store();
        let id = store.create_workspace().unwrap().summary.id;
        let req = uuid::Uuid::new_v4().to_string();
        store.update_draft(&id, "draft", 1).unwrap();
        store.start(&id, "hello", &req, 0).unwrap();
        let workspace = store.workspace(&id).unwrap();
        assert_eq!(workspace.messages.len(), 2);
        assert_eq!(workspace.draft, "");
        assert_eq!(workspace.draft_version, 2);
        assert!(store
            .start(&id, "again", &uuid::Uuid::new_v4().to_string(), 0)
            .is_err());
        store.cancel_pending(&id, &req).unwrap();
        assert!(store.finish(&id, &req, 0, "complete", "late").is_err());
        assert!(store.start(&id, "again", &req, 0).is_err());
        store.delete(&id).unwrap();
        let another = store.create_workspace().unwrap().summary.id;
        assert!(store.start(&another, "again", &req, 0).is_err());
        assert!(store.finish(&id, &req, 0, "complete", "late").is_err());
    }
    #[test]
    fn completion_is_once_and_config_invalidates() {
        let mut store = store();
        let id = store.create_workspace().unwrap().summary.id;
        let req = uuid::Uuid::new_v4().to_string();
        store.start(&id, "hello", &req, 0).unwrap();
        store.finish(&id, &req, 0, "complete", "world").unwrap();
        assert!(store.finish(&id, &req, 0, "complete", "duplicate").is_err());
        let req2 = uuid::Uuid::new_v4().to_string();
        store.start(&id, "hello again", &req2, 0).unwrap();
        let mut provider = store.provider().unwrap();
        provider.generation = 1;
        store.invalidate_provider(&provider).unwrap();
        assert!(store.finish(&id, &req2, 0, "complete", "late").is_err());
        assert_eq!(
            store.workspace(&id).unwrap().messages[3].status,
            "cancelled"
        );
    }
    #[test]
    fn restart_preserves_history_and_marks_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path()).unwrap();
        let id = store.create_workspace().unwrap().summary.id;
        let req = uuid::Uuid::new_v4().to_string();
        store.start(&id, "remember me", &req, 0).unwrap();
        store.update_draft(&id, "next thought", 2).unwrap();
        drop(store);
        let store = Store::open(temp.path()).unwrap();
        let ws = store.workspace(&id).unwrap();
        assert_eq!(ws.draft, "next thought");
        assert_eq!(ws.messages[0].content, "remember me");
        assert_eq!(ws.messages[1].status, "error");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(temp.path().join("forma.sqlite3"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn unknown_versions_and_corrupt_storage_fail_without_reset() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            Store::initialize(db),
            Err(AppError {
                code: "storage_version",
                ..
            })
        ));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("forma.sqlite3");
        std::fs::write(&path, b"not a sqlite database").unwrap();
        assert!(Store::open(temp.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"not a sqlite database");
    }
    #[test]
    fn history_scope_and_context_limit_are_explicit() {
        let mut store = store();
        let id = store.create_workspace().unwrap().summary.id;
        let other = store.create_workspace().unwrap().summary.id;
        for _ in 0..4 {
            let req = uuid::Uuid::new_v4().to_string();
            store
                .start(&id, &"x".repeat(validation::MAX_TEXT), &req, 0)
                .unwrap();
            store.cancel_pending(&id, &req).unwrap();
        }
        assert!(store
            .start(&id, "too much", &uuid::Uuid::new_v4().to_string(), 0)
            .is_err());
        assert!(store.workspace(&other).unwrap().messages.is_empty());
    }
}
