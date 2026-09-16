/**
 * Pure host-side contract and fixtures for scoped Interface tool requests.
 *
 * PURE CONTRACT FIXTURES ONLY. This module defines request/result shapes and
 * deterministic classification rules for the five host-exposed Interface agent
 * tools. It has zero runtime, network, native, or IO dependencies and performs
 * no host state access. Nothing here is evidence of live Hermes, native app,
 * browser, backend, device, or account acceptance; parent HI06
 * (hermes-interface-tools) remains OPEN and unclaimed.
 */

// ---------- Tool names ----------

export type InterfaceToolName =
  | "interface_create"
  | "interface_inspect"
  | "interface_rename"
  | "interface_revise"
  | "interface_remove";

export const INTERFACE_TOOL_NAMES: readonly InterfaceToolName[] = [
  "interface_create",
  "interface_inspect",
  "interface_rename",
  "interface_revise",
  "interface_remove",
];

// ---------- Documented size bound ----------

/**
 * Maximum serialized payload size accepted for any single Interface tool
 * request. This is a product decision frozen by this contract; a future
 * parent-scope job may amend it in this one place.
 */
export const MAX_PAYLOAD_BYTES = 65536;

// ---------- Requests ----------

export interface InterfaceCreateRequest {
  tool: "interface_create";
  workspace_id: string;
  name: string;
  spec: object;
}

export interface InterfaceInspectRequest {
  tool: "interface_inspect";
  workspace_id: string;
  object_id: string;
}

export interface InterfaceRenameRequest {
  tool: "interface_rename";
  workspace_id: string;
  object_id: string;
  name: string;
}

export interface InterfaceReviseRequest {
  tool: "interface_revise";
  workspace_id: string;
  object_id: string;
  expected_revision: string;
  spec: object;
}

export interface InterfaceRemoveRequest {
  tool: "interface_remove";
  workspace_id: string;
  object_id: string;
}

export type InterfaceToolRequest =
  | InterfaceCreateRequest
  | InterfaceInspectRequest
  | InterfaceRenameRequest
  | InterfaceReviseRequest
  | InterfaceRemoveRequest;

// ---------- Host-supplied snapshot (pure data in, classified result out) ----------

export type ObjectLifecycle = "active" | "deleted" | "revoked";

/** Pure caller-supplied snapshot describing grants and object state. */
export interface ScopeSnapshot {
  granted_workspace_ids: readonly string[];
  granted_object_ids: readonly string[];
  /**
   * Lifecycle of the target object, if known. Unknown (absent) objects are
   * treated as outside scope for object-referencing tools; interface_create
   * with an existing object_id is not applicable (create has no object_id).
   */
  object_lifecycle?: ObjectLifecycle;
  /** Current revision of the target object, for revise conflict checks. */
  current_revision?: string;
}

// ---------- Results ----------

export type InterfaceToolResultKind =
  | "ok"
  | "validation_error"
  | "grant_mismatch"
  | "stale_revision"
  | "tombstone"
  | "revoked";

export interface InterfaceToolResult {
  kind: InterfaceToolResultKind;
  /** Names the offending field or size limit for validation errors. */
  field?: string;
  tool: InterfaceToolName;
  /**
   * For stale_revision: the last working revision, preserved unchanged.
   * No result path applies a partial revision.
   */
  preserved_revision?: string;
  message: string;
}

// ---------- Routine vs consequential (ADR 0004 host approval contract) ----------

export type ActionClassification = "routine" | "consequential";

/**
 * interface_remove is always consequential; create/inspect/rename and
 * in-scope revise are routine per the host approval contract.
 */
export const ACTION_CLASSIFICATIONS: Readonly<Record<InterfaceToolName, ActionClassification>> = {
  interface_create: "routine",
  interface_inspect: "routine",
  interface_rename: "routine",
  interface_revise: "routine",
  interface_remove: "consequential",
};

export function classifyAction(tool: InterfaceToolName): ActionClassification {
  return ACTION_CLASSIFICATIONS[tool];
}

// ---------- Validation ----------

