import { invoke } from '@tauri-apps/api/core';

export type ConnectorReadOperation = 'forma_gmail_list_metadata' | 'forma_calendar_list_events';
export type ConnectorDataCategory = 'gmail.metadata' | 'calendar.events';
export type ConnectorEgressConsent = {
  runtimeOrigin: string;
  modelOrigin: string;
  dataCategories: ConnectorDataCategory[];
  consented: boolean;
  retention: 'ephemeral-run';
};
export type ConnectorGrantRequest = {
  workspaceId: string;
  deviceId: string;
  connectionId: string;
  operations: ConnectorReadOperation[];
  expiresAt: string;
  accountReadConsented: boolean;
  egress: ConnectorEgressConsent;
};
export type ConnectorGrant = ConnectorGrantRequest & { grantId: string; generation: number };
export type ConnectorReadCapabilities = {
  liveGoogle: false;
  registrationAvailable: boolean;
  reasons: string[];
  maxGrantSeconds: number;
  maxReadSeconds: number;
  maxWindowDays: number;
  maxItems: number;
};

export const CONNECTOR_EGRESS_DISCLOSURE =
  'Account permission is separate from model processing. Only the selected metadata categories may be sent to the displayed runtime and model origins. Grants expire within one hour and are not restored on restart. Already transmitted data cannot be recalled. Live reading remains blocked until Google compliance and remote retention/purge are verified; this control does not enable schedules.';
export const CONNECTOR_READS_UNAVAILABLE: ConnectorReadCapabilities = {
  liveGoogle: false,
  registrationAvailable: false,
  reasons: ['Native account tools are unavailable. Google client registration, policy compliance, and remote retention/revocation evidence are required.'],
  maxGrantSeconds: 3600,
  maxReadSeconds: 120,
  maxWindowDays: 7,
  maxItems: 100,
};

// No read dispatcher in renderer JS. The native runtime alone constructs trusted
// run envelopes; these functions belong to first-party operator controls only.
export const listConnectorGrants = (): Promise<ConnectorGrant[]> => invoke('connectors_grants_list');
export const connectorReadCapabilities = (): Promise<ConnectorReadCapabilities> => invoke('connectors_read_capabilities');
export const registerConnectorGrant = (request: ConnectorGrantRequest): Promise<ConnectorGrant> => invoke('connectors_grant_register', { request });
export const revokeConnectorGrant = (grantId: string): Promise<void> => invoke('connectors_grant_revoke', { grantId });
export const cancelConnectorRun = (workspaceId: string, runId: string): Promise<void> => invoke('connectors_run_cancel', { workspaceId, runId });
