use serde::{Deserialize, Serialize};

// Internal durable identity fence. No runtime connection settings are exposed to IPC.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    pub endpoint: String,
    pub enabled: bool,
    pub verified: bool,
    pub generation: u64,
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            endpoint: super::transport::LOCAL_BINDING.into(),
            enabled: true,
            verified: false,
            generation: 1,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredRuntime {
    pub config: RuntimeConfig,
    pub state: String,
    pub message: Option<String>,
    pub config_epoch: String,
    pub model_generation: Option<u64>,
    pub background_approved: bool,
    #[serde(default)]
    pub approved_models: Vec<String>,
    #[serde(default)]
    pub bound_library_id: Option<String>,
    #[serde(default)]
    pub bound_device_id: Option<String>,
}
impl Default for StoredRuntime {
    fn default() -> Self {
        Self {
            config: RuntimeConfig::default(),
            state: "starting".into(),
            message: None,
            config_epoch: uuid::Uuid::new_v4().to_string(),
            model_generation: None,
            background_approved: false,
            approved_models: Vec::new(),
            bound_library_id: None,
            bound_device_id: None,
        }
    }
}