function validationError(tool: InterfaceToolName, field: string, message: string): InterfaceToolResult {
  return { kind: "validation_error", tool, field, message };
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Deterministic validation. Rejects malformed shapes, missing fields, and
 * payloads above MAX_PAYLOAD_BYTES. Always returns a classified result naming
 * the offending field or the size limit; never throws unclassified exceptions.
 */
export function validateInterfaceToolRequest(
  request: unknown,
): InterfaceToolResult | null {
  if (!isPlainObject(request)) {
    return validationError("interface_create", "request", "Request must be a plain object.");
  }
  const tool = request.tool;
  if (typeof tool !== "string" || !INTERFACE_TOOL_NAMES.includes(tool as InterfaceToolName)) {
    return validationError("interface_create", "tool", `Unknown tool: ${JSON.stringify(tool)}.`);
  }
  const t = tool as InterfaceToolName;

  const size = JSON.stringify(request).length;
  if (size > MAX_PAYLOAD_BYTES) {
    return validationError(
      t,
      "payload_size",
      `Payload of ${size} bytes exceeds MAX_PAYLOAD_BYTES (${MAX_PAYLOAD_BYTES}).`,
    );
  }

  if (typeof request.workspace_id !== "string" || request.workspace_id.length === 0) {
    return validationError(t, "workspace_id", "workspace_id must be a non-empty string.");
  }

  if (t !== "interface_create") {
    if (typeof request.object_id !== "string" || request.object_id.length === 0) {
      return validationError(t, "object_id", "object_id must be a non-empty string.");
    }
  }

  if (t === "interface_create" || t === "interface_rename") {
    if (typeof request.name !== "string" || request.name.length === 0) {
      return validationError(t, "name", "name must be a non-empty string.");
    }
  }

  if (t === "interface_create" || t === "interface_revise") {
    if (!isPlainObject(request.spec)) {
      return validationError(t, "spec", "spec must be a plain object.");
    }
  }

  if (t === "interface_revise") {
    if (typeof request.expected_revision !== "string" || request.expected_revision.length === 0) {
      return validationError(
        t,
        "expected_revision",
        "expected_revision must be a non-empty string.",
      );
    }
  }

  return null; // valid shape
}

// ---------- Classification ----------

/**
 * Full deterministic classification for a request against a pure scope
 * snapshot. Order: validation, grant mismatch, tombstone/revoked, stale
 * revision, ok. No result path can recreate a deleted object: create carries
 * no object_id, and every object-referencing tool against a deleted object
 * returns tombstone, never ok.
 */
export function classifyInterfaceToolRequest(
  request: InterfaceToolRequest,
  snapshot: ScopeSnapshot,
): InterfaceToolResult {
  const invalid = validateInterfaceToolRequest(request);
  if (invalid) return invalid;

  const t = request.tool;

  if (!snapshot.granted_workspace_ids.includes(request.workspace_id)) {
    return {
      kind: "grant_mismatch",
      tool: t,
      field: "workspace_id",
      message: `Workspace ${request.workspace_id} is outside the granted scope.`,
    };
  }

  if (t !== "interface_create") {
    if (!snapshot.granted_object_ids.includes(request.object_id)) {
      return {
        kind: "grant_mismatch",
        tool: t,
        field: "object_id",
        message: `Object ${request.object_id} is outside the granted scope.`,
      };
    }
    if (snapshot.object_lifecycle === "deleted") {
      return {
        kind: "tombstone",
        tool: t,
        message: `Object ${request.object_id} is deleted; late events cannot recreate it.`,
      };
    }
    if (snapshot.object_lifecycle === "revoked") {
      return {
        kind: "revoked",
        tool: t,
        message: `Object ${request.object_id} is revoked.`,
      };
    }
  }

  if (t === "interface_revise") {
    if (snapshot.current_revision !== request.expected_revision) {
      return {
        kind: "stale_revision",
        tool: t,
        field: "expected_revision",
        preserved_revision: snapshot.current_revision,
        message:
          "Expected revision does not match the current revision. The last working revision is preserved; no partial application occurred.",
      };
    }
  }

  return { kind: "ok", tool: t, message: "Request accepted by the pure contract classifier." };
}

/**
 * Runtime-checked invariant helper: asserts that no result kind recreates a
 * deleted object. Any ok result for an object-referencing tool against a
 * tombstoned lifecycle is a contract violation.
 */
export function assertNoRecreation(
  results: readonly InterfaceToolResult[],
  targetLifecycle: ObjectLifecycle,
): boolean {
  if (targetLifecycle !== "deleted") return true;
  return results.every((r) => r.kind !== "ok");
}
