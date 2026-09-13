use serde::{Deserialize, Serialize};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AppError {
    pub code: &'static str,
    pub message: &'static str,
}
impl AppError {
    pub const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
    pub fn storage() -> Self {
        Self::new("storage", "Local storage is unavailable. Restart Forma or restore a known-good backup; your data has not been reset.")
    }
    pub fn invalid() -> Self {
        Self::new(
            "invalid_input",
            "Some input is invalid or exceeds the supported limit.",
        )
    }
    pub fn missing() -> Self {
        Self::new("not_found", "This workspace or request no longer exists.")
    }
    pub fn stale() -> Self {
        Self::new(
            "stale_request",
            "The request was cancelled or the provider configuration changed.",
        )
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(_: rusqlite::Error) -> Self {
        Self::storage()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsInput {
    pub onboarding_complete: bool,
    pub onboarding_step: u8,
    pub display_name: String,
    pub ambient_motion: bool,
    /// "Assistant may drive the browser": gates the forma_browser_session
    /// CDP hand-off. Defaults OFF, including for preferences stored before
    /// this field existed (serde default keeps old rows decodable).
    #[serde(default)]
    pub assistant_browser_drive: bool,
}
impl Default for SettingsInput {
    fn default() -> Self {
        Self {
            onboarding_complete: false,
            onboarding_step: 0,
            display_name: String::new(),
            ambient_motion: true,
            assistant_browser_drive: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderInput {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_key: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfig {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    pub verified: bool,
    pub last_checked_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u8,
    #[serde(flatten)]
    pub preferences: SettingsInput,
    pub provider: ProviderConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWorkspace {
    #[serde(flatten)]
    pub summary: WorkspaceSummary,
    pub draft: String,
    pub draft_version: u64,
    pub messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub created_at: String,
    pub request_id: Option<String>,
}

#[derive(Serialize)]
pub struct Bootstrap {
    pub settings: AppSettings,
    pub workspaces: Vec<WorkspaceSummary>,
}

#[derive(Serialize)]
pub struct ProviderCheck {
    pub provider: ProviderConfig,
    pub models: Vec<String>,
}

#[derive(Serialize)]
pub struct ModelList {
    pub models: Vec<String>,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
