/**
 * Pure contract fixture tests for the Interface tool contract.
 *
 * Run with Node v22: node --test --experimental-strip-types tests/interface-tools.test.ts
 * (or node --test with type stripping enabled by default in newer v22.x).
 *
 * PURE CONTRACT FIXTURES ONLY: synthetic data, no network, no installs, no
 * filesystem outside the repo, no credentials or personal data. These tests
 * are not evidence of live Hermes, native, browser, backend, device, or
 * account acceptance; parent HI06 (hermes-interface-tools) remains OPEN.
 */

import { test, describe } from "node:test";
import assert from "node:assert/strict";

import {
  INTERFACE_TOOL_NAMES,
  MAX_PAYLOAD_BYTES,
  ACTION_CLASSIFICATIONS,
  classifyAction,
  validateInterfaceToolRequest,
  classifyInterfaceToolRequest,
  assertNoRecreation,
} from "../src/contracts/interface-tools.ts";

const baseSnapshot = {
  granted_workspace_ids: ["ws-1"],
  granted_object_ids: ["if-1"],
  object_lifecycle: "active" as const,
  current_revision: "rev-7",
};

const validRequests = {
  interface_create: { tool: "interface_create", workspace_id: "ws-1", name: "Inbox overview", spec: { kind: "list" } },
  interface_inspect: { tool: "interface_inspect", workspace_id: "ws-1", object_id: "if-1" },
  interface_rename: { tool: "interface_rename", workspace_id: "ws-1", object_id: "if-1", name: "Renamed surface" },
  interface_revise: { tool: "interface_revise", workspace_id: "ws-1", object_id: "if-1", expected_revision: "rev-7", spec: { kind: "table" } },
  interface_remove: { tool: "interface_remove", workspace_id: "ws-1", object_id: "if-1" },
};

describe("valid operations", () => {
  for (const tool of INTERFACE_TOOL_NAMES) {
    test(`${tool} classifies ok in scope`, () => {
      const result = classifyInterfaceToolRequest(validRequests[tool], baseSnapshot);
      assert.equal(result.kind, "ok");
      assert.equal(result.tool, tool);
    });
  }
});

describe("validation errors", () => {
  test("malformed shape (not an object)", () => {
    const r = validateInterfaceToolRequest("nonsense");
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "request");
  });
  test("unknown tool", () => {
    const r = validateInterfaceToolRequest({ tool: "interface_explode", workspace_id: "ws-1" });
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "tool");
  });
  test("missing workspace_id", () => {
    const r = validateInterfaceToolRequest({ tool: "interface_inspect", object_id: "if-1" });
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "workspace_id");
  });
  test("missing object_id on inspect", () => {
    const r = validateInterfaceToolRequest({ tool: "interface_inspect", workspace_id: "ws-1" });
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "object_id");
  });
  test("missing name on create", () => {
    const r = validateInterfaceToolRequest({ tool: "interface_create", workspace_id: "ws-1", spec: {} });
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "name");
  });
  test("missing expected_revision on revise", () => {
    const r = validateInterfaceToolRequest({ tool: "interface_revise", workspace_id: "ws-1", object_id: "if-1", spec: {} });
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "expected_revision");
  });
  test("oversized payload names the bound", () => {
    const big = { tool: "interface_create", workspace_id: "ws-1", name: "big", spec: { blob: "x".repeat(MAX_PAYLOAD_BYTES) } };
    const r = validateInterfaceToolRequest(big);
    assert.equal(r?.kind, "validation_error");
    assert.equal(r?.field, "payload_size");
    assert.ok(r?.message.includes(String(MAX_PAYLOAD_BYTES)));
  });
});

describe("grant mismatch", () => {
  test("workspace outside scope", () => {
    const r = classifyInterfaceToolRequest(validRequests.interface_inspect, {
      ...baseSnapshot,
      granted_workspace_ids: ["ws-other"],
    });
    assert.equal(r.kind, "grant_mismatch");
    assert.equal(r.field, "workspace_id");
  });
  test("object outside scope", () => {
    const r = classifyInterfaceToolRequest(validRequests.interface_inspect, {
      ...baseSnapshot,
      granted_object_ids: ["if-other"],
    });
    assert.equal(r.kind, "grant_mismatch");
    assert.equal(r.field, "object_id");
  });
});

describe("stale revision", () => {
  test("mismatch is a distinct conflict preserving the working revision", () => {
    const r = classifyInterfaceToolRequest(
      { ...validRequests.interface_revise, expected_revision: "rev-3" },
      baseSnapshot,
    );
    assert.equal(r.kind, "stale_revision");
    assert.equal(r.preserved_revision, "rev-7");
    assert.ok(r.message.includes("preserved"));
  });
});

describe("tombstone and revoked", () => {
  for (const tool of ["interface_inspect", "interface_rename", "interface_revise", "interface_remove"] as const) {
    test(`${tool} against deleted object is tombstone`, () => {
      const r = classifyInterfaceToolRequest(validRequests[tool], { ...baseSnapshot, object_lifecycle: "deleted" });
      assert.equal(r.kind, "tombstone");
    });
    test(`${tool} against revoked object is revoked`, () => {
      const r = classifyInterfaceToolRequest(validRequests[tool], { ...baseSnapshot, object_lifecycle: "revoked" });
      assert.equal(r.kind, "revoked");
    });
  }
  test("no result path recreates a deleted object (invariant sweep)", () => {
    const results = INTERFACE_TOOL_NAMES.map((tool) =>
      classifyInterfaceToolRequest(validRequests[tool], { ...baseSnapshot, object_lifecycle: "deleted" }),
    );
    assert.equal(assertNoRecreation(results, "deleted"), true);
    // Extra safety: no ok results at all against a tombstone.
    assert.ok(results.every((r) => r.kind !== "ok"));
  });
});

describe("action classification (ADR 0004 host approval contract)", () => {
  test("remove is consequential; others routine", () => {
    assert.equal(classifyAction("interface_remove"), "consequential");
    for (const tool of INTERFACE_TOOL_NAMES) {
      if (tool === "interface_remove") continue;
      assert.equal(classifyAction(tool), "routine");
    }
    assert.equal(ACTION_CLASSIFICATIONS.interface_remove, "consequential");
  });
});
