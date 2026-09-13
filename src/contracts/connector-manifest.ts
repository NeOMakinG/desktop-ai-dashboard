// Connector manifest schema and validator — pure contract, fixture-only.
// No OAuth, no network, no real service integration. Erasable TypeScript only
// (runs under Node v22 built-in type stripping with zero dependencies).

export type Semver = string;
export type SourceKind = "web" | "api" | "file" | "fixture";
export type FreshnessUnits = "seconds" | "minutes" | "hours" | "days";

export interface ManifestError {
  field: string;
  code: string;
  message: string;
}

export interface BoundedReadOperation {
  name: string;
  kind: "boundedRead";
  maxItems: number;
  windowDays?: number;
}

export interface SourceMetadata {
  kind: SourceKind;
  label: string;
}

export interface FreshnessMetadata {
  units: FreshnessUnits;
  value: number;
}

export interface SdkCompatibility {
  min: Semver;
  max?: Semver;
}

export interface ConnectorManifest {
  /** Must be true: manifests in this contract are test fixtures, never real integrations. */
  isTestFixture: true;
  id: string;
  version: Semver;
  /** SDK version this manifest targets; checked against sdkCompatibility and the host SDK. */
  sdkVersion: Semver;
  sdkCompatibility: SdkCompatibility;
  operations: BoundedReadOperation[];
  prerequisites: { providerTermsAccepted: boolean };
  source: SourceMetadata;
  freshness: FreshnessMetadata;
  metadata?: Record<string, unknown>;
}

export interface ValidateOptions {
  /** Host SDK version; when provided, the SDK compatibility gate is enforced. */
  hostSdkVersion?: Semver;
}

export interface ValidationResult {
  ok: boolean;
  errors: ManifestError[];
}

const SEMVER_RE = /^\d+\.\d+\.\d+$/;
const ID_RE = /^[a-z0-9][a-z0-9-]*$/;

export function isValidSemver(v: unknown): v is Semver {
  return typeof v === "string" && SEMVER_RE.test(v);
}

/** Minimal explicit semver subset: exact X.Y.Z, numeric compare. Not full semver-spec coverage (no prerelease tags). */
export function compareSemver(a: Semver, b: Semver): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}

/** Case-insensitive deny-list of credential-like key shapes. Shared with tests and catalog sanitization. */
export function isSecretLikeKey(key: string): boolean {
  return /credential|token|secret|password|passphrase|api[-_]?key|apikey|client[-_]?secret|refresh[-_]?key|auth[-_]?header|bearer/i.test(
    key
  );
}

