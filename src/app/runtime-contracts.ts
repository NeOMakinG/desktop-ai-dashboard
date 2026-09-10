import type { InterfaceSpec } from './interface-contracts';

export interface RuntimeCapabilities {
  contractVersion: 'forma-runtime-v1'; deviceId: string; libraryId: string; modelOrigin: string;
  runtime: { kind: 'hermes'; ready: boolean; revision: string; reason?: string };
  features: { eventPolling: boolean; interfaces: boolean; schedules: boolean; nativeToolBridge: boolean; generatedCodeExecution: false; liveGoogle: false };
  tools: { name: string; available: boolean; execution: 'runtime' | 'nativeDevice'; reason?: string }[];
  limits: RuntimeBudgets & { maxToolResultBytes: number; dailyModelRequests?: number; maxQueuedRuns?: number };
}
export interface RuntimeModel { id: string; name: string; available: boolean; reason?: string }
/** Native-managed lifecycle; no listener address or runtime credentials reach the renderer. */
export interface RuntimeStatus {
  state: 'starting' | 'ready' | 'needsModel' | 'error'; message: string | null; verified: boolean;
  generation: number; capabilities: RuntimeCapabilities | null; models: RuntimeModel[];
}
export interface RuntimeWorkspace { workspaceId: string; route: 'hermes'; modelId: string; generation: number; remoteInitialized: boolean }
export interface RuntimeBudgets { maxIterations: number; maxToolCalls: number; maxOutputTokens: number; maxDurationSeconds: number }
export interface RuntimeGrantRef { id: string; generation: number }
export type RuntimeRunState = 'queued' | 'running' | 'waiting_for_device' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted' | 'blocked';
export interface RuntimeRun {
  id: string; workspaceId: string; origin: 'operator' | 'schedule'; scheduleId?: string; state: RuntimeRunState;
  reason?: string; modelId: string; hermesRevision: string; budgets: RuntimeBudgets; createdAt: string;
  startedAt?: string; finishedAt?: string; lastSeq: number; finalMessage?: string;
}
export type RuntimeEvent = { runId: string; seq: number; at: string } & (
  { type: 'run.state'; payload: { state: RuntimeRunState; reason?: string } } |
  { type: 'assistant.delta' | 'assistant.message'; payload: { text: string } } |
  { type: 'tool.requested'; payload: { requestId: string; toolName: string; deviceId?: string } } |
  { type: 'tool.result'; payload: { requestId: string; outcome: string } } |
  { type: 'interface.proposed'; payload: { proposalId: string; interfaceId?: string; expectedRevision: number } } |
  { type: 'interface.updated'; payload: { interfaceId: string; revision: number } } |
  { type: 'schedule.created'; payload: { scheduleId: string; state: 'paused' } } |
  { type: 'error'; payload: { code: string; message: string; retryable: boolean } }
);
/** Native projection deliberately excludes native tool arguments and claim credentials. */
export interface RuntimeProgress { workspaceId: string; requestId: string; generation: number; run: RuntimeRun | null; events: RuntimeEvent[]; cursor: number; state: RuntimeRunState | 'uncertain'; }
export interface RuntimeInterface { provenance?: { dataMode: 'synthetic' | 'nonAccount'; sourceRunId?: string }; id: string; libraryId: string; revision: number; title: string; spec: InterfaceSpec; createdAt: string; updatedAt: string }
export interface RuntimeInterfaceRevision { interfaceId: string; revision: number; parentRevision?: number; title: string; spec: InterfaceSpec; createdAt: string }
export interface RuntimeProposal {
  id: string; workspaceId: string; sourceRunId?: string; interfaceId?: string; expectedRevision: number;
  title: string; spec: InterfaceSpec; state: 'pending' | 'published' | 'invalidated'; createdAt: string;
}
export interface RuntimeProposalInput { workspaceId: string; interfaceId?: string; expectedRevision: number; title: string; spec: InterfaceSpec }
export interface RuntimeScheduleInput {
  workspaceId: string; interfaceId: string; expectedInterfaceRevision: number; prompt: string; modelId: string;
  cron: string; timezone: 'UTC'; endAt: string; maxRuns: number; budgets: RuntimeBudgets; grantRefs: RuntimeGrantRef[];
}
export interface RuntimeSchedule {
  id: string; workspaceId: string; interfaceId: string; interfaceRevisionPolicy: 'latestAtRunStart'; version: number;
  state: 'paused' | 'enabled' | 'ended'; prompt: string; modelId: string; cron: string; timezone: 'UTC'; endAt: string;
  maxRuns: number; runsStarted: number; budgets: RuntimeBudgets; grantRefs: RuntimeGrantRef[]; createdAt: string;
  updatedAt: string; nextRunAt?: string; lastRunId?: string; reason?: string;
}
export interface RuntimeScheduleConsent { scheduleVersion: number; modelId: string; grantRefs: RuntimeGrantRef[] }
export interface RuntimeDeleted { id: string; deletedAt: string }
export type RuntimeInterfaceSummary = Omit<RuntimeInterface, 'spec'>;
export type RuntimeProposalSummary = Omit<RuntimeProposal, 'spec'>;
export type RuntimeScheduleSummary = Omit<RuntimeSchedule, 'prompt'>;
export type RuntimeRunSummary = Omit<RuntimeRun, 'finalMessage'>;
export type RuntimeInterfaceRevisionSummary = Omit<RuntimeInterfaceRevision, 'spec'>;
