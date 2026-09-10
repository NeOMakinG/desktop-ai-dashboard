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

pub struct Core {
    pub store: Store,
    pub(crate) credentials: Box<dyn Credentials>,
    pub(crate) runtime_requests: HashMap<String, CancellationToken>,
    pub(crate) controller: Option<crate::runtime::transport::Controller>,
    pub(crate) synced_model_generation: Option<u64>,
    check: Option<CancellationToken>,
    discovery: Option<CancellationToken>,
}
pub struct NativeState {
    inner: Mutex<AppResult<Core>>,
    pub client: AppResult<reqwest::Client>,
    pub runtime_gate: tokio::sync::Mutex<()>,
    pub runtime_stopping: std::sync::atomic::AtomicBool,
    pub runtime_process: crate::runtime::transport::ProcessOwner,
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
            runtime_gate: tokio::sync::Mutex::new(()),
            runtime_stopping: std::sync::atomic::AtomicBool::new(false),
            runtime_process: crate::runtime::transport::ProcessOwner::default(),
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
            runtime_requests: HashMap::new(),
            controller: None,
            synced_model_generation: None,
            check: None,
            discovery: None,
        }
    }
    pub(crate) fn key(&self, provider: &StoredProvider) -> AppResult<Option<Zeroizing<String>>> {
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
        crate::runtime::managed::provider_changed(self)?;
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
        crate::runtime::runs::start(self, workspace, content, request)
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
        self.store.workspace(workspace)
    }
    pub fn delete(&mut self, workspace: &str) -> AppResult<()> {
        self.store.delete(workspace)?;
        Ok(())
    }
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
    check_model(state, None).await
}
pub async fn check_selected_model(
    state: &NativeState,
    model: &str,
    generation: u64,
) -> AppResult<ProviderCheck> {
    validation::model(model, false)?;
    check_model(state, Some((model, generation))).await
}
pub(crate) fn model_check_failed(core: &Core) -> AppResult<()> {
    let mut runtime = core.store.runtime_config()?;
    runtime.state = "error".into();
    runtime.message = Some(
        "The model check failed. Your local Hermes library and schedule Pause remain available."
            .into(),
    );
    core.store.db.execute(
        "UPDATE runtime_config SET value=?1 WHERE id=1",
        [crate::runtime::storage::encode(&runtime)?],
    )?;
    Ok(())
}
async fn check_model(
    state: &NativeState,
    selection: Option<(&str, u64)>,
) -> AppResult<ProviderCheck> {
    let client = state.client.as_ref().map_err(Clone::clone)?;
    let (mut snapshot, key, token) = {
        let mut core = state.lock()?;
        if core.check.is_some() {
            return Err(AppError::new(
                "busy",
                "A model connection check is already running.",
            ));
        }
        let mut provider = core.store.provider()?;
        if selection.is_some_and(|(_, generation)| generation != provider.generation) {
            return Err(AppError::stale());
        }
        if provider.config.base_url.is_empty() {
            return Err(AppError::new(
                "provider_not_ready",
                "Enter your model provider endpoint before checking.",
            ));
        }
        let key = core.key(&provider)?;
        provider.config.verified = false;
        provider.config.last_checked_at = None;
        core.store.set_provider(&provider)?;
        let mut runtime = core.store.runtime_config()?;
        runtime.state = "needsModel".into();
        runtime.message = Some("Checking the approved model connection.".into());
        core.store.db.execute(
            "UPDATE runtime_config SET value=?1 WHERE id=1",
            [crate::runtime::storage::encode(&runtime)?],
        )?;
        let token = CancellationToken::new();
        core.check = Some(token.clone());
        (provider, key, token)
    };
    let result = tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()),
    result = provider::check(client,&snapshot.config.base_url,key.as_deref().map(String::as_str)) => result };
    let models = {
        let mut core = state.lock()?;
        if token.is_cancelled() || core.store.provider()?.generation != snapshot.generation {
            return Err(AppError::stale());
        }
        core.check = None;
        let models = match result {
            Ok(models) => models,
            Err(error) => {
                model_check_failed(&core)?;
                return Err(error);
            }
        };
        let chosen = selection
            .map(|(model, _)| model)
            .unwrap_or(&snapshot.config.model);
        let chosen = match provider::selected_model(chosen, &models) {
            Ok(chosen) => chosen.to_owned(),
            Err(error) => {
                model_check_failed(&core)?;
                return Err(error);
            }
        };
        // The workspace picker verifies its selection without rewriting app-wide defaults.
        if selection.is_none() {
            snapshot.config.model = chosen;
        }
        snapshot.config.verified = true;
        snapshot.config.last_checked_at = Some(now());
        core.store.set_provider(&snapshot)?;
        models
    };
    if let Err(error) =
        crate::runtime::managed::sync_model(state, Some((snapshot.generation, models.clone())))
            .await
    {
        let core = state.lock()?;
        if core.store.provider()?.generation == snapshot.generation {
            model_check_failed(&core)?;
        }
        return Err(error);
    }
    if token.is_cancelled() || state.lock()?.store.provider()?.generation != snapshot.generation {
        return Err(AppError::stale());
    }
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
    fn verified_provider_cannot_bypass_missing_owned_hermes() {
        let mut core = core();
        let id = core.store.create_workspace().unwrap().summary.id;
        core.store.update_draft(&id, "preserve draft", 1).unwrap();
        assert!(core
            .start(&id, "preserve draft", &uuid::Uuid::new_v4().to_string())
            .is_err());
        let workspace = core.store.workspace(&id).unwrap();
        assert!(workspace.messages.is_empty());
        assert_eq!(workspace.draft, "preserve draft");
        assert_eq!(
            core.store.runtime_workspace(&id).unwrap().route,
            crate::runtime::dto::Route::Hermes
        );
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
}