export function validateManifest(
  m: unknown,
  opts: ValidateOptions = {}
): ValidationResult {
  const errors: ManifestError[] = [];
  const o = (m ?? {}) as Record<string, any>;

  if (typeof o.id !== "string" || o.id.length === 0) {
    errors.push({ field: "id", code: "MISSING_ID", message: "manifest id is required" });
  } else if (!ID_RE.test(o.id)) {
    errors.push({ field: "id", code: "INVALID_ID", message: "manifest id must match " + ID_RE.source });
  }

  if (typeof o.version !== "string" || o.version.length === 0) {
    errors.push({ field: "version", code: "MISSING_VERSION", message: "manifest version is required" });
  } else if (!isValidSemver(o.version)) {
    errors.push({ field: "version", code: "INVALID_SEMVER", message: "version must be exact X.Y.Z semver" });
  }

  if (typeof o.sdkVersion !== "string" || o.sdkVersion.length === 0) {
    errors.push({ field: "sdkVersion", code: "MISSING_SDK_VERSION", message: "sdkVersion is required" });
  } else if (!isValidSemver(o.sdkVersion)) {
    errors.push({ field: "sdkVersion", code: "INVALID_SEMVER", message: "sdkVersion must be exact X.Y.Z semver" });
  }

  const compat = o.sdkCompatibility ?? {};
  if (!isValidSemver(compat.min)) {
    errors.push({ field: "sdkCompatibility.min", code: "INVALID_SEMVER", message: "sdkCompatibility.min must be exact X.Y.Z semver" });
  }
  if (compat.max !== undefined && !isValidSemver(compat.max)) {
    errors.push({ field: "sdkCompatibility.max", code: "INVALID_SEMVER", message: "sdkCompatibility.max must be exact X.Y.Z semver" });
  }

  if (
    isValidSemver(o.sdkVersion) &&
    isValidSemver(compat.min) &&
    (compat.max === undefined || isValidSemver(compat.max))
  ) {
    if (compareSemver(o.sdkVersion, compat.min) < 0) {
      errors.push({ field: "sdkVersion", code: "SDK_OUT_OF_RANGE", message: "sdkVersion is below sdkCompatibility.min" });
    }
    if (compat.max !== undefined && compareSemver(o.sdkVersion, compat.max) > 0) {
      errors.push({ field: "sdkVersion", code: "SDK_OUT_OF_RANGE", message: "sdkVersion is above sdkCompatibility.max" });
    }
  }

  if (
    opts.hostSdkVersion !== undefined &&
    isValidSemver(opts.hostSdkVersion) &&
    isValidSemver(compat.min) &&
    (compat.max === undefined || isValidSemver(compat.max))
  ) {
    if (
      compareSemver(opts.hostSdkVersion, compat.min) < 0 ||
      (compat.max !== undefined && compareSemver(opts.hostSdkVersion, compat.max) > 0)
    ) {
      errors.push({ field: "sdkVersion", code: "SDK_OUT_OF_RANGE", message: "host SDK version is outside the manifest compatibility range" });
    }
  }

  if (!Array.isArray(o.operations) || o.operations.length === 0) {
    errors.push({ field: "operations", code: "MISSING_OPERATIONS", message: "at least one declared operation is required" });
  } else {
    const seen = new Set<string>();
    o.operations.forEach((op: any, i: number) => {
      if (op === null || typeof op !== "object") {
        errors.push({ field: `operations[${i}]`, code: "INVALID_OPERATION", message: "operation must be an object" });
        return;
      }
      if (op.kind !== "boundedRead") {
        errors.push({ field: `operations[${i}].kind`, code: "NON_BOUNDED_KIND", message: `undeclared or non-bounded operation kind "${String(op.kind)}"; only "boundedRead" is declared by this contract` });
      }
      if (typeof op.name !== "string" || op.name.length === 0) {
        errors.push({ field: `operations[${i}].name`, code: "MISSING_OPERATION_NAME", message: "operation name is required" });
      } else if (seen.has(op.name)) {
        errors.push({ field: "operations", code: "DUPLICATE_OPERATION_NAME", message: `duplicate operation name "${op.name}"` });
      } else {
        seen.add(op.name);
      }
      if (typeof op.maxItems !== "number" || !Number.isInteger(op.maxItems) || op.maxItems <= 0) {
        errors.push({ field: `operations[${i}].maxItems`, code: "INVALID_MAX_ITEMS", message: "bounded-read maxItems must be a positive integer" });
      }
    });
  }

  const prereq = o.prerequisites ?? {};
  if (typeof prereq.providerTermsAccepted !== "boolean") {
    errors.push({ field: "prerequisites.providerTermsAccepted", code: "MISSING_PROVIDER_TERMS", message: "provider terms prerequisite must be declared as a boolean" });
  }

  const source = o.source ?? {};
  if (typeof source.kind !== "string" || !["web", "api", "file", "fixture"].includes(source.kind)) {
    errors.push({ field: "source.kind", code: "MISSING_SOURCE_KIND", message: "explicit source kind (web|api|file|fixture) is required" });
  }

  const fresh = o.freshness ?? {};
  if (typeof fresh.units !== "string" || !["seconds", "minutes", "hours", "days"].includes(fresh.units)) {
    errors.push({ field: "freshness.units", code: "MISSING_FRESHNESS_UNITS", message: "explicit freshness units (seconds|minutes|hours|days) are required" });
  }
  if (typeof fresh.value !== "number" || !(fresh.value > 0)) {
    errors.push({ field: "freshness.value", code: "INVALID_FRESHNESS_VALUE", message: "freshness value must be a positive number" });
  }

  if (o.isTestFixture !== true) {
    errors.push({ field: "isTestFixture", code: "NOT_FIXTURE_LABELED", message: "manifest must be labeled isTestFixture: true; this contract carries fixtures only" });
  }

  // Secret-like credential fields, including nested metadata.
  scanSecretKeys(m, "", errors);

  return { ok: errors.length === 0, errors };
}

function scanSecretKeys(node: unknown, prefix: string, errors: ManifestError[]): void {
  if (node === null || typeof node !== "object") return;
  if (Array.isArray(node)) {
    node.forEach((v, i) => scanSecretKeys(v, `${prefix}[${i}]`, errors));
    return;
  }
  for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${k}` : k;
    if (isSecretLikeKey(k)) {
      errors.push({ field: path, code: "SECRET_LIKE_FIELD", message: `credential-like field "${k}" is not allowed in a manifest` });
    }
    scanSecretKeys(v, path, errors);
  }
}

/**
 * SYNTHETIC REFERENCE MAIL CONNECTOR — TEST FIXTURE.
 * This is a developer example and contract fixture. It is NOT a real service
 * integration and must never be presented as a connected account.
 */
export const REFERENCE_MAIL_MANIFEST: ConnectorManifest = {
  isTestFixture: true,
  id: "fixture-reference-mail",
  version: "1.0.0",
  sdkVersion: "22.0.0",
  sdkCompatibility: { min: "22.0.0", max: "22.9.0" },
  operations: [
    { name: "listRecentMessages", kind: "boundedRead", maxItems: 100, windowDays: 7 },
    { name: "readMessageHeaders", kind: "boundedRead", maxItems: 100, windowDays: 7 }
  ],
  prerequisites: { providerTermsAccepted: true },
  source: { kind: "fixture", label: "synthetic in-memory mail fixture" },
  freshness: { units: "hours", value: 24 },
  metadata: {
    note: "reference fixture for contract tests; no real mailbox"
  }
};
