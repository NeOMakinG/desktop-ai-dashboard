export type MessageRole = 'user' | 'assistant';
export type MessageStatus = 'complete' | 'pending' | 'error' | 'cancelled';

export interface ChatMessage {
  id: string;
  role: MessageRole;
  content: string;
  status: MessageStatus;
  createdAt: string;
  requestId: string | null;
}

export interface WorkspaceSummary {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export interface ChatWorkspace extends WorkspaceSummary {
  draft: string;
  draftVersion: number;
  messages: ChatMessage[];
}

export interface ProviderConfig {
  label: string;
  baseUrl: string;
  model: string;
  hasKey: boolean;
  verified: boolean;
  lastCheckedAt: string | null;
}

export interface AppSettings {
  schemaVersion: 1;
  onboardingComplete: boolean;
  onboardingStep: number;
  displayName: string;
  ambientMotion: boolean;
  /** "Assistant may drive the browser" — gates the CDP hand-off. Default off. */
  assistantBrowserDrive: boolean;
  provider: ProviderConfig;
}

export interface SettingsInput {
  onboardingComplete: boolean;
  onboardingStep: number;
  displayName: string;
  ambientMotion: boolean;
  assistantBrowserDrive: boolean;
}

export interface ProviderInput {
  label: string;
  baseUrl: string;
  model: string;
  apiKey?: string;
  clearKey?: boolean;
}

export interface Bootstrap {
  settings: AppSettings;
  workspaces: WorkspaceSummary[];
}

export interface ProviderCheck {
  provider: ProviderConfig;
  models: string[];
}

export interface ModelList {
  models: string[];
}

export interface AppBridge {
  readonly native: boolean;
  runtimeStatus?(): Promise<import('./runtime-contracts').RuntimeStatus>;
  runtimeWorkspace?(workspaceId: string): Promise<import('./runtime-contracts').RuntimeWorkspace>;
  runtimeRetry?(): Promise<import('./runtime-contracts').RuntimeStatus>;
  finishWindowClose?(): Promise<'hidden' | 'closed'>;
  runtimeSelectModel?(workspaceId: string, model: string): Promise<import('./runtime-contracts').RuntimeWorkspace>;
  runtimeCheck?(): Promise<import('./runtime-contracts').RuntimeStatus>;
  runtimeProgress?(workspaceId: string, requestId: string): Promise<import('./runtime-contracts').RuntimeProgress>;
  bootstrap(): Promise<Bootstrap>;
  saveSettings(input: SettingsInput): Promise<AppSettings>;
  configureProvider(input: ProviderInput): Promise<ProviderConfig>;
  checkProvider(): Promise<ProviderCheck>;
  listModels(): Promise<ModelList>;
  createWorkspace(): Promise<ChatWorkspace>;
  getWorkspace(id: string): Promise<ChatWorkspace>;
  updateDraft(id: string, draft: string, version: number): Promise<ChatWorkspace>;
  renameWorkspace(id: string, title: string): Promise<ChatWorkspace>;
  deleteWorkspace(id: string): Promise<void>;
  startMessage(id: string, content: string, requestId: string): Promise<ChatWorkspace>;
  completeMessage(id: string, requestId: string): Promise<ChatWorkspace>;
  cancelMessage(id: string, requestId: string): Promise<ChatWorkspace>;
}
