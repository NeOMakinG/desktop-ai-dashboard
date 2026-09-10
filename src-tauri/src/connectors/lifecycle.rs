//! Host-owned, bounded attempt and refresh ownership. Never serialized with secrets.
use crate::types::{AppError, AppResult};
use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AttemptPhase {
    Pending,
    Exchanging,
    Connected,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptStatus {
    pub id: String,
    pub phase: AttemptPhase,
    pub expires_at: String,
    pub error: Option<AppError>,
}

struct Pending {
    id: String,
    state: Zeroizing<String>,
    port: u16,
    deadline: Instant,
    callback_active: bool,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct Lifecycle {
    pub revision: u64,
    pub attempt: Option<AttemptStatus>,
    pub(super) grants: super::grants::Grants,
    pending: Option<Pending>,
    refreshes: HashMap<String, String>,
}

pub fn timeout_error() -> AppError {
    AppError::new(
        "connector_timeout",
        "Sign-in did not complete in time. Try again.",
    )
}
pub fn stale_error() -> AppError {
    AppError::new(
        "connector_stale",
        "This connector action is no longer active.",
    )
}

impl Lifecycle {
    pub fn begin(
        &mut self,
        port: u16,
        state: String,
        timeout: Duration,
    ) -> AppResult<(String, CancellationToken, Instant)> {
        self.expire();
        if self.pending.is_some() {
            return Err(AppError::new(
                "connector_pending",
                "A Google sign-in is already in progress. Finish or cancel it first.",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let deadline = Instant::now() + timeout;
        self.pending = Some(Pending {
            id: id.clone(),
            state: Zeroizing::new(state),
            port,
            deadline,
            callback_active: true,
            cancel: cancel.clone(),
        });
        self.attempt = Some(AttemptStatus {
            id: id.clone(),
            phase: AttemptPhase::Pending,
            expires_at: super::oauth::expires_at_from_secs(chrono::Utc::now(), timeout.as_secs()),
            error: None,
        });
        self.revision += 1;
        Ok((id, cancel, deadline))
    }

    pub fn allows_callback(&self, url: &reqwest::Url) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        pending.callback_active
            && !pending.cancel.is_cancelled()
            && Instant::now() < pending.deadline
            && super::loopback::matches_callback_url(
                url,
                pending.port,
                super::CALLBACK_PATH,
                &pending.state,
            )
    }

    pub fn check(&mut self, id: &str) -> AppResult<()> {
        self.expire();
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.id == id && !p.cancel.is_cancelled())
        {
            Ok(())
        } else {
            Err(stale_error())
        }
    }

    pub fn exchanging(&mut self, id: &str) -> AppResult<()> {
        self.check(id)?;
        self.pending.as_mut().unwrap().callback_active = false;
        self.attempt.as_mut().unwrap().phase = AttemptPhase::Exchanging;
        self.revision += 1;
        Ok(())
    }

    pub fn finish(&mut self, id: &str, result: AppResult<()>) -> bool {
        self.expire();
        if !self.pending.as_ref().is_some_and(|p| p.id == id) {
            return false;
        }
        let phase = if result.is_ok() {
            AttemptPhase::Connected
        } else {
            AttemptPhase::Failed
        };
        self.terminal(phase, result.err());
        true
    }

    /// Used only while the mutation lock is held, after an unexpired check and
    /// successful DB publication. The commit linearizes at that last check;
    /// crossing the clock boundary during SQLite commit cannot relabel it failed.
    pub fn committed(&mut self, id: &str) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.terminal(AttemptPhase::Connected, None);
        }
    }

    pub fn cancel(&mut self, id: &str) -> AppResult<()> {
        self.check(id)?;
        self.terminal(AttemptPhase::Cancelled, None);
        Ok(())
    }

    pub fn expire(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.deadline)
        {
            self.terminal(AttemptPhase::Failed, Some(timeout_error()));
        }
    }

    fn terminal(&mut self, phase: AttemptPhase, error: Option<AppError>) {
        if let Some(pending) = self.pending.take() {
            pending.cancel.cancel();
        }
        if let Some(status) = &mut self.attempt {
            status.phase = phase;
            status.error = error;
        }
        self.revision += 1;
    }

    pub fn begin_refresh(&mut self, id: &str) -> AppResult<String> {
        if self.refreshes.contains_key(id) {
            return Err(AppError::new(
                "connector_pending",
                "This connection is already refreshing.",
            ));
        }
        let generation = uuid::Uuid::new_v4().to_string();
        self.refreshes.insert(id.to_owned(), generation.clone());
        Ok(generation)
    }

    pub fn check_refresh(&self, id: &str, generation: &str) -> AppResult<()> {
        if self
            .refreshes
            .get(id)
            .is_some_and(|current| current == generation)
        {
            Ok(())
        } else {
            Err(stale_error())
        }
    }

    pub fn finish_refresh(&mut self, id: &str, generation: &str) {
        if self.check_refresh(id, generation).is_ok() {
            self.refreshes.remove(id);
        }
    }

    pub fn disconnect(&mut self, id: &str) {
        self.grants.revoke_connection(id);
        self.refreshes.remove(id);
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn callback(port: u16, state: &str) -> reqwest::Url {
        reqwest::Url::parse(&format!(
            "http://127.0.0.1:{port}/oauth2/google/callback?state={state}&code=fixture"
        ))
        .unwrap()
    }
    #[test]
    fn single_attempt_cancel_and_stale_results_are_bounded() {
        let mut life = Lifecycle::default();
        let (first, cancel, _) = life
            .begin(32123, "one".into(), Duration::from_secs(10))
            .unwrap();
        assert!(life.allows_callback(&callback(32123, "one")));
        assert!(life
            .begin(32124, "two".into(), Duration::from_secs(10))
            .is_err());
        life.cancel(&first).unwrap();
        assert!(cancel.is_cancelled());
        assert!(!life.allows_callback(&callback(32123, "one")));
        let (second, _, _) = life
            .begin(32124, "two".into(), Duration::from_secs(10))
            .unwrap();
        assert!(!life.finish(&first, Ok(())));
        assert!(life.cancel(&first).is_err());
        assert_eq!(life.attempt.as_ref().unwrap().id, second);
        life.exchanging(&second).unwrap();
        assert!(!life.allows_callback(&callback(32124, "two")));
        assert!(life.finish(
            &second,
            Err(AppError::new(
                "connector_exchange",
                "Provider refused sign-in."
            ))
        ));
        assert_eq!(life.attempt.as_ref().unwrap().phase, AttemptPhase::Failed);
        assert_eq!(
            life.attempt.as_ref().unwrap().error.as_ref().unwrap().code,
            "connector_exchange"
        );
        assert!(!life.finish(&second, Ok(())));
    }
    #[test]
    fn accepted_commit_stays_connected_when_deadline_crosses_during_db_commit() {
        let mut life = Lifecycle::default();
        let (id, cancel, _) = life
            .begin(32123, "one".into(), Duration::from_secs(10))
            .unwrap();
        life.exchanging(&id).unwrap();
        life.check(&id).unwrap(); // commit_new's final unexpired check under mutation lock
        life.pending.as_mut().unwrap().deadline = Instant::now(); // SQLite commit crosses deadline
        life.committed(&id);
        let status = life.attempt.as_ref().unwrap();
        assert_eq!(status.phase, AttemptPhase::Connected);
        assert!(status.error.is_none());
        assert!(cancel.is_cancelled());
        assert!(!life.allows_callback(&callback(32123, "one")));
        assert!(!life.finish(&id, Err(timeout_error())));
        assert_eq!(
            life.attempt.as_ref().unwrap().phase,
            AttemptPhase::Connected
        );
    }
    #[test]
    fn timeout_releases_attempt_and_rejects_late_success() {
        let mut life = Lifecycle::default();
        let (id, cancel, _) = life.begin(32123, "one".into(), Duration::ZERO).unwrap();
        assert!(!life.allows_callback(&callback(32123, "one")));
        assert!(!life.finish(&id, Ok(())));
        assert!(cancel.is_cancelled());
        assert_eq!(
            life.attempt.as_ref().unwrap().error.as_ref().unwrap().code,
            "connector_timeout"
        );
        assert!(life
            .begin(32124, "two".into(), Duration::from_secs(10))
            .is_ok());
    }
    #[test]
    fn refresh_disconnect_reconnect_invalidates_old_generation() {
        let mut life = Lifecycle::default();
        let first = life.begin_refresh("connection").unwrap();
        assert!(life.begin_refresh("connection").is_err());
        life.disconnect("connection");
        let next = life.begin_refresh("connection").unwrap();
        assert!(life.check_refresh("connection", &first).is_err());
        life.finish_refresh("connection", &first);
        assert!(life.check_refresh("connection", &next).is_ok());
        life.finish_refresh("connection", &next);
        assert!(life.check_refresh("connection", &next).is_err());
    }
}
