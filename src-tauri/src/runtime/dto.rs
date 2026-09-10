use super::config::RuntimeConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    pub contract_version: String,
    pub device_id: String,
    pub library_id: String,
    pub model_origin: String,
    pub runtime: RuntimeIdentity,
    pub features: Features,
    pub tools: Vec<ToolCapability>,
    pub limits: Limits,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub kind: String,
    pub ready: bool,
    pub revision: String,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Features {
    pub event_polling: bool,
    pub interfaces: bool,
    pub schedules: bool,
    pub native_tool_bridge: bool,
    pub generated_code_execution: bool,
    pub live_google: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCapability {
    pub name: String,
    pub available: bool,
    pub execution: String,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
    pub max_duration_seconds: u32,
    pub max_tool_result_bytes: u32,
    pub daily_model_requests: Option<u32>,
    pub max_queued_runs: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budgets {
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
    pub max_duration_seconds: u32,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_iterations: 8,
            max_tool_calls: 12,
            max_output_tokens: 4096,
            max_duration_seconds: 180,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeModel {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Items<T> {
    pub items: Vec<T>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    #[serde(skip)]
    pub config: RuntimeConfig,
    pub state: String,
    pub message: Option<String>,
    pub generation: u64,
    pub verified: bool,
    pub capabilities: Option<Capabilities>,
    pub models: Vec<RuntimeModel>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Hermes,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWorkspace {
    pub workspace_id: String,
    pub route: Route,
    pub model_id: String,
    pub generation: u64,
    pub remote_initialized: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantRef {
    pub id: String,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Running,
    WaitingForDevice,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    Blocked,
}
impl RunState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted | Self::Blocked
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Run {
    pub id: String,
    pub workspace_id: String,
    pub origin: String,
    pub schedule_id: Option<String>,
    pub state: RunState,
    pub reason: Option<String>,
    pub model_id: String,
    pub hermes_revision: String,
    pub budgets: Budgets,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub last_seq: u64,
    pub final_message: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub run_id: String,
    pub seq: u64,
    pub at: String,
    #[serde(flatten)]
    pub kind: EventKind,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum EventKind {
    #[serde(rename = "run.state")]
    State {
        state: RunState,
        reason: Option<String>,
    },
    #[serde(rename = "assistant.delta")]
    Delta { text: String },
    #[serde(rename = "assistant.message")]
    Message { text: String },
    #[serde(rename = "tool.requested")]
    ToolRequested {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "deviceId")]
        device_id: Option<String>,
    },
    #[serde(rename = "tool.result")]
    ToolResult {
        #[serde(rename = "requestId")]
        request_id: String,
        outcome: String,
    },
    #[serde(rename = "interface.proposed")]
    InterfaceProposed {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        #[serde(rename = "interfaceId")]
        interface_id: Option<String>,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
    #[serde(rename = "interface.updated")]
    InterfaceUpdated {
        #[serde(rename = "interfaceId")]
        interface_id: String,
        revision: u64,
    },
    #[serde(rename = "schedule.created")]
    ScheduleCreated {
        #[serde(rename = "scheduleId")]
        schedule_id: String,
        state: String,
    },
    #[serde(rename = "error")]
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRequest {
    pub id: String,
    pub run_id: String,
    pub workspace_id: String,
    pub device_id: String,
    pub connection_id: String,
    pub grant_ref: GrantRef,
    pub tool_name: String,
    pub args: Value,
    pub expires_at: String,
    pub state: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunPoll {
    pub run: Run,
    pub events: Vec<Event>,
    pub next_after: u64,
    pub has_more: bool,
    pub tool_requests: Vec<ToolRequest>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelReceipt {
    pub id: String,
    pub state: RunState,
    pub admitted: bool,
    pub run: Option<Run>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolClaim {
    pub request: ToolRequest,
    pub claim_token: String,
    pub claim_expires_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunInput {
    pub workspace_id: String,
    pub message: String,
    pub model_id: String,
    pub grant_refs: Vec<GrantRef>,
    pub interface_ids: Vec<String>,
    pub budgets: Budgets,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub import_history: Vec<HistoryMessage>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeProgress {
    pub workspace_id: String,
    pub request_id: String,
    pub generation: u64,
    pub run: Option<Run>,
    pub events: Vec<Event>,
    pub cursor: u64,
    pub state: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceProvenance {
    pub data_mode: InterfaceDataMode,
    pub source_run_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum InterfaceDataMode {
    #[serde(rename = "synthetic")]
    Synthetic,
    #[serde(rename = "nonAccount")]
    NonAccount,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interface {
    pub provenance: Option<InterfaceProvenance>,
    pub id: String,
    pub library_id: String,
    pub revision: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub spec: Value,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub id: String,
    pub workspace_id: String,
    pub source_run_id: Option<String>,
    pub interface_id: Option<String>,
    pub expected_revision: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub spec: Value,
    pub state: String,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalInput {
    pub workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_id: Option<String>,
    pub expected_revision: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub spec: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceRevision {
    pub interface_id: String,
    pub revision: u64,
    pub parent_revision: Option<u64>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub spec: Value,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleInput {
    pub workspace_id: String,
    pub interface_id: String,
    pub expected_interface_revision: u64,
    pub prompt: String,
    pub model_id: String,
    pub cron: String,
    pub timezone: String,
    pub end_at: String,
    pub max_runs: u32,
    pub budgets: Budgets,
    pub grant_refs: Vec<GrantRef>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Schedule {
    pub id: String,
    pub workspace_id: String,
    pub interface_id: String,
    pub interface_revision_policy: String,
    pub version: u64,
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    pub model_id: String,
    pub cron: String,
    pub timezone: String,
    pub end_at: String,
    pub max_runs: u32,
    pub runs_started: u32,
    pub budgets: Budgets,
    pub grant_refs: Vec<GrantRef>,
    pub created_at: String,
    pub updated_at: String,
    pub next_run_at: Option<String>,
    pub last_run_id: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleConsent {
    pub schedule_version: u64,
    pub model_id: String,
    pub grant_refs: Vec<GrantRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Deleted {
    pub id: String,
    pub deleted_at: String,
}
