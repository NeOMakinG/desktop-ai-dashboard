use super::{config::StoredRuntime, dto::*};
use crate::{store::Store, types::*, validation};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredRequest {
    pub input: RunInput,
    pub config_generation: u64,
    pub workspace_generation: u64,
    pub dispatched: bool,
    pub cancel_requested: bool,
    pub progress: RuntimeProgress,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOwner {
    pub endpoint: String,
    pub library_id: String,
    pub device_id: String,
}
pub fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| AppError::storage())
}
pub fn decode<T: serde::de::DeserializeOwned>(value: String) -> AppResult<T> {
    serde_json::from_str(&value).map_err(|_| AppError::storage())
}

pub fn migrate(db: &mut Connection) -> AppResult<()> {
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 5 {
        return Ok(());
    }
    if version == 4 {
        let tx = db.transaction()?;
        // Archive remote ownership/intents, never replay them against the new local service.
        tx.execute_batch("CREATE TABLE runtime_legacy_owners AS SELECT * FROM runtime_owners;
            CREATE TABLE runtime_legacy_requests AS SELECT * FROM runtime_requests;
            UPDATE messages SET status='error',content=CASE WHEN content='' THEN 'Interrupted during migration to app-owned Hermes. Previous remote work was not cancelled or replayed.' ELSE content END WHERE status='pending';
            DELETE FROM runtime_requests; DELETE FROM runtime_sessions; DELETE FROM runtime_owners;
            UPDATE runtime_workspaces SET value=json_set(value,'$.route','hermes','$.remoteInitialized',json('false'),'$.generation',coalesce(json_extract(value,'$.generation'),0)+1,'$.modelId',CASE WHEN coalesce(json_extract(value,'$.modelId'),'')='' THEN coalesce((SELECT json_extract(value,'$.model') FROM provider WHERE id=1),'') ELSE json_extract(value,'$.modelId') END),endpoint_generation='';
            PRAGMA user_version=5;")?;
        tx.execute(
            "UPDATE runtime_config SET value=?1,capabilities=NULL,models='[]' WHERE id=1",
            [encode(&StoredRuntime::default())?],
        )?;
        tx.commit()?;
        return Ok(());
    }
    if version == 3 {
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE runtime_owners (workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE, endpoint TEXT NOT NULL, library_id TEXT NOT NULL, device_id TEXT NOT NULL, PRIMARY KEY(workspace_id,endpoint,library_id,device_id));
            INSERT OR IGNORE INTO runtime_owners SELECT * FROM runtime_sessions;
            INSERT OR IGNORE INTO runtime_owners
                SELECT w.workspace_id,w.endpoint_generation,
                    CASE WHEN w.endpoint_generation=json_extract(c.value,'$.config.endpoint') THEN coalesce(json_extract(c.capabilities,'$.libraryId'),'') ELSE '' END,
                    CASE WHEN w.endpoint_generation=json_extract(c.value,'$.config.endpoint') THEN coalesce(json_extract(c.capabilities,'$.deviceId'),'') ELSE '' END
                FROM runtime_workspaces w CROSS JOIN runtime_config c
                WHERE w.endpoint_generation NOT IN ('','0') AND NOT EXISTS (SELECT 1 FROM runtime_owners o WHERE o.workspace_id=w.workspace_id AND o.endpoint=w.endpoint_generation);
            PRAGMA user_version=4;")?;
        tx.commit()?;
        return migrate(db);
    }
    if version == 2 {
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE runtime_sessions (workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE, endpoint TEXT NOT NULL, library_id TEXT NOT NULL, device_id TEXT NOT NULL, PRIMARY KEY(workspace_id,endpoint,library_id,device_id)); PRAGMA user_version=3;")?;
        tx.commit()?;
        return migrate(db);
    }
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE runtime_config (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL, capabilities TEXT, models TEXT NOT NULL DEFAULT '[]');
        CREATE TABLE runtime_workspaces (workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE, value TEXT NOT NULL, endpoint_generation TEXT NOT NULL);
        CREATE TABLE runtime_requests (request_id TEXT PRIMARY KEY REFERENCES request_ids(id), workspace_id TEXT NOT NULL, value TEXT NOT NULL);
        CREATE INDEX runtime_requests_workspace ON runtime_requests(workspace_id);
        PRAGMA user_version=2;")?;
    tx.execute(
        "INSERT INTO runtime_config(id,value) VALUES(1,?1)",
        [encode(&StoredRuntime::default())?],
    )?;
    // The subsequent managed migration preserves content and normalizes legacy routing.
    let ids: Vec<String> = {
        let mut s = tx.prepare("SELECT id FROM workspaces")?;
        let rows = s
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for workspace_id in ids {
        let value = RuntimeWorkspace {
            workspace_id: workspace_id.clone(),
            route: Route::Hermes,
            model_id: String::new(),
            generation: 0,
            remote_initialized: false,
        };
        tx.execute(
            "INSERT INTO runtime_workspaces VALUES(?1,?2,0)",
            params![workspace_id, encode(&value)?],
        )?;
    }
    tx.commit()?;
    migrate(db)
}
impl Store {
    pub fn runtime_config(&self) -> AppResult<StoredRuntime> {
        decode(
            self.db
                .query_row("SELECT value FROM runtime_config WHERE id=1", [], |r| {
                    r.get(0)
                })?,
        )
    }
    pub fn runtime_status(&self) -> AppResult<RuntimeStatus> {
        let saved = self.runtime_config()?;
        let config = saved.config;
        let (capabilities, models): (Option<String>, String) = self.db.query_row(
            "SELECT capabilities,models FROM runtime_config WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let legacy: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM runtime_legacy_owners) OR EXISTS(SELECT 1 FROM runtime_legacy_requests)", [], |r| r.get(0))?;
        Ok(RuntimeStatus {
            state: saved.state,
            message: saved.message.or_else(|| legacy.then(|| "Previous remote work was archived locally. It was not cancelled or replayed by managed Hermes.".into())),
            generation: config.generation,
            verified: config.verified,
            config,
            capabilities: capabilities.map(decode).transpose()?,
            models: decode(models)?,
        })
    }
    pub fn runtime_save_config(&mut self, value: &StoredRuntime) -> AppResult<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE runtime_config SET value=?1,models='[]' WHERE id=1",
            [encode(value)?],
        )?;
        tx.execute("UPDATE messages SET status='cancelled',content='Hermes reply fenced because the runtime configuration changed.' WHERE status='pending' AND request_id IN (SELECT request_id FROM runtime_requests)", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn runtime_checked(
        &self,
        value: &StoredRuntime,
        capabilities: &Capabilities,
        models: &[RuntimeModel],
    ) -> AppResult<()> {
        self.db.execute(
            "UPDATE runtime_config SET value=?1,capabilities=?2,models=?3 WHERE id=1",
            params![encode(value)?, encode(capabilities)?, encode(&models)?],
        )?;
        Ok(())
    }
    pub fn runtime_workspace(&self, workspace: &str) -> AppResult<RuntimeWorkspace> {
        self.workspace(workspace)?;
        let config = self.runtime_config()?.config;
        let row: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT value,endpoint_generation FROM runtime_workspaces WHERE workspace_id=?1",
                [workspace],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((raw, _)) = row {
            let mut result: RuntimeWorkspace = decode(raw)?;
            result.route = Route::Hermes;
            result.remote_initialized = match self.runtime_status()?.capabilities {
                Some(caps) => self.db.query_row("SELECT EXISTS(SELECT 1 FROM runtime_sessions WHERE workspace_id=?1 AND endpoint=?2 AND library_id=?3 AND device_id=?4)", params![workspace,config.endpoint,caps.library_id,caps.device_id], |r| r.get(0))?,
                None => false,
            };
            return Ok(result);
        }
        let value = RuntimeWorkspace {
            workspace_id: workspace.to_owned(),
            route: Route::Hermes,
            model_id: self.provider()?.config.model,
            generation: 0,
            remote_initialized: false,
        };
        self.runtime_save_workspace(&value)?;
        Ok(value)
    }
    pub fn runtime_save_workspace(&self, workspace: &RuntimeWorkspace) -> AppResult<()> {
        self.workspace(&workspace.workspace_id)?;
        let status = self.runtime_status()?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute("INSERT INTO runtime_workspaces VALUES(?1,?2,?3) ON CONFLICT(workspace_id) DO UPDATE SET value=excluded.value,endpoint_generation=excluded.endpoint_generation", params![workspace.workspace_id,encode(workspace)?,status.config.endpoint])?;
        if workspace.remote_initialized {
            let caps = status.capabilities.ok_or_else(AppError::stale)?;
            tx.execute(
                "INSERT OR IGNORE INTO runtime_owners VALUES(?1,?2,?3,?4)",
                params![
                    workspace.workspace_id,
                    status.config.endpoint,
                    caps.library_id,
                    caps.device_id
                ],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO runtime_sessions VALUES(?1,?2,?3,?4)",
                params![
                    workspace.workspace_id,
                    status.config.endpoint,
                    caps.library_id,
                    caps.device_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    // An intent is ownership even if the mutative HTTP response never arrives.
    // This is deliberately independent from the server's human-history initialization.
    pub fn runtime_own_workspace(&self, workspace: &str, generation: u64) -> AppResult<()> {
        self.workspace(workspace)?;
        let status = self.runtime_status()?;
        let caps = super::require_binding(&status, false)?;
        if status.config.generation != generation {
            return Err(AppError::stale());
        }
        self.db.execute(
            "INSERT OR IGNORE INTO runtime_owners VALUES(?1,?2,?3,?4)",
            params![
                workspace,
                status.config.endpoint,
                caps.library_id,
                caps.device_id
            ],
        )?;
        Ok(())
    }
    pub fn runtime_owners(&self, workspace: &str) -> AppResult<Vec<RemoteOwner>> {
        self.workspace(workspace)?;
        let mut statement = self.db.prepare(
            "SELECT endpoint,library_id,device_id FROM runtime_owners WHERE workspace_id=?1",
        )?;
        let rows = statement
            .query_map([workspace], |r| {
                Ok(RemoteOwner {
                    endpoint: r.get(0)?,
                    library_id: r.get(1)?,
                    device_id: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn runtime_deletion_owner(&self, workspace: &str) -> AppResult<Option<RemoteOwner>> {
        let owners = self.runtime_owners(workspace)?;
        if owners.is_empty() {
            return Ok(None);
        }
        let status = self.runtime_status()?;
        let caps = super::require_binding(&status, false)?;
        let current = RemoteOwner {
            endpoint: status.config.endpoint.clone(),
            library_id: caps.library_id.clone(),
            device_id: caps.device_id.clone(),
        };
        if owners.contains(&current) {
            return Ok(Some(current));
        }
        Err(AppError::new("runtime_ownership_unresolved", "This workspace may still own remote schedules at a previous or unknown runtime binding. Reconnect and verify the original runtime/library/device before deleting; local data was kept."))
    }
    pub fn runtime_acknowledge_deletion(
        &self,
        workspace: &str,
        owner: &RemoteOwner,
        generation: u64,
    ) -> AppResult<()> {
        if self.runtime_config()?.config.generation != generation
            || self.runtime_deletion_owner(workspace)?.as_ref() != Some(owner)
        {
            return Err(AppError::stale());
        }
        self.db.execute("DELETE FROM runtime_owners WHERE workspace_id=?1 AND endpoint=?2 AND library_id=?3 AND device_id=?4",
            params![workspace,owner.endpoint,owner.library_id,owner.device_id])?;
        Ok(())
    }
    pub fn runtime_request(&self, request: &str) -> AppResult<Option<StoredRequest>> {
        validation::id(request)?;
        self.db
            .query_row(
                "SELECT value FROM runtime_requests WHERE request_id=?1",
                [request],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(decode)
            .transpose()
    }
    pub fn runtime_save_request(&self, request: &StoredRequest) -> AppResult<()> {
        let changed = self.db.execute(
            "UPDATE runtime_requests SET value=?1 WHERE request_id=?2 AND workspace_id=?3",
            params![
                encode(request)?,
                request.progress.request_id,
                request.input.workspace_id
            ],
        )?;
        if changed != 1 {
            return Err(AppError::stale());
        }
        Ok(())
    }
    pub fn runtime_current(&self, request: &StoredRequest) -> AppResult<()> {
        if request.cancel_requested
            || self
                .runtime_request(&request.progress.request_id)?
                .ok_or_else(AppError::stale)?
                .cancel_requested
        {
            return Err(AppError::stale());
        }
        self.runtime_pending(request)
    }
    // Cancellation reconciliation uses the same durable identity fences but must
    // accept an already persisted cancellation intent after a process restart.
    pub fn runtime_pending(&self, request: &StoredRequest) -> AppResult<()> {
        let config = self.runtime_config()?.config;
        let workspace = self.runtime_workspace(&request.input.workspace_id)?;
        if config.generation != request.config_generation
            || workspace.generation != request.workspace_generation
            || workspace.route != Route::Hermes
            || !config.enabled
        {
            return Err(AppError::stale());
        }
        let pending: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE request_id=?1 AND workspace_id=?2 AND status='pending')", params![request.progress.request_id,request.input.workspace_id], |r| r.get(0))?;
        if !pending {
            return Err(AppError::stale());
        }
        Ok(())
    }
    pub fn runtime_finish_cancel(&self, request: &StoredRequest) -> AppResult<()> {
        self.runtime_pending(request)?;
        if !request.cancel_requested
            || request
                .progress
                .run
                .as_ref()
                .is_some_and(|run| !run.state.terminal())
            || (request.progress.run.is_none() && request.progress.state != "cancelled")
        {
            return Err(AppError::stale());
        }
        let tx = self.db.unchecked_transaction()?;
        tx.execute("UPDATE messages SET status='cancelled',content='Hermes stopped after reconciliation. Remote actions are not rolled back.' WHERE workspace_id=?1 AND request_id=?2 AND status='pending'", params![request.input.workspace_id,request.progress.request_id])?;
        tx.execute(
            "UPDATE runtime_requests SET value=?1 WHERE request_id=?2",
            params![encode(request)?, request.progress.request_id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn runtime_finish(
        &mut self,
        request: &StoredRequest,
        status: &str,
        text: &str,
    ) -> AppResult<()> {
        self.runtime_current(request)?;
        validation::text(text, 100_000, true)?;
        let tx = self.db.transaction()?;
        let changed = tx.execute("UPDATE messages SET status=?1,content=?2 WHERE workspace_id=?3 AND request_id=?4 AND generation=?5 AND status='pending'", params![status,text,request.input.workspace_id,request.progress.request_id,request.config_generation])?;
        if changed != 1 {
            return Err(AppError::stale());
        }
        tx.execute(
            "UPDATE runtime_requests SET value=?1 WHERE request_id=?2",
            params![encode(request)?, request.progress.request_id],
        )?;
        tx.execute(
            "UPDATE workspaces SET updated_at=?1 WHERE id=?2",
            params![now(), request.input.workspace_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}
