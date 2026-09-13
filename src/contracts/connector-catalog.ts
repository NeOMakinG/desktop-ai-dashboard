// Connector catalog with honest readiness resolution — pure contract, fixture-only.
// Reason codes only: no OAuth, no popups, no grant/expiry/revocation enforcement
// (owned by existing host approval modules in runtime/), no native setup, no real services.

import {
  validateManifest,
  isSecretLikeKey,
  type ConnectorManifest,
  type ManifestError
} from "./connector-manifest.ts";

export type ReasonCode =
  | "UNKNOWN_CONNECTOR"
  | "MANIFEST_INVALID"
  | "DUPLICATE_ID"
  | "SDK_OUT_OF_RANGE"
  | "FIXTURE_DISABLED"
  | "BROWSER_SESSION_REQUIRED"
  | "API_AUTHORIZATION_REQUIRED"
  | "OS_PERMISSION_REQUIRED";

export interface ReadinessComponents {
  browserSession: boolean;
  apiAuthorization: boolean;
  osPermission: boolean;
}

export interface CatalogEntry {
  id: string;
  connected: boolean;
  reasonCode?: ReasonCode;
  unsatisfiedComponents: string[];
  manifestErrors?: ManifestError[];
}

interface StoredConnector {
  id: string;
  manifest: unknown;
}

export interface ConnectorCatalog {
  connectors: Map<string, StoredConnector>;
  duplicateIds: Set<string>;
  invalidIds: Set<string>;
}

export interface ResolveOptions {
  /** Explicit fixture enablement; without it a connector is never connected. */
  fixtureEnabled: boolean;
  /** Host SDK version; the version gate is re-checked at resolve time, not only at add time. */
  hostSdkVersion?: string;
}

export interface AddResult {
  ok: boolean;
  reasonCode?: ReasonCode;
  errors?: ManifestError[];
}

export function createCatalog(): ConnectorCatalog {
  return { connectors: new Map(), duplicateIds: new Set(), invalidIds: new Set() };
}

export function addConnector(
  catalog: ConnectorCatalog,
  manifest: unknown,
  opts: ResolveOptions = { fixtureEnabled: false }
): AddResult {
  const id = (manifest as any)?.id;
  if (typeof id !== "string") {
    return { ok: false, reasonCode: "MANIFEST_INVALID", errors: validateManifest(manifest, opts).errors };
  }
  if (catalog.connectors.has(id)) {
    catalog.duplicateIds.add(id);
    return { ok: false, reasonCode: "DUPLICATE_ID" };
  }
  const result = validateManifest(manifest, opts);
  if (!result.ok) {
    catalog.invalidIds.add(id);
    catalog.connectors.set(id, { id, manifest });
    return { ok: false, reasonCode: "MANIFEST_INVALID", errors: result.errors };
  }
  catalog.connectors.set(id, { id, manifest });
  return { ok: true };
}

/** Resolve readiness honestly: manifest claims can never force a connected state. */
export function resolve(
  catalog: ConnectorCatalog,
  id: string,
  readiness: ReadinessComponents,
  opts: ResolveOptions
): CatalogEntry {
  const stored = catalog.connectors.get(id);
  if (!stored) {
    return { id, connected: false, reasonCode: "UNKNOWN_CONNECTOR", unsatisfiedComponents: [] };
  }
  if (catalog.duplicateIds.has(id)) {
    return { id, connected: false, reasonCode: "DUPLICATE_ID", unsatisfiedComponents: [] };
  }
  const validation = validateManifest(stored.manifest, opts);
  if (!validation.ok) {
    return {
      id,
      connected: false,
      reasonCode: "MANIFEST_INVALID",
      unsatisfiedComponents: [],
      manifestErrors: validation.errors
    };
  }
  const sdkError = validation.errors;
  void sdkError;
  if (hasSdkOutOfRange(validation)) {
    return { id, connected: false, reasonCode: "SDK_OUT_OF_RANGE", unsatisfiedComponents: [] };
  }
  if (!opts.fixtureEnabled) {
    return { id, connected: false, reasonCode: "FIXTURE_DISABLED", unsatisfiedComponents: [] };
  }

  // Components are evaluated independently; any unsatisfied one blocks connection.
  const unsatisfied: string[] = [];
  if (!readiness.browserSession) unsatisfied.push("browserSession");
  if (!readiness.apiAuthorization) unsatisfied.push("apiAuthorization");
  if (!readiness.osPermission) unsatisfied.push("osPermission");
  if (unsatisfied.length > 0) {
    const reasonCode: ReasonCode =
      unsatisfied[0] === "browserSession"
        ? "BROWSER_SESSION_REQUIRED"
        : unsatisfied[0] === "apiAuthorization"
          ? "API_AUTHORIZATION_REQUIRED"
          : "OS_PERMISSION_REQUIRED";
    return { id, connected: false, reasonCode, unsatisfiedComponents: unsatisfied };
  }
  return { id, connected: true, unsatisfiedComponents: [] };
}

function hasSdkOutOfRange(validation: { errors: ManifestError[] }): boolean {
  return validation.errors.some((e) => e.code === "SDK_OUT_OF_RANGE");
}

/** Actionable setup seam for both lookup and resolve: never null, never fabricated connected. */
export function lookup(
  catalog: ConnectorCatalog,
  id: string,
  readiness: ReadinessComponents,
  opts: ResolveOptions
): CatalogEntry {
  return resolve(catalog, id, readiness, opts);
}

export function connectedSet(
  catalog: ConnectorCatalog,
  readiness: ReadinessComponents,
  opts: ResolveOptions
): string[] {
  const out: string[] = []
  for (const id of catalog.connectors.keys()) {
    if (resolve(catalog, id, readiness, opts).connected) out.push(id);
  }
  return out;
}

/** Deep-clone with credential-like keys removed; suitable for fixture receipts. */
export function sanitize(value: unknown): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(sanitize);
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    if (isSecretLikeKey(k)) continue;
    out[k] = sanitize(v);
  }
  return out;
}

export function serializeManifest(manifest: unknown): string {
  return JSON.stringify(sanitize(manifest));
}

export function serializeEntry(entry: CatalogEntry): string {
  return JSON.stringify(sanitize(entry));
}
