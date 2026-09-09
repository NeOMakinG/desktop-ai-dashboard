use crate::{
    provider::{self, Credentials},
    store::{Store, StoredProvider},
    types::*,
    validation,
};
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

struct PendingRequest {
    workspace: String,
    generation: u64,
    token: CancellationToken,
    started: bool,
}
pub struct Core {
    pub store: Store,
    credentials: Box<dyn Credentials>,
    requests: HashMap<String, PendingRequest>,
    check: Option<CancellationToken>,
    discovery: Option<CancellationToken>,
}
pub struct NativeState {
    inner: Mutex<AppResult<Core>>,
    pub client: AppResult<reqwest::Client>,
}
pub struct CoreGuard<'a>(MutexGuard<'a, AppResult<Core>>);
impl std::ops::Deref for CoreGuard<'_> {
    type Target = Core;
    fn deref(&self) -> &Core {
        self.0.as_ref().expect("checked core state")
    }
}
impl std::ops::DerefMut for CoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Core {
        self.0.as_mut().expect("checked core state")
    }
}
impl NativeState {
    pub fn new(core: AppResult<Core>) -> Self {
        Self {
            inner: Mutex::new(core),
            client: provider::client(),
        }
    }
    pub fn lock(&self) -> AppResult<CoreGuard<'_>> {
        let guard = self.inner.lock().map_err(|_| AppError::storage())?;
        if let Err(error) = guard.as_ref() {
            return Err(error.clone());
        }
        Ok(CoreGuard(guard))
    }
}
impl Core {
    pub fn new(store: Store, credentials: Box<dyn Credentials>) -> Self {
        Self {
            store,
            credentials,
            requests: HashMap::new(),
            check: None,
            discovery: None,
        }
    }
    fn key(&self, provider: &StoredProvider) -> AppResult<Option<Zeroizing<String>>> {
        provider
            .credential
            .as_deref()
            .map(|reference| {
                if !provider::credential_matches(&provider.config.base_url, reference) {
                    return Err(AppError::new("credential_store", "The saved key does not belong to this endpoint. Reconnect your AI provider."));
                }
                self.credentials.read(reference)
            })
            .transpose()
    }
    pub fn configure(&mut self, mut input: ProviderInput) -> AppResult<ProviderConfig> {
        let secret = input.api_key.take().map(Zeroizing::new);
        validation::single_line(&input.label, 80, true)?;
        validation::model(&input.model, true)?;
        let endpoint = validation::endpoint(&input.base_url)?;
        if (input.clear_key && secret.is_some())
            || (endpoint.is_empty() && (!input.model.is_empty() || secret.is_some()))
        {
            return Err(AppError::invalid());
        }
        if let Some(secret) = secret.as_ref() {
            validation::api_key(secret)?;
            if [&input.label, &endpoint, &input.model]
                .iter()
                .any(|v| v.contains(secret.as_str()))
            {
                return Err(AppError::invalid());
            }
        }
        let old = self.store.provider()?;
        let mut credential = if endpoint == old.config.base_url && !input.clear_key {
            old.credential.clone()
        } else {
            None
        };
        let mut fresh = None;
        if let Some(secret) = secret.as_ref() {
            let reference = provider::credential_reference(&endpoint);
            self.credentials.write(&reference, secret)?;
            match self.credentials.read(&reference) {
                Ok(stored) if stored.as_str() == secret.as_str() => (),
                _ => {
                    let _ = self.credentials.remove(&reference);
                    return Err(AppError::new("credential_store", "The OS credential store could not confirm the new key. Configuration was not changed."));
                }
            }
            credential = Some(reference.clone());
            fresh = Some(reference);
        }
        let generation = old
            .generation
            .checked_add(1)
            .filter(|g| *g <= validation::MAX_VERSION)
            .ok_or_else(AppError::storage)?;
        let updated = StoredProvider {
            config: ProviderConfig {
                label: input.label.trim().to_owned(),
                base_url: endpoint,
                model: input.model,
                has_key: credential.is_some(),
                verified: false,
                last_checked_at: None,
            },
            generation,
            credential,
        };
        if let Err(error) = self.store.invalidate_provider(&updated) {
            if let Some(reference) = fresh {
                let _ = self.credentials.remove(&reference);
            }
            return Err(error);
        }
        for (_, pending) in self.requests.drain() {
            pending.token.cancel();
        }
        if let Some(check) = self.check.take() {
            check.cancel();
        }
        if let Some(discovery) = self.discovery.take() {
            discovery.cancel();
        }
        if old.credential != updated.credential {
            if let Some(reference) = old.credential {
                self.credentials.remove(&reference)?;
            }
        }
        Ok(updated.config)
    }
    fn begin_discovery(&mut self) -> AppResult<Discovery> {
        if self.discovery.is_some() {
            return Err(AppError::new("busy", "A model refresh is already running."));
        }
        let provider = self.store.provider()?;
        if provider.config.base_url.is_empty() {
            return Err(AppError::new(
                "provider_not_ready",
                "Set up an AI connection before refreshing models.",
            ));
        }
        let key = self.key(&provider)?;
        let token = CancellationToken::new();
        self.discovery = Some(token.clone());
        Ok(Discovery {
            base: provider.config.base_url,
            generation: provider.generation,
            key,
            token,
        })
    }
    fn finish_discovery(
        &mut self,
        discovery: &Discovery,
        result: AppResult<Vec<String>>,
    ) -> AppResult<ModelList> {
        if discovery.token.is_cancelled()
            || self.store.provider()?.generation != discovery.generation
        {
            return Err(AppError::stale());
        }
        self.discovery = None;
        Ok(ModelList { models: result? })
    }
    pub fn start(
        &mut self,
        workspace: &str,
        content: &str,
        request: &str,
    ) -> AppResult<ChatWorkspace> {
        let provider = self.store.provider()?;
        if !provider.config.verified
            || provider.config.base_url.is_empty()
            || provider.config.model.is_empty()
        {
            return Err(AppError::new(
                "provider_not_ready",
                "Connect and check your AI provider in Settings before sending.",
            ));
        }
        if self.requests.len() >= 8 {
            return Err(AppError::new(
                "busy",
                "Too many replies are pending. Finish or cancel one first.",
            ));
        }
        self.key(&provider)?;
        self.store
            .start(workspace, content, request, provider.generation)?;
        self.requests.insert(
            request.to_owned(),
            PendingRequest {
                workspace: workspace.to_owned(),
                generation: provider.generation,
                token: CancellationToken::new(),
                started: false,
            },
        );
        self.store.workspace(workspace)
    }
    pub fn cancel(&mut self, workspace: &str, request: &str) -> AppResult<ChatWorkspace> {
        validation::id(workspace)?;
        validation::id(request)?;
        let current = self.store.workspace(workspace)?;
        if !current
            .messages
            .iter()
            .any(|m| m.request_id.as_deref() == Some(request))
        {
            return Err(AppError::missing());
        }
        self.store.cancel_pending(workspace, request)?;
        if let Some(pending) = self.requests.get(request) {
            if pending.workspace != workspace {
                return Err(AppError::missing());
            }
        }
        if let Some(pending) = self.requests.remove(request) {
            pending.token.cancel();
        }
        self.store.workspace(workspace)
    }
    pub fn delete(&mut self, workspace: &str) -> AppResult<()> {
        self.store.delete(workspace)?;
        self.requests.retain(|_, pending| {
            if pending.workspace == workspace {
                pending.token.cancel();
                false
            } else {
                true
            }
        });
        Ok(())
    }
    fn begin_complete(&mut self, workspace: &str, request: &str) -> AppResult<Completion> {
        validation::id(workspace)?;
        validation::id(request)?;
        let pending = self.requests.get(request).ok_or_else(AppError::missing)?;
        if pending.workspace != workspace || pending.started {
            return Err(AppError::stale());
        }
        let generation = pending.generation;
        let provider = self.store.provider()?;
        if generation != provider.generation {
            return Err(AppError::stale());
        }
        let key = self.key(&provider)?;
        let body = provider::chat_body(&provider.config.model, &self.store.workspace(workspace)?)?;
        let pending = self
            .requests
            .get_mut(request)
            .ok_or_else(AppError::missing)?;
        pending.started = true;
        Ok(Completion {
            base: provider.config.base_url,
            generation,
            key,
            body,
            token: pending.token.clone(),
        })
    }
    fn apply_complete(
        &mut self,
        workspace: &str,
        request: &str,
        generation: u64,
        result: AppResult<String>,
    ) -> AppResult<ChatWorkspace> {
        let pending = self.requests.get(request).ok_or_else(AppError::stale)?;
        if pending.workspace != workspace
            || pending.generation != generation
            || pending.token.is_cancelled()
        {
            return Err(AppError::stale());
        }
        let (status, text) = match &result {
            Ok(text) => ("complete", text.as_str()),
            Err(error) => ("error", error.message),
        };
        self.store
            .finish(workspace, request, generation, status, text)?;
        self.requests.remove(request);
        // Provider errors become durable visible assistant errors, not a lost history snapshot.
        self.store.workspace(workspace)
    }
}
struct Completion {
    base: String,
    generation: u64,
    key: Option<Zeroizing<String>>,
    body: serde_json::Value,
    token: CancellationToken,
}
pub async fn complete(
    state: &NativeState,
    workspace: String,
    request: String,
) -> AppResult<ChatWorkspace> {
    let client = state.client.as_ref().map_err(Clone::clone)?;
    let completion = {
        let mut core = state.lock()?;
        match core.begin_complete(&workspace, &request) {
            Ok(completion) => completion,
            Err(error) => {
                // A credential-store failure after start must not strand a pending reply.
                if error.code == "credential_store" || error.code == "context_limit" {
                    if let Some(pending) = core.requests.remove(&request) {
                        pending.token.cancel();
                        core.store.finish(
                            &workspace,
                            &request,
                            pending.generation,
                            "error",
                            error.message,
                        )?;
                    }
                }
                return Err(error);
            }
        }
    };
    let result = tokio::select! {
        biased;
        _ = completion.token.cancelled() => return Err(AppError::stale()),
        result = provider::complete(client,&completion.base,completion.key.as_deref().map(String::as_str),completion.body) => result,
    };
    state
        .lock()?
        .apply_complete(&workspace, &request, completion.generation, result)
}
struct Discovery {
    base: String,
    generation: u64,
    key: Option<Zeroizing<String>>,
    token: CancellationToken,
}
pub async fn list_models(state: &NativeState) -> AppResult<ModelList> {
    let client = state.client.as_ref().map_err(Clone::clone)?;
    let discovery = state.lock()?.begin_discovery()?;
    let result = tokio::select! {
        biased;
        _ = discovery.token.cancelled() => return Err(AppError::stale()),
        result = provider::check(client, &discovery.base, discovery.key.as_deref().map(String::as_str)) => result,
    };
    state.lock()?.finish_discovery(&discovery, result)
}
pub async fn check(state: &NativeState) -> AppResult<ProviderCheck> {
    let client = state.client.as_ref().map_err(Clone::clone)?;
    let (mut snapshot, key, token) = {
        let mut core = state.lock()?;
        if core.check.is_some() {
            return Err(AppError::new(
                "busy",
                "An AI connection check is already running.",
            ));
        }
        let mut provider = core.store.provider()?;
        if provider.config.base_url.is_empty() {
            return Err(AppError::new(
                "provider_not_ready",
                "Enter your provider endpoint before checking the connection.",
            ));
        }
        let key = core.key(&provider)?;
        provider.config.verified = false;
        provider.config.last_checked_at = None;
        core.store.set_provider(&provider)?;
        let token = CancellationToken::new();
        core.check = Some(token.clone());
        (provider, key, token)
    };
    let result = tokio::select! {
        biased;
        _ = token.cancelled() => return Err(AppError::stale()),
        result = provider::check(client,&snapshot.config.base_url,key.as_deref().map(String::as_str)) => result,
    };
    let mut core = state.lock()?;
    if token.is_cancelled() || core.store.provider()?.generation != snapshot.generation {
        return Err(AppError::stale());
    }
    core.check = None;
    let models = result?;
    snapshot.config.model = provider::selected_model(&snapshot.config.model, &models)?.to_owned();
    snapshot.config.verified = true;
    snapshot.config.last_checked_at = Some(now());
    core.store.set_provider(&snapshot)?;
    Ok(ProviderCheck {
        provider: snapshot.config,
        models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[derive(Default)]
    struct TestCredentials(Mutex<HashMap<String, String>>);
    impl Credentials for TestCredentials {
        fn read(&self, reference: &str) -> AppResult<Zeroizing<String>> {
            self.0
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .map(Zeroizing::new)
                .ok_or_else(AppError::missing)
        }
        fn write(&self, reference: &str, value: &str) -> AppResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(reference.into(), value.into());
            Ok(())
        }
        fn remove(&self, reference: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(reference);
            Ok(())
        }
    }
    include!("../tests/support/external_fixture.rs");

    fn core() -> Core {
        let core = Core::new(
            Store::initialize(rusqlite::Connection::open_in_memory().unwrap()).unwrap(),
            Box::<TestCredentials>::default(),
        );
        core.store
            .set_provider(&StoredProvider {
                config: ProviderConfig {
                    base_url: "http://localhost:9220/v1".into(),
                    model: "test-model".into(),
                    verified: true,
                    ..Default::default()
                },
                generation: 0,
                credential: None,
            })
            .unwrap();
        core
    }
    #[test]
    fn in_flight_cancellation_and_deletion_reject_late_results() {
        let mut core = core();
        let ws = core.store.create_workspace().unwrap().summary.id;
        let req = uuid::Uuid::new_v4().to_string();
        core.start(&ws, "hello", &req).unwrap();
        let completion = core.begin_complete(&ws, &req).unwrap();
        assert!(core.begin_complete(&ws, &req).is_err());
        core.cancel(&ws, &req).unwrap();
        assert!(completion.token.is_cancelled());
        assert!(core
            .apply_complete(&ws, &req, 0, Ok("late".into()))
            .is_err());
        let req = uuid::Uuid::new_v4().to_string();
        core.start(&ws, "hello", &req).unwrap();
        let completion = core.begin_complete(&ws, &req).unwrap();
        core.delete(&ws).unwrap();
        assert!(completion.token.is_cancelled());
        assert!(core
            .apply_complete(&ws, &req, 0, Ok("late".into()))
            .is_err());
        assert!(core.store.summaries().unwrap().is_empty());
    }
    #[test]
    fn discovery_is_independent_read_only_bounded_and_stale_guarded() {
        let mut core = core();
        let before = serde_json::to_value(core.store.settings().unwrap()).unwrap();
        let discovery = core.begin_discovery().unwrap();
        assert!(core.begin_discovery().is_err());
        let result = core
            .finish_discovery(&discovery, Ok(vec!["claude-opus-5".into()]))
            .unwrap();
        assert_eq!(result.models, vec!["claude-opus-5"]);
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({"models":["claude-opus-5"]})
        );
        assert_eq!(
            serde_json::to_value(core.store.settings().unwrap()).unwrap(),
            before
        );
        let failed = core.begin_discovery().unwrap();
        assert!(core
            .finish_discovery(&failed, Err(AppError::new("network", "Test failure")))
            .is_err());
        assert_eq!(
            serde_json::to_value(core.store.settings().unwrap()).unwrap(),
            before
        );
        let stale = core.begin_discovery().unwrap();
        core.configure(ProviderInput {
            label: "Test".into(),
            base_url: "http://localhost:9221/v1".into(),
            model: "unavailable".into(),
            api_key: None,
            clear_key: false,
        })
        .unwrap();
        let current = core.begin_discovery().unwrap();
        assert!(stale.token.is_cancelled());
        assert!(core
            .finish_discovery(&stale, Ok(vec!["old".into()]))
            .is_err());
        assert!(core.discovery.is_some());
        assert_eq!(
            core.finish_discovery(&current, Ok(vec!["new".into()]))
                .unwrap()
                .models,
            vec!["new"]
        );
        assert!(!core.store.provider().unwrap().config.verified);
        assert_eq!(core.store.provider().unwrap().config.model, "unavailable");
    }
    #[test]
    fn model_only_configuration_preserves_endpoint_bound_key() {
        let mut core = core();
        let input = |model: &str, key: Option<&str>| ProviderInput {
            label: "Test".into(),
            base_url: "http://localhost:9220/v1".into(),
            model: model.into(),
            api_key: key.map(str::to_owned),
            clear_key: false,
        };
        core.configure(input("test-model", Some("synthetic-key")))
            .unwrap();
        let before = core.store.provider().unwrap();
        let configured = core.configure(input("claude-opus-5", None)).unwrap();
        let after = core.store.provider().unwrap();
        assert!(configured.has_key);
        assert!(!configured.verified);
        assert_eq!(after.credential, before.credential);
        assert_eq!(core.key(&after).unwrap().unwrap().as_str(), "synthetic-key");
        assert_eq!(after.config.model, "claude-opus-5");
        assert!(after.generation > before.generation);
    }
    #[test]
    fn endpoint_changes_do_not_reuse_keys_and_cancel_checks() {
        let mut core = core();
        let input = |base: &str, key: Option<&str>| ProviderInput {
            label: "Local".into(),
            base_url: base.into(),
            model: "test-model".into(),
            api_key: key.map(str::to_owned),
            clear_key: false,
        };
        let result = core
            .configure(input("http://localhost:9220/v1", Some("fake-key")))
            .unwrap();
        assert!(result.has_key);
        assert!(!result.verified);
        assert!(!serde_json::to_string(&core.store.settings().unwrap())
            .unwrap()
            .contains("fake-key"));
        let check = CancellationToken::new();
        core.check = Some(check.clone());
        let result = core
            .configure(input("http://localhost:9221/v1", None))
            .unwrap();
        assert!(!result.has_key);
        assert!(check.is_cancelled());
        assert!(!result.verified);
    }
    #[test]
    fn config_edit_cancels_request_and_fixed_prompt_has_only_this_workspace() {
        let mut core = core();
        let ws = core.store.create_workspace().unwrap().summary.id;
        let other = core.store.create_workspace().unwrap().summary.id;
        core.store
            .update_draft(&other, "private other chat", 1)
            .unwrap();
        let req = uuid::Uuid::new_v4().to_string();
        core.start(&ws, "hello", &req).unwrap();
        let completion = core.begin_complete(&ws, &req).unwrap();
        assert_eq!(
            completion.body["messages"][0]["content"],
            provider::SYSTEM_PROMPT
        );
        assert_eq!(completion.body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(completion.body["stream"], false);
        assert_eq!(completion.body["max_tokens"], 2048);
        assert!(!completion.body.to_string().contains("private other chat"));
        core.configure(ProviderInput {
            label: String::new(),
            base_url: "http://localhost:9220/v1".into(),
            model: "new-model".into(),
            api_key: None,
            clear_key: false,
        })
        .unwrap();
        assert!(completion.token.is_cancelled());
        assert!(core
            .apply_complete(&ws, &req, 0, Ok("late".into()))
            .is_err());
        assert_eq!(
            core.store.workspace(&ws).unwrap().messages[1].status,
            "cancelled"
        );
    }
}
